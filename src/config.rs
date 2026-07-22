//! Global configuration for VSF converter
//!
//! Manages paths for working directory and CRT-specific options.
//!
// Copyright (c) 2025-2026 Tommy Olsen
// Licensed under the MIT License.

use crate::progress::Progress;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub const VERSION: &str = "2.3";

/// How the input file should be interpreted by the converters.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum InputMode {
    /// Detect a freezer signature; fall back to a VICE VSF snapshot.
    /// This is the default.
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

/// Compression format for the restored snapshot blocks. Each maps to a lzan encoder and an
/// embedded 6502 decruncher. LZSA1 is the default.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PackFormat {
    Lzsa1,
    Lzsa2,
    Zx0,
    Zx02,
    LzanMin,
    Bolt,
    Bb2,
}

impl Default for PackFormat {
    fn default() -> Self {
        PackFormat::Lzsa1
    }
}

impl PackFormat {
    /// CLI identifier.
    pub fn as_str(self) -> &'static str {
        match self {
            PackFormat::Lzsa1 => "lzsa1",
            PackFormat::Lzsa2 => "lzsa2",
            PackFormat::Zx0 => "zx0",
            PackFormat::Zx02 => "zx02",
            PackFormat::LzanMin => "lzan-min",
            PackFormat::Bolt => "bolt",
            PackFormat::Bb2 => "bb2",
        }
    }

    /// Human-readable label for the GUI selector.
    pub fn label(self) -> &'static str {
        match self {
            PackFormat::Lzsa1 => "LZSA1 (default)",
            PackFormat::Lzsa2 => "LZSA2 (very slow compression)",
            PackFormat::Zx0 => "ZX0 (very slow compression)",
            PackFormat::Zx02 => "ZX02 (very slow compression)",
            PackFormat::LzanMin => "LZAN-min (very slow compression)",
            PackFormat::Bolt => "BoltLZ (fastest decompression)",
            PackFormat::Bb2 => "ByteBoozer2",
        }
    }

    /// Parse a CLI identifier (accepts a few aliases).
    pub fn from_str(s: &str) -> Option<Self> {
        Some(match s.to_ascii_lowercase().as_str() {
            "lzsa1" | "lzsa" => PackFormat::Lzsa1,
            "lzsa2" => PackFormat::Lzsa2,
            "zx0" => PackFormat::Zx0,
            "zx02" => PackFormat::Zx02,
            "lzan-min" | "lzanmin" | "lzan" => PackFormat::LzanMin,
            "bolt" | "boltlz" => PackFormat::Bolt,
            "bb2" | "byteboozer2" => PackFormat::Bb2,
            _ => return None,
        })
    }

    /// All formats, in selector order (LZSA1 first).
    pub fn all() -> [PackFormat; 7] {
        [
            PackFormat::Lzsa1,
            PackFormat::Lzsa2,
            PackFormat::Zx0,
            PackFormat::Zx02,
            PackFormat::LzanMin,
            PackFormat::Bolt,
            PackFormat::Bb2,
        ]
    }
}

/// Removes a conversion's temporary work directory when it goes out of scope, so the intermediate
/// files are gone whether the conversion succeeded, failed, was cancelled or panicked.
pub struct WorkDirGuard(PathBuf);

impl WorkDirGuard {
    pub fn new(work_path: impl Into<PathBuf>) -> Self {
        Self(work_path.into())
    }

    /// The directory this guard will remove.
    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for WorkDirGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Common tail for a conversion, run after its [`WorkDirGuard`] has removed the work directory.
///
/// A cancelled run deletes the output file it had begun writing. The converters refuse to
/// overwrite an existing output, and the caller removes any file the user agreed to replace, so
/// the file at that path came from this run and is truncated.
///
/// A successful run whose work directory is still on disk is reported as an error.
pub fn finish_conversion(
    outcome: Result<u32, String>,
    work_path: &Path,
    output_path: &str,
) -> Result<u32, String> {
    if outcome
        .as_ref()
        .err()
        .is_some_and(|e| crate::progress::is_cancelled_error(e))
    {
        let _ = std::fs::remove_file(output_path);
    }
    match outcome {
        Ok(_) if work_path.exists() => Err(format!(
            "Conversion succeeded, but the temporary directory could not be removed:\n{}",
            work_path.display()
        )),
        other => other,
    }
}

#[derive(Clone)]
pub struct Config {
    pub work_path: PathBuf,
    /// How to interpret the input file (VSF vs cartridge freeze).
    pub input_mode: InputMode,
    /// Zero RAM regions holding the C64 power-on pattern before the
    /// free-block scan (see `FindRam::clear_poweron_pattern`). Highly
    /// experimental; default off.
    pub clear_poweron_ram: bool,
    /// Compression format for the snapshot blocks. Default LZSA1.
    pub pack_format: PackFormat,
    /// Cancellation flag and current-step text, shared with whoever started the conversion.
    /// Default is a handle that is never cancelled, so non-interactive callers ignore it.
    pub progress: Progress,
}

impl Config {
    pub fn new(work_path: impl AsRef<Path>) -> Self {
        Self {
            work_path: work_path.as_ref().to_path_buf(),
            input_mode: InputMode::Auto,
            clear_poweron_ram: false,
            pack_format: PackFormat::default(),
            progress: Progress::default(),
        }
    }

    /// Set the compression format (builder style).
    pub fn with_pack_format(mut self, format: PackFormat) -> Self {
        self.pack_format = format;
        self
    }

    /// Share a progress/cancel handle with the caller (builder style).
    pub fn with_progress(mut self, progress: Progress) -> Self {
        self.progress = progress;
        self
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
