//! Magic Desk CRT snapshot converter
//!
//! Converts Vice VSF snapshots to Magic Desk CRT cartridge files.
//! Uses ROML-only layout with CBM80 boot signature.
//!
//! Architecture (no embedded files):
//! - Bank 0 ROML @ $8000: Boot code (CBM80) + payload start
//! - Banks 0-N ROML: Restore code + relocated decompressor + RAM.lzsa
//!
//! Architecture (with embedded PRG files, LOAD hook):
//! - Bank 0 ROML: directory bank
//!     $8000 boot code (CBM80)
//!     $8400 LOAD handler
//!     $9000 file metadata
//!     $9800 filenames
//! - Banks 1-N ROML: Restore code + relocated decompressor + RAM.lzsa
//! - Banks N+1..   : embedded PRG file data
//!
//! Magic Desk's $DE00 bit 7 toggles EXROM and is fully reversible, so (unlike the
//! original assumption in earlier versions) the cartridge can be banked in and out
//! at runtime. This makes a KERNAL LOAD hook possible, identical in behaviour to
//! the EasyFlash format. The small trampoline lives in C64 RAM; the handler,
//! metadata and filenames live in the cartridge directory bank.
//!
// Copyright (c) 2025-2026 Tommy Olsen
// Licensed under the MIT License.

use crate::config::CrtConfig;
use crate::crt_builder::{CRTBuilder, CartridgeType, BANK_SIZE_8K};
use crate::file_system_manager::FileSystemManager;
use crate::find_ram::FindRam;
use crate::make_magic_desk_boot_asm::MakeMagicDeskBootAsm;
use crate::make_magic_desk_crt_asm::MakeMagicDeskCRTAsm;
use crate::make_magic_desk_load_save::{
    MagicDeskLoadSaveHook, FILENAMES_ADDRESS, HANDLER_ADDRESS, METADATA_ADDRESS,
};
use crate::parse_vsf::{C64Mem, C64Snapshot, ParseVSF};
use crate::patch_mem::PatchMem;
use std::fs;

/// Maximum Magic Desk banks (6-bit bank register => 64 banks => 512 KB).
const MAX_BANKS: usize = 64;

pub struct ConvertSnapshotMagicDeskCRT {
    config: CrtConfig,
    extra_ram_blocks: Vec<(u16, u16)>,
    poweron_cleared: std::cell::Cell<u32>,
}

impl ConvertSnapshotMagicDeskCRT {
    pub fn new(config: CrtConfig) -> Self {
        Self::with_extra_blocks(config, Vec::new())
    }

    /// Create a new converter with extra RAM blocks
    /// Each block is (address, count)
    pub fn with_extra_blocks(config: CrtConfig, extra_ram_blocks: Vec<(u16, u16)>) -> Self {
        Self { config, extra_ram_blocks, poweron_cleared: std::cell::Cell::new(0) }
    }

    /// Bytes zeroed by the power-on pattern pass during the last `convert` call
    /// (0 if the pass is disabled or nothing matched).
    pub fn poweron_cleared(&self) -> u32 {
        self.poweron_cleared.get()
    }

    /// Convert a VSF snapshot to a Magic Desk CRT file
    pub fn convert(&self, input_path: &str, output_path: &str) -> Result<(), String> {
        self.poweron_cleared.set(0);
        if std::path::Path::new(output_path).exists() {
            return Err(format!(
                "Output file already exists:\n{}\n\nPlease choose a different filename.",
                output_path
            ));
        }

        // Parse the VSF file
        let parser = ParseVSF::import(input_path, &self.config.base_config)
            .map_err(|e| format!("Failed to read VSF file: {}", e))?;

        let snap = parser
            .parse_import()
            .map_err(|e| format!("Failed to parse VSF: {}", e))?;

        // Preserve $F8-$FF before any patching
        let mut f8_ff_data = [0u8; 8];
        f8_ff_data.copy_from_slice(&snap.mem.ram[0xF8..=0xFF]);

        // Discover embedded PRG files (if requested). Only enable the LOAD hook
        // when there is at least one file to serve.
        let want_files = self.config.include_dir.is_some() && self.config.patch_load_save;
        let prg_files = if let (true, Some(dir)) = (want_files, self.config.include_dir.as_ref()) {
            let fs_manager = FileSystemManager::with_filename_start(dir, FILENAMES_ADDRESS);
            fs_manager.read_prg_files()?
        } else {
            Vec::new()
        };
        let has_files = !prg_files.is_empty();

        // Zero out manually specified extra blocks before compression
        let mut ram = snap.mem.ram.clone();
        for &(address, count) in &self.extra_ram_blocks {
            let start = address as usize;
            let end = (start + count as usize).min(ram.len());
            for i in start..end {
                ram[i] = 0;
            }
        }

        // Optionally clear RAM still holding the C64 power-on pattern so it
        // becomes usable free space (mirrors the manual "f 0000 ffff 00" step).
        self.poweron_cleared.set(if self.config.base_config.clear_poweron_ram {
            FindRam::clear_poweron_pattern(&mut ram)
        } else {
            0
        });

        // Hook the LOAD/SAVE trampoline into RAM BEFORE FindRam/PatchMem so the
        // trampoline area is seen as "used" and never allocated over (matches the
        // EasyFlash converter).
        let mut load_save_hook = if has_files {
            // Determine trampoline address (same mechanism as EasyFlash)
            // Auto location: use $100 if SP >= 242, otherwise $334
            let trampoline_addr = if self.config.auto_location || self.config.trampoline_address.is_none() {
                if snap.cpu.sp >= 242 {
                    0x0100 // SP is high enough, safe to use $0100
                } else {
                    0x0334 // SP is low, use $0334 to avoid stack collision
                }
            } else {
                self.config.trampoline_address.unwrap_or(0x0100)
            };

            let mut hook = MagicDeskLoadSaveHook::new(true, Some(trampoline_addr));
            hook.hook_load_and_save(&mut ram[..])
                .map_err(|e| format!("Failed to hook LOAD/SAVE: {}", e))?;
            Some(hook)
        } else {
            None
        };

        // Initialize RAM finder AFTER trampoline is written.
        let mut ram_finder = FindRam::with_extra_blocks(&ram, &self.extra_ram_blocks);

        // Patch memory with restoration code (using PatchMem)
        let patch_mem = PatchMem::new(&snap, &mut *ram, &mut ram_finder)
            .map_err(|e| format!("Memory patching failed: {}", e))?;

        // Create patched snapshot
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

        // Extract and compress components
        let (ram_path, color_path, zp_path, vic_path, sid_path, cia1_path, cia2_path) = parser
            .extract_ram(&patched_snap)
            .map_err(|e| format!("Failed to extract components: {}", e))?;

        parser
            .compress_lzsa(&ram_path, &format!("{}.lzsa", ram_path))
            .map_err(|e| format!("Failed to compress RAM: {}", e))?;
        parser
            .compress_lzsa(&color_path, &format!("{}.lzsa", color_path))
            .map_err(|e| format!("Failed to compress color RAM: {}", e))?;
        parser
            .compress_lzsa(&zp_path, &format!("{}.lzsa", zp_path))
            .map_err(|e| format!("Failed to compress zero page: {}", e))?;
        parser
            .compress_lzsa(&vic_path, &format!("{}.lzsa", vic_path))
            .map_err(|e| format!("Failed to compress VIC: {}", e))?;
        parser
            .compress_lzsa(&sid_path, &format!("{}.lzsa", sid_path))
            .map_err(|e| format!("Failed to compress SID: {}", e))?;

        // Read compressed RAM size
        let ram_lzsa = fs::read(format!("{}.lzsa", ram_path))
            .map_err(|e| format!("Failed to read RAM LZSA: {}", e))?;
        let ram_lzsa_size = ram_lzsa.len();

        // When files are embedded, bank 0 is reserved for the directory and the
        // restore payload begins at bank 1. Otherwise it follows the boot code in
        // bank 0.
        let restore_start_bank = if has_files { 1 } else { 0 };

        // Generate boot code first to know its size (pass 1 with restoreCodeSize=0)
        let boot_asm_pass1 = MakeMagicDeskBootAsm::with_restore_start_bank(0, restore_start_bank);
        let boot_code_pass1 = boot_asm_pass1.generate_boot_code()?;
        let boot_code_size = boot_code_pass1.len();

        // Generate relocated decompressor (to get size)
        let crt_asm_temp = MakeMagicDeskCRTAsm::new(
            &format!("{}.lzsa", color_path),
            &format!("{}.lzsa", vic_path),
            &format!("{}.lzsa", sid_path),
            &cia1_path,
            &cia2_path,
            &format!("{}.lzsa", zp_path),
            patch_mem.get_block9_addr(),
            f8_ff_data,
            &self.config.base_config,
            0,
            ram_lzsa_size,
            0,
            boot_code_size,
            restore_start_bank,
        )?;

        let relocated_binary = crt_asm_temp.generate_relocated_decompressor()?;
        let relocated_size = relocated_binary.len();

        // Generate restore code (pass 1 to get size)
        let crt_asm_pass1 = MakeMagicDeskCRTAsm::new(
            &format!("{}.lzsa", color_path),
            &format!("{}.lzsa", vic_path),
            &format!("{}.lzsa", sid_path),
            &cia1_path,
            &cia2_path,
            &format!("{}.lzsa", zp_path),
            patch_mem.get_block9_addr(),
            f8_ff_data,
            &self.config.base_config,
            relocated_size,
            ram_lzsa_size,
            0, // First pass
            boot_code_size,
            restore_start_bank,
        )?;

        let restore_code_pass1 = crt_asm_pass1.generate_restore_code_binary()?;
        let restore_code_size = restore_code_pass1.len();

        // Generate restore code (pass 2 with actual size)
        let crt_asm_final = MakeMagicDeskCRTAsm::new(
            &format!("{}.lzsa", color_path),
            &format!("{}.lzsa", vic_path),
            &format!("{}.lzsa", sid_path),
            &cia1_path,
            &cia2_path,
            &format!("{}.lzsa", zp_path),
            patch_mem.get_block9_addr(),
            f8_ff_data,
            &self.config.base_config,
            relocated_size,
            ram_lzsa_size,
            restore_code_size,
            boot_code_size,
            restore_start_bank,
        )?;

        let final_restore_code = crt_asm_final.generate_restore_code_binary()?;
        let final_relocated = crt_asm_final.generate_relocated_decompressor()?;

        // Regenerate boot code with correct restore code size (for trampoline page count)
        let boot_asm_final =
            MakeMagicDeskBootAsm::with_restore_start_bank(final_restore_code.len(), restore_start_bank);
        let boot_code_binary = boot_asm_final.generate_boot_code()?;

        // Verify boot code size didn't change
        if boot_code_binary.len() != boot_code_size {
            return Err(format!(
                "Boot code size changed between passes: {} -> {}. This is a bug - please report it.",
                boot_code_size,
                boot_code_binary.len()
            ));
        }

        // Payload = restore code + relocated decompressor + RAM.lzsa
        let mut payload = Vec::new();
        payload.extend_from_slice(&final_restore_code);
        payload.extend_from_slice(&final_relocated);
        payload.extend_from_slice(&ram_lzsa);

        // Cartridge name
        let cartridge_name = self
            .config
            .cartridge_name
            .as_deref()
            .unwrap_or("VICE SNAPSHOT");

        if has_files {
            self.build_with_files(
                output_path,
                cartridge_name,
                &boot_code_binary,
                &payload,
                &prg_files,
                load_save_hook.as_mut().unwrap(),
            )
        } else {
            self.build_plain(output_path, cartridge_name, &boot_code_binary, &payload)
        }
    }

    /// Build a plain Magic Desk cartridge (no embedded files).
    /// Bank 0 holds the boot code followed by the payload; the payload spills into
    /// banks 1..N.
    fn build_plain(
        &self,
        output_path: &str,
        cartridge_name: &str,
        boot_code_binary: &[u8],
        payload: &[u8],
    ) -> Result<(), String> {
        let total_payload_size = payload.len();

        let bank0_payload_space = BANK_SIZE_8K - boot_code_binary.len();
        let required_banks = if total_payload_size <= bank0_payload_space {
            1
        } else {
            let remaining = total_payload_size - bank0_payload_space;
            1 + remaining.div_ceil(BANK_SIZE_8K)
        };

        if required_banks > MAX_BANKS {
            return Err(Self::too_large_error(required_banks));
        }

        let num_banks = required_banks.max(8);

        let mut crt = CRTBuilder::new(CartridgeType::MagicDesk, num_banks, cartridge_name)?;

        // Bank 0: boot code first, then payload
        crt.fill_bank(0, boot_code_binary, 0)?;

        let mut data_offset = 0;
        let bank0_chunk = bank0_payload_space.min(payload.len());
        crt.fill_bank(0, &payload[..bank0_chunk], boot_code_binary.len())?;
        data_offset += bank0_chunk;

        let mut bank_idx = 1;
        while data_offset < payload.len() && bank_idx < num_banks {
            let chunk_size = BANK_SIZE_8K.min(payload.len() - data_offset);
            crt.fill_bank(bank_idx, &payload[data_offset..data_offset + chunk_size], 0)?;
            data_offset += chunk_size;
            bank_idx += 1;
        }

        if data_offset < payload.len() {
            return Err(Self::write_error(payload.len(), data_offset));
        }

        crt.make_crt(output_path)
    }

    /// Build a Magic Desk cartridge with an embedded-file LOAD directory.
    /// Bank 0 is the directory; the restore payload occupies banks 1..N; file data
    /// occupies the banks after that.
    fn build_with_files(
        &self,
        output_path: &str,
        cartridge_name: &str,
        boot_code_binary: &[u8],
        payload: &[u8],
        prg_files: &[crate::file_system_manager::PRGFile],
        hook: &mut MagicDeskLoadSaveHook,
    ) -> Result<(), String> {
        // Restore payload occupies banks 1..=restore_payload_banks.
        let restore_payload_banks = payload.len().div_ceil(BANK_SIZE_8K).max(1);
        let first_file_bank = restore_payload_banks + 1;

        let include_dir = self.config.include_dir.as_ref().unwrap();
        let fs_manager = FileSystemManager::with_filename_start(include_dir, FILENAMES_ADDRESS);

        // Allocate file data into the banks after the restore payload.
        let available_banks: Vec<usize> = (first_file_bank..MAX_BANKS).collect();
        let allocations = fs_manager.allocate_files(prg_files, &available_banks)?;
        let metadata = fs_manager.generate_metadata(&allocations)?;
        let filenames = fs_manager.generate_filenames(&allocations)?;

        // Generate the LOAD handler that lives in the directory bank @ $8400.
        let handler_code = hook.generate_handler_rom_code()?;

        // The directory bank layout must not overlap: boot < $8400, handler <
        // $9000, metadata <= 2KB at $9000, filenames <= 2KB at $9800.
        let handler_offset = (HANDLER_ADDRESS - 0x8000) as usize;
        let metadata_offset = (METADATA_ADDRESS - 0x8000) as usize;
        let filenames_offset = (FILENAMES_ADDRESS - 0x8000) as usize;
        if boot_code_binary.len() > handler_offset {
            return Err(format!(
                "Boot code ({} bytes) overlaps the LOAD handler at $8400",
                boot_code_binary.len()
            ));
        }
        if handler_offset + handler_code.len() > metadata_offset {
            return Err(format!(
                "LOAD handler ({} bytes) overlaps the metadata table at $9000",
                handler_code.len()
            ));
        }

        // Highest used bank.
        let highest_file_bank = fs_manager
            .get_allocated_banks(&allocations)
            .into_iter()
            .max()
            .unwrap_or(restore_payload_banks);
        let banks_needed = highest_file_bank + 1;

        if banks_needed > MAX_BANKS {
            return Err(Self::too_large_error(banks_needed));
        }

        let num_banks = banks_needed.max(8);
        let mut crt = CRTBuilder::new(CartridgeType::MagicDesk, num_banks, cartridge_name)?;

        // Bank 0: directory (boot + handler + metadata + filenames)
        crt.fill_bank(0, boot_code_binary, 0)?;
        crt.fill_bank(0, &handler_code, handler_offset)?;
        crt.fill_bank(0, &metadata, metadata_offset)?;
        crt.fill_bank(0, &filenames, filenames_offset)?;

        // Banks 1..: restore payload
        let mut data_offset = 0;
        let mut bank_idx = 1;
        while data_offset < payload.len() {
            if bank_idx >= num_banks {
                return Err(Self::write_error(payload.len(), data_offset));
            }
            let chunk_size = BANK_SIZE_8K.min(payload.len() - data_offset);
            crt.fill_bank(bank_idx, &payload[data_offset..data_offset + chunk_size], 0)?;
            data_offset += chunk_size;
            bank_idx += 1;
        }

        // File data banks
        fs_manager.write_files_to_banks(&mut crt, &allocations)?;

        crt.make_crt(output_path)
    }

    fn too_large_error(required_banks: usize) -> String {
        format!(
            "Snapshot data is too large for Magic Desk cartridge!\n\n\
             Required banks: {}\nMaximum banks:  {} ({} bytes)\n\n\
             The snapshot is too large or doesn't compress well enough.",
            required_banks,
            MAX_BANKS,
            MAX_BANKS * BANK_SIZE_8K
        )
    }

    fn write_error(total: usize, written: usize) -> String {
        format!(
            "Failed to write all data to CRT banks!\n\n\
             Data size: {} bytes\nWritten:   {} bytes\nMissing:   {} bytes\n\n\
             This should not happen - please report this bug.",
            total,
            written,
            total - written
        )
    }
}
