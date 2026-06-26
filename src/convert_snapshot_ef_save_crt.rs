//! EasyFlash SAVE CRT snapshot converter.
//!
//! Produces an EasyFlash cartridge that restores a VSF snapshot AND embeds
//! drunella's libefs so the restored program's KERNAL SAVE/LOAD calls read and
//! write a persistent flash filesystem.
//!
//! Cartridge layout:
//!   bank 0 LOROM ($8000) : libefs library
//!   bank 0 HIROM         : read-only EFS dir | EAPI ($B800) | config ($BB00) | boot
//!   banks 1..N LOROM     : snapshot restore payload (restore code + decompressor + RAM.lzsa)
//!   banks 56-63          : rewritable save area (two 64 KB ping-pong sectors, $FF = erased)
//!
//! A small RAM trampoline (in the snapshot image) hooks the LOAD/SAVE vectors to
//! libefs; see make_ef_save_hook.
//
// Copyright (c) 2025-2026 Tommy Olsen
// Licensed under the MIT License.

use crate::config::{CrtConfig, EapiBuffer};
use crate::crt_builder::{CRTBuilder, CartridgeType, BANK_SIZE_8K};
use crate::ef_save::{self, EfsConfig};
use crate::make_efs_image::{build_efs_area, read_prg_dir, DIVISOR_8K, EFS_DIR_SIZE};
use crate::find_ram::FindRam;
use crate::make_crt_asm::MakeCRTAsm;
use crate::make_ef_save_boot_asm::MakeEfSaveBootAsm;
use crate::make_ef_save_hook::EfSaveHook;
use crate::parse_vsf::{C64Mem, C64Snapshot, ParseVSF};
use crate::patch_mem::PatchMem;
use std::fs;

/// Restore payload starts at bank 1 (bank 0 LOROM is libefs).
const RESTORE_START_BANK: usize = 1;
/// First bank of the rewritable area. Both rewritable areas live in HIROM
/// (chip 1): area 1 = banks 48-55, area 2 = banks 56-63. Keeping them off chip 0
/// (where libefs executes) means a defragment sector-erase never blanks the
/// running code's chip. See [`EfsConfig::with_top_rw_sector`].
const FIRST_RW_BANK: u8 = 48;
/// RAM the trampoline must stay below (cart shadows $8000-$BFFF).
const RAM_LIMIT: u16 = 0x8000;
/// ...and above (avoid zero page, stack, BASIC input buffer, cassette buffer and
/// especially the screen $0400-$07FF, which is a $20-filled "free-looking" run).
const RAM_FLOOR: u16 = 0x0900;
/// The EAPI flash buffer must be reachable in Ultimax mode, i.e. in $0000-$0FFF.
/// Auto placement searches $0900-$0FFF (above the default screen); the screen
/// fallback uses $0400-$0C00. The buffer is a page-aligned ~1 KB.
const EAPI_BUFFER_CEIL: u16 = 0x1000;
const EAPI_BUFFER_LEN: u16 = 768;
const SCREEN_STASH_LEN: u16 = 1024;

pub struct ConvertSnapshotEfSaveCRT {
    config: CrtConfig,
    extra_ram_blocks: Vec<(u16, u16)>,
}

impl ConvertSnapshotEfSaveCRT {
    pub fn new(config: CrtConfig) -> Self {
        Self::with_extra_blocks(config, Vec::new())
    }

    pub fn with_extra_blocks(config: CrtConfig, extra_ram_blocks: Vec<(u16, u16)>) -> Self {
        Self { config, extra_ram_blocks }
    }

    pub fn convert(&self, input_path: &str, output_path: &str) -> Result<(u16, Option<u16>, u16), String> {
        if std::path::Path::new(output_path).exists() {
            return Err(format!(
                "Output file already exists:\n{}\n\nPlease choose a different filename.",
                output_path
            ));
        }

        let parser = ParseVSF::import(input_path, &self.config.base_config)
            .map_err(|e| format!("Failed to read VSF file: {}", e))?;
        let snap = parser.parse_import().map_err(|e| format!("Failed to parse VSF: {}", e))?;

        let mut f8_ff_data = [0u8; 8];
        f8_ff_data.copy_from_slice(&snap.mem.ram[0xF8..=0xFF]);

        // Prepare RAM image: zero manual blocks + auto-clear power-on pattern.
        let mut ram = snap.mem.ram.clone();
        for &(address, count) in &self.extra_ram_blocks {
            let start = address as usize;
            let end = (start + count as usize).min(ram.len());
            for b in &mut ram[start..end] {
                *b = 0;
            }
        }
        FindRam::clear_poweron_pattern(&mut ram);

        // Reserve free RAM (below $8000) for the EAPI buffer (page-aligned 768 B)
        // and the LOAD/SAVE trampoline, then hook the vectors.
        let mut ram_finder = FindRam::with_extra_blocks(&ram, &self.extra_ram_blocks);

        let screen_addr = screen_address(&snap);

        // The EAPI flash write/erase code executes from this buffer in Ultimax
        // mode, so it must live in $0000-$0FFF (page-aligned, ~1 KB).
        let (eapi_page_hi, stash_addr) = self.resolve_eapi_buffer(&snap, &mut ram_finder, screen_addr)?;

        // The LOAD/SAVE trampoline goes at a caller-chosen address if given
        // (e.g. low stack), otherwise auto-placed in free RAM. Measure it first.
        let mut hook = EfSaveHook::new(0, eapi_page_hi).with_blank(self.config.force_blank);
        if let Some(stash) = stash_addr {
            hook = hook.with_stash(stash, screen_addr);
        }
        hook.generate_binary()?;
        let tramp_len = hook.reserved_len() as u16;
        let blob_addr = match self.config.trampoline_address {
            Some(addr) => {
                let end = addr as usize + tramp_len as usize;
                // Must stay clear of the screen ($0400-$07FF) and the cart window ($8000-$BFFF).
                let hits_screen = (addr as usize) < 0x0800 && end > 0x0400;
                let hits_cart = (addr as usize) < 0xC000 && end > 0x8000;
                if end > 0xD000 || hits_screen || hits_cart {
                    return Err(format!(
                        "The SAVE/LOAD trampoline ({} bytes) does not fit at ${:04X}; \
                         choose a location with that much free RAM below $0400, in $0800-$7FFF, or in $C000-$CFFF.",
                        tramp_len, addr
                    ));
                }
                ram_finder.reserve(addr, tramp_len);
                addr
            }
            None => {
                // Try to allocate in $C000-$CFFF first, then fall back to $0900-$7FFF
                if let Some((addr, _)) = ram_finder.allocate_in_range(tramp_len, 0xC000, 0xD000) {
                    addr
                } else {
                    ram_finder
                        .allocate_in_range(tramp_len, RAM_FLOOR, RAM_LIMIT)
                        .ok_or("Not enough free RAM for the SAVE/LOAD trampoline")?
                        .0
                }
            }
        };

        let mut hook = EfSaveHook::new(blob_addr, eapi_page_hi).with_blank(self.config.force_blank);
        if let Some(stash) = stash_addr {
            hook = hook.with_stash(stash, screen_addr);
        }
        hook.hook(&mut ram[..]).map_err(|e| format!("Failed to hook SAVE/LOAD: {}", e))?;

        // Patch restore code into RAM (uses the already-reduced free list).
        let patch_mem = PatchMem::new(&snap, &mut *ram, &mut ram_finder)
            .map_err(|e| format!("Memory patching failed: {}", e))?;

        let patched_snap = C64Snapshot {
            cpu: snap.cpu.clone(),
            mem: C64Mem {
                cpu_port_data: snap.mem.cpu_port_data,
                cpu_port_dir: snap.mem.cpu_port_dir,
                ram,
            },
            vic: snap.vic.clone(),
            cia1: snap.cia1.clone(),
            cia2: snap.cia2.clone(),
            sid: snap.sid.clone(),
        };

        // Extract + compress components.
        let (ram_path, color_path, zp_path, vic_path, sid_path, cia1_path, cia2_path) = parser
            .extract_ram(&patched_snap)
            .map_err(|e| format!("Failed to extract components: {}", e))?;
        for p in [&ram_path, &color_path, &zp_path, &vic_path, &sid_path] {
            parser
                .compress_lzsa(p, &format!("{}.lzsa", p))
                .map_err(|e| format!("Failed to compress {}: {}", p, e))?;
        }
        let ram_lzsa = fs::read(format!("{}.lzsa", ram_path))
            .map_err(|e| format!("Failed to read RAM LZSA: {}", e))?;
        let ram_lzsa_size = ram_lzsa.len();

        // Build restore code (bank-shifted to RESTORE_START_BANK).
        let mk = |relocated, restore| -> Result<MakeCRTAsm, String> {
            Ok(MakeCRTAsm::new(
                &format!("{}.lzsa", color_path),
                &format!("{}.lzsa", vic_path),
                &format!("{}.lzsa", sid_path),
                &cia1_path,
                &cia2_path,
                &format!("{}.lzsa", zp_path),
                patch_mem.get_block9_addr(),
                f8_ff_data,
                &self.config.base_config,
                relocated,
                ram_lzsa_size,
                restore,
                0, // load/save code is in RAM, not ROML
            )?
            .with_restore_start_bank(RESTORE_START_BANK))
        };

        let relocated_size = mk(0, 0)?.generate_relocated_decompressor()?.len();
        let restore_code_size = mk(relocated_size, 0)?.generate_restore_code_binary()?.len();
        let final_asm = mk(relocated_size, restore_code_size)?;
        let restore_code = final_asm.generate_restore_code_binary()?;
        let relocated = final_asm.generate_relocated_decompressor()?;

        if restore_code.len() > BANK_SIZE_8K {
            return Err(format!(
                "Restore code ({} bytes) exceeds one bank; not supported in the SAVE variant",
                restore_code.len()
            ));
        }

        // Payload = restore code + relocated decompressor + RAM.lzsa, in banks 1+.
        let mut payload = Vec::new();
        payload.extend_from_slice(&restore_code);
        payload.extend_from_slice(&relocated);
        payload.extend_from_slice(&ram_lzsa);
        let payload_banks = payload.len().div_ceil(BANK_SIZE_8K);
        // Payload + read-only files share LOROM (chip 0); the rewritable areas are
        // in HIROM (chip 1). Only the 64-bank LOROM space bounds the payload.
        if RESTORE_START_BANK + payload_banks > 64 {
            return Err(format!(
                "Snapshot payload ({} banks) does not fit in LOROM",
                payload_banks
            ));
        }

        // Read-only files (area 0) live in the banks right after the restore payload.
        let first_ro_bank = (RESTORE_START_BANK + payload_banks) as u8;
        let ro_files = match self.config.include_dir.as_deref() {
            Some(d) => read_prg_dir(d)?,
            None => Vec::new(),
        };
        let area0 = build_efs_area(&ro_files, first_ro_bank, 0, DIVISOR_8K)?;
        let ro_banks = area0.file_banks(0, DIVISOR_8K);
        // Read-only files live in LOROM (chip 0); the rewritable areas are in HIROM
        // (chip 1), so the only limit is the 64-bank LOROM space.
        if first_ro_bank as usize + ro_banks > 64 {
            return Err(format!(
                "Read-only files ({} banks) do not fit in LOROM (start bank {})",
                ro_banks, first_ro_bank
            ));
        }

        // Default files (area 1) seed the rewritable area; its directory occupies
        // the leading $1800 of the area, so files start at offset $1800.
        let rw_files = match self.config.rw_dir.as_deref() {
            Some(d) => read_prg_dir(d)?,
            None => Vec::new(),
        };
        let area1 = build_efs_area(&rw_files, FIRST_RW_BANK, EFS_DIR_SIZE, DIVISOR_8K)?;
        const RW_BANKS: usize = 8; // one area = 8 banks (64 KB)
        if EFS_DIR_SIZE + area1.files.len() > RW_BANKS * BANK_SIZE_8K {
            return Err("Default (rewritable) files do not fit in the save area".to_string());
        }

        // Config: read-only area 0 files placed at first_ro_bank (LOROM).
        let mut cfg = EfsConfig::with_top_rw_sector(FIRST_RW_BANK);
        cfg.area0.files_bank = first_ro_bank;
        cfg.area0.files_high = 0x80;
        cfg.area0.mode = ef_save::MODE_LLLL;

        let name = self.config.cartridge_name.as_deref().unwrap_or("VICE SNAPSHOT");
        let name_config = ef_save::generate_efs_name_and_config(name, &cfg);
        let efs_dir = if ro_files.is_empty() { None } else { Some(area0.dir.as_slice()) };
        let boot = MakeEfSaveBootAsm::new(restore_code.len(), RESTORE_START_BANK);
        let romh = boot.generate_romh(ef_save::eapi_code(), &name_config, efs_dir)?;

        // Assemble the cartridge.
        let mut crt = CRTBuilder::new(CartridgeType::EasyFlash, 64, name)?;
        let ff = [0xFFu8; BANK_SIZE_8K];
        for b in 0..64 {
            crt.clear_bank(b, 0xFF)?;
            crt.set_bank_romh(b, &ff)?;
        }
        crt.fill_bank(0, ef_save::lib_efs_code(), 0)?; // libefs
        crt.set_bank_romh(0, &romh)?; // boot + EAPI + config + area0 dir

        // Restore payload (banks 1.., LOROM)
        place_lorom_stream(&mut crt, RESTORE_START_BANK, 0, &payload)?;
        // Read-only files (banks first_ro_bank.., LOROM)
        place_lorom_stream(&mut crt, first_ro_bank as usize, 0, &area0.files)?;
        // Rewritable area 1 seed: directory ($0000) then files ($1800), in HIROM of
        // banks FIRST_RW_BANK.. (area 2 stays $FF/erased). Both areas are HIROM so a
        // defragment erase never blanks chip 0 (libefs).
        let mut area1_image = area1.dir.clone();
        area1_image.extend_from_slice(&area1.files);
        place_hirom_stream(&mut crt, FIRST_RW_BANK as usize, &area1_image)?;

        crt.make_crt(output_path)?;
        Ok((blob_addr, stash_addr, (eapi_page_hi as u16) << 8))
    }

    /// Decide where the EAPI flash buffer lives and return its page high byte
    /// and an optional screen stashing address.
    ///
    /// The EAPI's write/erase code runs from this buffer in Ultimax mode, so it
    /// must be page-aligned in `$0000-$0FFF`. Default ([`EapiBuffer::Auto`]) is
    /// free RAM in `$0900-$0FFF`; if there is none it falls back to the running
    /// program's screen RAM (clobbered during each LOAD/SAVE, then redrawn by the
    /// program). `Screen`/`Fixed` force those choices.
    fn resolve_eapi_buffer(
        &self,
        _snap: &C64Snapshot,
        ram_finder: &mut FindRam,
        screen_addr: u16,
    ) -> Result<(u8, Option<u16>), String> {
        let eapi_page_hi = match self.config.eapi_buffer {
            EapiBuffer::Fixed(addr) => place_eapi_buffer(addr, ram_finder, "The chosen EAPI buffer")?,
            EapiBuffer::Screen => {
                place_eapi_buffer(screen_addr + 0x0100, ram_finder, "The screen RAM")?
            }
            EapiBuffer::Auto => {
                if let Some((alloc, _)) =
                    ram_finder.allocate_aligned_in_range(EAPI_BUFFER_LEN, 256, RAM_FLOOR, EAPI_BUFFER_CEIL)
                {
                    if alloc % 256 != 0 || alloc + EAPI_BUFFER_LEN > EAPI_BUFFER_CEIL {
                        return Err(format!(
                            "Sanity check failed: aligned EAPI buffer allocation at ${:04X} is invalid",
                            alloc
                        ));
                    }
                    (alloc >> 8) as u8
                } else {
                    place_eapi_buffer(
                        screen_addr + 0x0100,
                        ram_finder,
                        "No free RAM in $0900-$0FFF, and the screen RAM",
                    )?
                }
            }
        };

        let eapi_addr = (eapi_page_hi as u16) << 8;
        let is_using_screen = eapi_addr >= screen_addr && eapi_addr < screen_addr + 1024;
        let stash_addr = if is_using_screen || self.config.force_stash {
            // Try to allocate a 1024-byte block in $C000-$CFFF first, then fall back to $0800-$7FFF.
            if let Some((alloc, _)) = ram_finder.allocate_in_range(SCREEN_STASH_LEN, 0xC000, 0xD000) {
                Some(alloc)
            } else if let Some((alloc, _)) = ram_finder.allocate_in_range(SCREEN_STASH_LEN, 0x0800, 0x8000) {
                Some(alloc)
            } else if self.config.force_stash {
                return Err("Failed to force screen stashing: no free 1 KB block in $C000-$CFFF or $0800-$7FFF found".to_string());
            } else {
                None
            }
        } else {
            None
        };

        Ok((eapi_page_hi, stash_addr))
    }
}

/// Address of the running program's screen matrix, from the snapshot's VIC bank
/// (CIA2 port A) and video-matrix bits (VIC `$D018`).
fn screen_address(snap: &C64Snapshot) -> u16 {
    let pra = snap.cia2.ora; // CIA2 port A drives the VIC 16K bank (inverted)
    let vic_bank = ((!pra) & 0x03) as u16 * 0x4000;
    let matrix = (((snap.vic.registers[0x18] >> 4) & 0x0F) as u16) * 0x0400;
    vic_bank + matrix
}

/// Validate that `addr` can hold the page-aligned EAPI buffer (must be reachable
/// in Ultimax mode and clear of the zero page/stack/vectors), then reserve it.
fn place_eapi_buffer(addr: u16, ram_finder: &mut FindRam, what: &str) -> Result<u8, String> {
    if addr & 0x00FF != 0
        || addr < 0x0400
        || addr as usize + EAPI_BUFFER_LEN as usize > EAPI_BUFFER_CEIL as usize
    {
        return Err(format!(
            "{} (${:04X}) can't hold the EAPI flash buffer: it must be a page-aligned \
             address in $0400-$0C00 (VIC bank 0), reachable in Ultimax mode.",
            what, addr
        ));
    }
    ram_finder.reserve(addr, EAPI_BUFFER_LEN);
    Ok((addr >> 8) as u8)
}

/// Write a byte stream into LOROM banks starting at (`bank`, `offset`), flowing
/// to the next bank's start when the 8 KB window fills.
fn place_lorom_stream(
    crt: &mut CRTBuilder,
    mut bank: usize,
    mut offset: usize,
    data: &[u8],
) -> Result<(), String> {
    let mut p = 0;
    while p < data.len() {
        let chunk = (BANK_SIZE_8K - offset).min(data.len() - p);
        crt.fill_bank(bank, &data[p..p + chunk], offset)?;
        p += chunk;
        bank += 1;
        offset = 0;
    }
    Ok(())
}

/// Write a byte stream into HIROM banks starting at `bank` offset 0. `CRTBuilder`
/// only sets whole 8 KB ROMH banks, so each bank is built as an `$FF`-padded 8 KB
/// image (matching erased flash) and written with `set_bank_romh`.
fn place_hirom_stream(crt: &mut CRTBuilder, bank: usize, data: &[u8]) -> Result<(), String> {
    for (i, chunk) in data.chunks(BANK_SIZE_8K).enumerate() {
        let mut page = [0xFFu8; BANK_SIZE_8K];
        page[..chunk.len()].copy_from_slice(chunk);
        crt.set_bank_romh(bank + i, &page)?;
    }
    Ok(())
}
