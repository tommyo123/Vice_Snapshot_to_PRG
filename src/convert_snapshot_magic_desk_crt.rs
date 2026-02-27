//! Magic Desk CRT snapshot converter
//!
//! Converts Vice VSF snapshots to Magic Desk CRT cartridge files.
//! Uses ROML-only layout with CBM80 boot signature.
//!
//! Architecture:
//! - Bank 0 ROML @ $8000: Boot code (CBM80) + payload start
//! - Banks 0-N ROML: Restore code + relocated decompressor + RAM.lzsa
//!
//! Note: Magic Desk has only a permanent kill bit ($DE00 bit 7). Unlike EasyFlash
//! ($DE02), there is no way to temporarily disable the cartridge. Once data is
//! copied to RAM, the cart must be killed permanently. LOAD/SAVE hooks are not
//! supported -- use EasyFlash format for that.
//!
// Copyright (c) 2025 Tommy Olsen
// Licensed under the MIT License.

use crate::config::CrtConfig;
use crate::crt_builder::{CRTBuilder, CartridgeType, BANK_SIZE_8K};
use crate::find_ram::FindRam;
use crate::make_magic_desk_boot_asm::MakeMagicDeskBootAsm;
use crate::make_magic_desk_crt_asm::MakeMagicDeskCRTAsm;
use crate::parse_vsf::{C64Mem, C64Snapshot, ParseVSF};
use crate::patch_mem::PatchMem;
use std::fs;

pub struct ConvertSnapshotMagicDeskCRT {
    config: CrtConfig,
    extra_ram_blocks: Vec<(u16, u16)>,
}

impl ConvertSnapshotMagicDeskCRT {
    pub fn new(config: CrtConfig) -> Self {
        Self::with_extra_blocks(config, Vec::new())
    }

    /// Create a new converter with extra RAM blocks
    /// Each block is (address, count)
    pub fn with_extra_blocks(config: CrtConfig, extra_ram_blocks: Vec<(u16, u16)>) -> Self {
        Self { config, extra_ram_blocks }
    }

    /// Convert a VSF snapshot to a Magic Desk CRT file
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

        // Zero out manually specified extra blocks before compression
        let mut ram = snap.mem.ram.clone();
        for &(address, count) in &self.extra_ram_blocks {
            let start = address as usize;
            let end = (start + count as usize).min(ram.len());
            for i in start..end {
                ram[i] = 0;
            }
        }

        // No LOAD/SAVE hooking for Magic Desk -- initialize RAM finder directly
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

        // Generate boot code first to know its size (pass 1 with restoreCodeSize=0)
        let boot_asm_pass1 = MakeMagicDeskBootAsm::new(0);
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
        )?;

        let final_restore_code = crt_asm_final.generate_restore_code_binary()?;
        let final_relocated = crt_asm_final.generate_relocated_decompressor()?;

        // Regenerate boot code with correct restore code size (for trampoline page count)
        let boot_asm_final = MakeMagicDeskBootAsm::new(final_restore_code.len());
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
        let total_payload_size = final_restore_code.len() + final_relocated.len() + ram_lzsa_size;

        // Calculate required banks
        let bank0_payload_space = BANK_SIZE_8K - boot_code_binary.len();
        let required_banks = if total_payload_size <= bank0_payload_space {
            1
        } else {
            let remaining = total_payload_size - bank0_payload_space;
            1 + (remaining + BANK_SIZE_8K - 1) / BANK_SIZE_8K
        };

        let max_banks = 64;
        if required_banks > max_banks {
            return Err(format!(
                "Snapshot data is too large for Magic Desk cartridge!\n\n\
                 Required banks: {}\nMaximum banks:  {} ({} bytes)\n\n\
                 The snapshot is too large or doesn't compress well enough.",
                required_banks,
                max_banks,
                max_banks * BANK_SIZE_8K
            ));
        }

        // Minimum 8 banks for Magic Desk compatibility
        let num_banks = required_banks.max(8);

        // Build the payload
        let mut payload = Vec::with_capacity(total_payload_size);
        payload.extend_from_slice(&final_restore_code);
        payload.extend_from_slice(&final_relocated);
        payload.extend_from_slice(&ram_lzsa);

        // Create CRT builder
        let cartridge_name = self
            .config
            .cartridge_name
            .as_deref()
            .unwrap_or("VICE SNAPSHOT");
        let mut crt = CRTBuilder::new(CartridgeType::MagicDesk, num_banks, cartridge_name)?;

        // Fill bank 0: boot code first, then payload
        crt.fill_bank(0, &boot_code_binary, 0)?;

        let mut data_offset = 0;
        let bank0_chunk = bank0_payload_space.min(payload.len());
        crt.fill_bank(0, &payload[..bank0_chunk], boot_code_binary.len())?;
        data_offset += bank0_chunk;

        // Remaining banks: payload from offset 0
        let mut bank_idx = 1;
        while data_offset < payload.len() && bank_idx < num_banks {
            let chunk_size = BANK_SIZE_8K.min(payload.len() - data_offset);
            crt.fill_bank(bank_idx, &payload[data_offset..data_offset + chunk_size], 0)?;
            data_offset += chunk_size;
            bank_idx += 1;
        }

        if data_offset < payload.len() {
            return Err(format!(
                "Failed to write all data to CRT banks!\n\n\
                 Data size: {} bytes\nWritten:   {} bytes\nMissing:   {} bytes\n\n\
                 This should not happen - please report this bug.",
                payload.len(),
                data_offset,
                payload.len() - data_offset
            ));
        }

        // Write CRT file
        crt.make_crt(output_path)?;

        Ok(())
    }
}
