//! Global configuration for VSF converter
//!
//! Manages paths for working directory.
//!
// Copyright (c) 2025 Tommy Olsen
// Licensed under the MIT License.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub const VERSION: &str = "0.9.1-beta";

#[derive(Clone)]
pub struct Config {
    pub work_path: PathBuf,
}

impl Config {
    pub fn new(work_path: impl AsRef<Path>) -> Self {
        Self {
            work_path: work_path.as_ref().to_path_buf(),
        }
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
