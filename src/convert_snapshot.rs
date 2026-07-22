//! Snapshot converter main API
//!
//! Converts Vice VSF snapshots to self-restoring PRG files with LZSA compression.
//!
// Copyright (c) 2025-2026 Tommy Olsen
// Licensed under the MIT License.

use crate::config::Config;
use crate::parse_vsf::{ParseVSF, C64Snapshot};
use crate::parse_ar;
use crate::find_ram::FindRam;
use crate::patch_mem::PatchMem;
use crate::make_prg_asm::MakePRGAsm;

pub struct ConvertSnapshot {
    config: Config,
    extra_ram_blocks: Vec<(u16, u16)>,
    poweron_cleared: std::cell::Cell<u32>,
}

impl ConvertSnapshot {
    /// Create a new converter with the given configuration
    pub fn new(config: Config) -> Self {
        Self::with_extra_blocks(config, Vec::new())
    }

    /// Create a new converter with extra RAM blocks
    /// Each block is (address, count)
    pub fn with_extra_blocks(config: Config, extra_ram_blocks: Vec<(u16, u16)>) -> Self {
        Self { config, extra_ram_blocks, poweron_cleared: std::cell::Cell::new(0) }
    }

    /// Bytes zeroed by the power-on pattern pass during the last [`convert`] call
    /// (0 if the pass is disabled or nothing matched).
    pub fn poweron_cleared(&self) -> u32 {
        self.poweron_cleared.get()
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
        self.poweron_cleared.set(0);
        if crate::util::paths_refer_to_same_file(input_path, output_path) {
            return Err(format!("Refusing to overwrite the input file:\n{}\n\nPlease choose a different output filename.", input_path));
        }
        if std::path::Path::new(output_path).exists() {
            return Err(format!("Output file already exists:\n{}\n\nPlease choose a different filename or delete the existing file first.", output_path));
        }

        let progress = self.config.progress.clone();
        progress.step("Reading snapshot...")?;

        // Accept either a VICE VSF snapshot or a self-restoring freezer image
        // (Action Replay etc.). Freeze files are decoded by replaying their own
        // restore stub; both paths yield a C64Snapshot for the rest of the pipeline.
        let input_bytes = std::fs::read(input_path)
            .map_err(|e| format!("Failed to read input file: {}", e))?;

        // Decide VSF vs cartridge freeze per the configured input mode (auto-detect,
        // forced VSF, or forced freeze). FC3 is 2-file: its '-fc' companion is found
        // next to the input by the resolver.
        let (parser, snap) = match parse_ar::resolve_input(input_path, &input_bytes, self.config.input_mode)
            .map_err(|e| format!("Failed to decode freezer snapshot: {}", e))?
        {
            parse_ar::FreezeOutcome::Freeze(snap) => {
                (ParseVSF::for_external_snapshot(input_path, &self.config), snap)
            }
            parse_ar::FreezeOutcome::Vsf => {
                let parser = ParseVSF::import(input_path, &self.config)
                    .map_err(|e| parse_ar::vsf_hint(e, &input_bytes))?;
                let snap = parser.parse_import()
                    .map_err(|e| parse_ar::vsf_hint(e, &input_bytes))?;
                (parser, snap)
            }
        };

        // Preserve $F8-$FF before any patching (critical for LZSA decompressor)
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

        // Optionally clear RAM still holding the C64 power-on pattern so it
        // becomes usable free space (mirrors the manual "f 0000 ffff 00" step).
        self.poweron_cleared.set(if self.config.clear_poweron_ram {
            FindRam::clear_poweron_pattern(&mut ram)
        } else {
            0
        });

        progress.step("Patching memory...")?;

        let mut ram_finder = FindRam::with_extra_blocks(&ram, &self.extra_ram_blocks);
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
        progress.step("Compressing RAM...")?;
        parser.compress_block(&ram_path, &format!("{}.lzsa", ram_path), true)
            .map_err(|e| format!("Failed to compress RAM: {}", e))?;
        progress.step("Compressing color RAM...")?;
        parser.compress_lzsa(&color_path, &format!("{}.lzsa", color_path))
            .map_err(|e| format!("Failed to compress color RAM: {}", e))?;
        progress.step("Compressing zero page...")?;
        parser.compress_lzsa(&zp_path, &format!("{}.lzsa", zp_path))
            .map_err(|e| format!("Failed to compress zero page: {}", e))?;
        progress.step("Compressing VIC...")?;
        parser.compress_lzsa(&vic_path, &format!("{}.lzsa", vic_path))
            .map_err(|e| format!("Failed to compress VIC: {}", e))?;
        progress.step("Compressing SID...")?;
        parser.compress_lzsa(&sid_path, &format!("{}.lzsa", sid_path))
            .map_err(|e| format!("Failed to compress SID: {}", e))?;

        progress.step("Assembling PRG...")?;

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

        // Fail if the user cancelled while the output was being assembled. The caller
        // removes the file that was just written.
        progress.check()?;

        Ok(())
    }
}
