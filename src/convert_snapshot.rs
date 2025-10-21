//! Snapshot converter main API
//!
//! Converts Vice VSF snapshots to self-restoring PRG files with LZSA compression.
//!
// Copyright (c) 2025 Tommy Olsen
// Licensed under the MIT License.

use crate::config::Config;
use crate::parse_vsf::{ParseVSF, C64Snapshot};
use crate::find_ram::FindRam;
use crate::patch_mem::PatchMem;
use crate::make_prg_asm::MakePRGAsm;

pub struct ConvertSnapshot {
    config: Config,
}

impl ConvertSnapshot {
    /// Create a new converter with the given configuration
    pub fn new(config: Config) -> Self {
        Self { config }
    }

    /// Convert a VSF snapshot to a PRG file
    ///
    /// # Arguments
    /// * `input_path` - Path to the input VSF file
    /// * `output_path` - Path to the output PRG file
    ///
    /// # Returns
    /// * `Ok(())` on success
    /// * `Err(String)` with user-friendly error message on failure
    pub fn convert(&self, input_path: &str, output_path: &str) -> Result<(), String> {
        if std::path::Path::new(output_path).exists() {
            return Err(format!("Output file already exists:\n{}\n\nPlease choose a different filename or delete the existing file first.", output_path));
        }

        let parser = ParseVSF::import(input_path, &self.config)
            .map_err(|e| format!("Failed to read VSF file: {}", e))?;

        let snap = parser.parse_import()
            .map_err(|e| format!("Failed to parse VSF: {}", e))?;

        // Preserve $F8-$FF before any patching (critical for LZSA decompressor)
        let mut f8_ff_data = [0u8; 8];
        f8_ff_data.copy_from_slice(&snap.mem.ram[0xF8..=0xFF]);

        let mut ram_finder = FindRam::new(&snap.mem.ram);

        let mut ram = snap.mem.ram.clone();
        let patch_mem = PatchMem::new(&snap, &mut *ram, &mut ram_finder)
            .map_err(|e| format!("Memory patching failed: {}", e))?;

        let patched_snap = C64Snapshot {
            cpu: snap.cpu.clone(),
            mem: crate::parse_vsf::C64Mem {
                cpu_port_data: snap.mem.cpu_port_data,
                cpu_port_dir: snap.mem.cpu_port_dir,
                ram,
            },
            vic: snap.vic.clone(),
            cia1: snap.cia1.clone(),
            cia2: snap.cia2.clone(),
            sid: snap.sid.clone(),
        };

        let (ram_path, color_path, zp_path, vic_path, sid_path, cia1_path, cia2_path) =
            parser.extract_ram(&patched_snap)
                .map_err(|e| format!("Failed to extract components: {}", e))?;

        // CIA files are not compressed (only 20 bytes each)
        parser.compress_lzsa(&ram_path, &format!("{}.lzsa", ram_path))
            .map_err(|e| format!("Failed to compress RAM: {}", e))?;
        parser.compress_lzsa(&color_path, &format!("{}.lzsa", color_path))
            .map_err(|e| format!("Failed to compress color RAM: {}", e))?;
        parser.compress_lzsa(&zp_path, &format!("{}.lzsa", zp_path))
            .map_err(|e| format!("Failed to compress zero page: {}", e))?;
        parser.compress_lzsa(&vic_path, &format!("{}.lzsa", vic_path))
            .map_err(|e| format!("Failed to compress VIC: {}", e))?;
        parser.compress_lzsa(&sid_path, &format!("{}.lzsa", sid_path))
            .map_err(|e| format!("Failed to compress SID: {}", e))?;

        let prg_maker = MakePRGAsm::new(
            &format!("{}.lzsa", color_path),
            &format!("{}.lzsa", vic_path),
            &format!("{}.lzsa", sid_path),
            &cia1_path,
            &cia2_path,
            &format!("{}.lzsa", zp_path),
            &format!("{}.lzsa", ram_path),
            patch_mem.get_block9_addr(),
            f8_ff_data,
            &self.config,
        ).map_err(|e| format!("Failed to initialize PRG maker: {}", e))?;

        prg_maker.generate_prg(output_path)
            .map_err(|e| format!("Failed to generate PRG: {}", e))?;

        Ok(())
    }
}
