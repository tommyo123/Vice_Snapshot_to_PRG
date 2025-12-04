//! EasyFlash CRT snapshot converter
//!
//! Converts Vice VSF snapshots to EasyFlash CRT cartridge files with optional
//! LOAD/SAVE hooking for embedded PRG files.
//!
// Copyright (c) 2025 Tommy Olsen
// Licensed under the MIT License.

use crate::config::CrtConfig;
use crate::crt_builder::{CRTBuilder, CartridgeType, BANK_SIZE_8K};
use crate::file_system_manager::FileSystemManager;
use crate::find_ram::FindRam;
use crate::load_save_hook::LoadSaveHook;
use crate::make_crt_asm::MakeCRTAsm;
use crate::make_romh_asm::MakeROMHAsm;
use crate::parse_vsf::{C64Mem, C64Snapshot, ParseVSF};
use crate::patch_mem::PatchMem;
use std::fs;

pub struct ConvertSnapshotCRT {
    config: CrtConfig,
    extra_ram_blocks: Vec<(u16, u16)>,
}

impl ConvertSnapshotCRT {
    pub fn new(config: CrtConfig) -> Self {
        Self::with_extra_blocks(config, Vec::new())
    }

    /// Create a new converter with extra RAM blocks
    /// Each block is (address, count)
    pub fn with_extra_blocks(config: CrtConfig, extra_ram_blocks: Vec<(u16, u16)>) -> Self {
        Self { config, extra_ram_blocks }
    }

    /// Convert a VSF snapshot to an EasyFlash CRT file
    pub fn convert(&self, input_path: &str, output_path: &str) -> Result<(), String> {
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

        // Check if we have files to include
        let has_files = self.config.include_dir.is_some() && self.config.patch_load_save;

        // Zero out manually specified extra blocks before compression
        let mut ram = snap.mem.ram.clone();
        for &(address, count) in &self.extra_ram_blocks {
            let start = address as usize;
            let end = (start + count as usize).min(ram.len());
            for i in start..end {
                ram[i] = 0;
            }
        }

        // Hook LOAD/SAVE trampoline BEFORE PatchMem to prevent allocation conflicts
        let mut load_save_hook = if has_files {
            // Determine trampoline address
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

            let mut hook = LoadSaveHook::new(
                snap.cpu.sp,
                true,
                Some(trampoline_addr),
            );

            // Patch trampoline code and vectors into RAM BEFORE PatchMem!
            hook.hook_load_and_save(&mut ram[..])
                .map_err(|e| format!("Failed to hook LOAD/SAVE: {}", e))?;

            Some(hook)
        } else {
            None
        };

        // Initialize RAM finder AFTER trampoline is written
        // This ensures FindRam sees the trampoline area as "used" (non-zero bytes)
        // and won't allocate restore code blocks over it
        let mut ram_finder = FindRam::with_extra_blocks(&ram, &self.extra_ram_blocks);

        // Patch memory with restoration code (using PatchMem)
        // This runs AFTER trampoline is written (if include-dir is set)
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

        // Read compressed sizes
        let ram_lzsa = fs::read(format!("{}.lzsa", ram_path))
            .map_err(|e| format!("Failed to read RAM LZSA: {}", e))?;
        let ram_lzsa_size = ram_lzsa.len();

        // Generate relocated decompressor first (to get size)
        let crt_asm_temp = MakeCRTAsm::new(
            &format!("{}.lzsa", color_path),
            &format!("{}.lzsa", vic_path),
            &format!("{}.lzsa", sid_path),
            &cia1_path,
            &cia2_path,
            &format!("{}.lzsa", zp_path),
            patch_mem.get_block9_addr(),
            f8_ff_data,
            &self.config.base_config,
            0, // Will be set after first pass
            ram_lzsa_size,
            0, // Will be set after first pass
            0, // Will be set after first pass
        )?;

        let relocated_binary = crt_asm_temp.generate_relocated_decompressor()?;
        let relocated_size = relocated_binary.len();

        // Generate LOAD/SAVE ROM code if we have files
        let load_save_code = if let Some(ref mut hook) = load_save_hook {
            Some(hook.generate_load_save_rom_code()?)
        } else {
            None
        };
        // Note: load_save_code_size is NOT used for ROML layout - LOAD/SAVE code is only in ROMH
        let _load_save_code_size = load_save_code.as_ref().map(|c| c.len()).unwrap_or(0);

        // Generate restore code (first pass to get size)
        // NOTE: load_save_code_size is 0 because LOAD/SAVE code is NOT in ROML
        // It's only in ROMH @ $A600, matching Kotlin implementation
        let crt_asm = MakeCRTAsm::new(
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
            0, // LOAD/SAVE code is NOT in ROML
        )?;

        let restore_code = crt_asm.generate_restore_code_binary()?;
        let restore_code_size = restore_code.len();

        // Final pass with correct sizes
        let crt_asm_final = MakeCRTAsm::new(
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
            0, // LOAD/SAVE code is NOT in ROML
        )?;

        let final_restore_code = crt_asm_final.generate_restore_code_binary()?;
        let final_relocated = crt_asm_final.generate_relocated_decompressor()?;

        // Calculate how many banks we need for restore data
        // NOTE: LOAD/SAVE code is NOT in ROML - it's only in ROMH @ $A600
        // This matches the Kotlin implementation
        let total_restore_data_size =
            final_restore_code.len() + final_relocated.len() + ram_lzsa_size;
        let restore_banks_needed = (total_restore_data_size + BANK_SIZE_8K - 1) / BANK_SIZE_8K;

        // Process files if include directory is set
        let (file_allocations, metadata, filenames) = if let Some(ref include_dir) = self.config.include_dir {
            let fs_manager = FileSystemManager::new(include_dir);
            let prg_files = fs_manager.read_prg_files()?;

            if !prg_files.is_empty() {
                // Calculate available banks (after restore data)
                let available_banks: Vec<usize> = (restore_banks_needed..64).collect();
                let allocations = fs_manager.allocate_files(&prg_files, &available_banks)?;
                let meta = fs_manager.generate_metadata(&allocations)?;
                let names = fs_manager.generate_filenames(&allocations)?;
                (Some(allocations), Some(meta), Some(names))
            } else {
                (None, None, None)
            }
        } else {
            (None, None, None)
        };

        // Determine total banks needed
        let file_banks = file_allocations
            .as_ref()
            .map(|a| {
                let fs_manager = FileSystemManager::new(self.config.include_dir.as_ref().unwrap());
                fs_manager.get_allocated_banks(a).into_iter().max().map(|m| m + 1).unwrap_or(0)
            })
            .unwrap_or(0);
        let total_banks = restore_banks_needed.max(file_banks).max(1);

        // Create CRT builder
        let cartridge_name = self
            .config
            .cartridge_name
            .as_deref()
            .unwrap_or("VICE Snapshot");
        let mut crt = CRTBuilder::new(CartridgeType::EasyFlash, total_banks, cartridge_name)?;

        // Fill bank 0 with restore code
        // ROML layout: [restore code] [relocated decompressor] [RAM.lzsa]
        // NOTE: LOAD/SAVE code is NOT in ROML - it's only in ROMH @ $A600
        let mut offset = 0;
        crt.fill_bank(0, &final_restore_code, offset)?;
        offset += final_restore_code.len();

        // Add relocated decompressor (no LOAD/SAVE code in ROML!)
        if offset + final_relocated.len() <= BANK_SIZE_8K {
            crt.fill_bank(0, &final_relocated, offset)?;
            offset += final_relocated.len();
        }

        // Add RAM LZSA (may span multiple banks)
        let mut ram_offset = 0;
        let mut current_bank = 0;
        while ram_offset < ram_lzsa.len() {
            let space_in_bank = BANK_SIZE_8K - offset;
            let chunk_size = space_in_bank.min(ram_lzsa.len() - ram_offset);
            crt.fill_bank(current_bank, &ram_lzsa[ram_offset..ram_offset + chunk_size], offset)?;
            ram_offset += chunk_size;
            offset = 0;
            current_bank += 1;
            if current_bank >= total_banks && ram_offset < ram_lzsa.len() {
                crt.add_bank();
            }
        }

        // Generate ROMH
        // NOTE: LOAD/SAVE trampoline is NOT passed here - it's written to RAM at $0334
        // and gets decompressed back when RAM.lzsa is decompressed
        let romh_generator = MakeROMHAsm::new(
            final_restore_code.len(),
            load_save_code.clone(),
            metadata.clone(),
            filenames.clone(),
        );
        let romh_data = romh_generator.generate_romh()?;
        crt.set_bank_romh(0, &romh_data)?;

        // Write files to banks if we have allocations
        if let Some(ref allocations) = file_allocations {
            let fs_manager = FileSystemManager::new(self.config.include_dir.as_ref().unwrap());
            fs_manager.write_files_to_banks(&mut crt, allocations)?;
        }

        // Write CRT file
        crt.make_crt(output_path)?;

        Ok(())
    }
}
