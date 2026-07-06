//! Global configuration for VSF converter
//!
//! Manages paths for working directory and CRT-specific options.
//!
// Copyright (c) 2025-2026 Tommy Olsen
// Licensed under the MIT License.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub const VERSION: &str = "2.2";

/// How the input file should be interpreted by the converters.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum InputMode {
    /// Detect a freezer signature; fall back to a VICE VSF snapshot.
    /// This is the default and preserves the original auto-detect behaviour (CLI).
    Auto,
    /// Force VICE VSF snapshot parsing (do not treat the file as a freeze).
    Vsf,
    /// Convert a cartridge freeze, optionally forcing the freezer family.
    Freeze(FreezeMethod),
}

/// Which freezer family to use when converting a cartridge freeze.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FreezeMethod {
    /// Auto-detect which freezer produced the file.
    Auto,
    /// Self-restoring replay engine: Action Replay MK3-V8.4 + clones,
    /// Super Snapshot 5, Freeze Machine, Expert Cartridge.
    SelfRestoring,
    /// ISEPIC 2-file freeze (feed the `-name` data file).
    Isepic,
    /// Final Cartridge III 2-file freeze (feed the `fc` stub; `-fc` is auto-found).
    Fc3,
}

impl Default for InputMode {
    fn default() -> Self {
        InputMode::Auto
    }
}

#[derive(Clone)]
pub struct Config {
    pub work_path: PathBuf,
    /// How to interpret the input file (VSF vs cartridge freeze).
    pub input_mode: InputMode,
    /// Zero RAM regions still holding the C64 power-on pattern before the
    /// free-block scan (see `FindRam::clear_poweron_pattern`). Highly
    /// experimental; default off.
    pub clear_poweron_ram: bool,
}

impl Config {
    pub fn new(work_path: impl AsRef<Path>) -> Self {
        Self {
            work_path: work_path.as_ref().to_path_buf(),
            input_mode: InputMode::Auto,
            clear_poweron_ram: false,
        }
    }

    /// Set how the input file is interpreted (builder style).
    pub fn with_input_mode(mut self, mode: InputMode) -> Self {
        self.input_mode = mode;
        self
    }

    /// Enable/disable the power-on RAM pattern clearing pass.
    pub fn with_clear_poweron(mut self, enabled: bool) -> Self {
        self.clear_poweron_ram = enabled;
        self
    }

    pub fn work_str(&self) -> &str {
        self.work_path.to_str().expect("Invalid work path")
    }

    /// Create a Config with a unique temporary work directory
    pub fn auto() -> Result<Self, Box<dyn std::error::Error>> {
        let work_path = Self::create_temp_work_dir()?;
        Ok(Self::new(work_path))
    }

    /// Create a unique temporary work directory
    fn create_temp_work_dir() -> Result<PathBuf, Box<dyn std::error::Error>> {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| format!("Failed to get system time: {}", e))?
            .as_millis();

        let temp_base = std::env::temp_dir();
        let work_dir = temp_base.join(format!("ViceSnapshotConvert.{}", timestamp));

        std::fs::create_dir_all(&work_dir)
            .map_err(|e| format!("Failed to create work directory {:?}: {}", work_dir, e))?;

        Ok(work_dir)
    }
}

impl Default for Config {
    fn default() -> Self {
        Self::auto().unwrap_or_else(|_| {
            // Fallback to current directory if auto fails
            Self::new(std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
        })
    }
}

/// Configuration for CRT (EasyFlash / Magic Desk cartridge) conversion
#[derive(Clone)]
pub struct CrtConfig {
    /// Base configuration (work directory)
    pub base_config: Config,
    /// Optional directory containing PRG files to embed
    pub include_dir: Option<String>,
    /// Custom trampoline address for LOAD/SAVE hooks
    pub trampoline_address: Option<u16>,
    /// Auto-detect trampoline location based on stack pointer
    pub auto_location: bool,
    /// Cartridge name (max 32 characters)
    pub cartridge_name: Option<String>,
    /// Enable LOAD/SAVE hooking
    pub patch_load_save: bool,
}

impl CrtConfig {
    /// Create a new CRT configuration
    pub fn new(base_config: Config) -> Self {
        Self {
            base_config,
            include_dir: None,
            trampoline_address: None,
            auto_location: true,
            cartridge_name: None,
            patch_load_save: false,
        }
    }

    /// Create with auto-generated work directory
    pub fn auto() -> Result<Self, Box<dyn std::error::Error>> {
        let base = Config::auto()?;
        Ok(Self::new(base))
    }

    /// Set the include directory for PRG files
    pub fn with_include_dir(mut self, dir: &str) -> Self {
        self.include_dir = Some(dir.to_string());
        self.patch_load_save = true;
        self
    }

    /// Set custom trampoline address
    pub fn with_trampoline_address(mut self, addr: u16) -> Self {
        self.trampoline_address = Some(addr);
        self.auto_location = false;
        self
    }

    /// Set cartridge name
    pub fn with_cartridge_name(mut self, name: &str) -> Self {
        self.cartridge_name = Some(name.to_string());
        self
    }

    /// Enable/disable LOAD/SAVE patching
    pub fn with_patch_load_save(mut self, enabled: bool) -> Self {
        self.patch_load_save = enabled;
        self
    }
}

impl Default for CrtConfig {
    fn default() -> Self {
        Self::auto().unwrap_or_else(|_| Self::new(Config::default()))
    }
}
