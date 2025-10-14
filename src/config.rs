//! Global configuration for VSF converter
//!
//! Manages paths for working directory and utilities.
//!
//! This program is unlicensed and dedicated to the public domain.
//! Developed by Tommy Olsen.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Application version
pub const VERSION: &str = "0.9-beta";

#[derive(Clone)]
pub struct Config {
    pub work_path: PathBuf,
    pub util_path: PathBuf,
}

impl Config {
    pub fn new(work_path: impl AsRef<Path>, util_path: impl AsRef<Path>) -> Self {
        Self {
            work_path: work_path.as_ref().to_path_buf(),
            util_path: util_path.as_ref().to_path_buf(),
        }
    }

    pub fn work_str(&self) -> &str {
        self.work_path.to_str().expect("Invalid work path")
    }

    pub fn util_str(&self) -> &str {
        self.util_path.to_str().expect("Invalid util path")
    }

    /// Create a Config with automatically determined paths
    ///
    /// - work_path: Creates a unique temp directory in the system temp folder
    /// - util_path: Uses the "util" directory next to the executable
    pub fn auto() -> Result<Self, Box<dyn std::error::Error>> {
        let work_path = Self::create_temp_work_dir()?;
        let util_path = Self::get_util_path()?;

        Ok(Self::new(work_path, util_path))
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

    /// Get the util directory path (next to executable)
    fn get_util_path() -> Result<PathBuf, Box<dyn std::error::Error>> {
        let exe_path = std::env::current_exe()
            .map_err(|e| format!("Failed to get executable path: {}", e))?;

        let exe_dir = exe_path.parent()
            .ok_or("Failed to get executable directory")?;

        Ok(exe_dir.join("util"))
    }
}

impl Default for Config {
    fn default() -> Self {
        Self::auto().unwrap_or_else(|_| {
            // Fallback to current directory if auto fails
            Self::new(
                std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
                PathBuf::from("util")
            )
        })
    }
}
