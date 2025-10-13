//! External 6502 assembler runner that invokes vasm6502_std
//!
//! Minimal API for assembling 6502 code to raw binary or PRG format.
//!
//! Path resolution:
//! - VASM executable is resolved by, in order:
//!   1) config.util_path directory
//!   2) environment variable `VASM_UTIL_PATH`
//!   3) `PATH` environment variable
//!   4) current working directory
//!
//! - Work directory is resolved by, in order:
//!   1) config.work_path
//!   2) environment variable `VASM_WORK_PATH`
//!   3) OS temp directory
//!
//! Output is raw binary (`-Fbin`) by default. Use `assemble_prg()` for PRG format.
//!
//! This program is unlicensed and dedicated to the public domain.
//! Developed by Tommy Olsen.

#![allow(dead_code)]

use crate::config::Config;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[derive(Debug)]
pub enum AsmError {
    Asm(String),
    Io(std::io::Error),
}

impl From<std::io::Error> for AsmError {
    fn from(e: std::io::Error) -> Self {
        AsmError::Io(e)
    }
}

pub struct Assembler6502 {
    config: Config,
}

impl Assembler6502 {
    pub fn new(config: &Config) -> Self {
        Assembler6502 {
            config: config.clone(),
        }
    }

    /// Assemble VASM-syntax source into raw bytes
    pub fn assemble_bytes(&mut self, src: &str) -> Result<Vec<u8>, AsmError> {
        let exe = resolve_vasm_exe(&self.config)?;
        let work = resolve_work_dir(&self.config)?;

        fs::create_dir_all(&work).map_err(|e| {
            AsmError::Asm(format!("Failed to create work directory {:?}: {}", work, e))
        })?;

        // Create unique temp file names
        let stamp = unique_stamp();
        let asm_path = work.join(format!("temp_{}.asm", stamp));
        let out_path = work.join(format!("temp_{}.bin", stamp));

        // Write source
        File::create(&asm_path)
            .and_then(|mut f| f.write_all(src.as_bytes()))
            .map_err(|e| {
                AsmError::Asm(format!("Failed to write source file {:?}: {}", asm_path, e))
            })?;

        // Run VASM with hidden console window on Windows
        let mut command = Command::new(&exe);
        command
            .current_dir(&work)
            .arg("-Fbin")
            .arg("-quiet")
            .arg("-chklabels")
            .arg("-o")
            .arg(&out_path)
            .arg(&asm_path);

        #[cfg(windows)]
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW

        let output = command
            .output()
            .map_err(|e| AsmError::Asm(format!("Failed to execute VASM: {}", e)))?;

        // Handle non-zero exit
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr_trimmed = stderr.trim();
            let stdout_trimmed = stdout.trim();

            let mut msg = format!(
                "VASM compilation failed (exit code: {})\n",
                output.status.code().unwrap_or(-1)
            );

            if !stderr_trimmed.is_empty() {
                msg.push_str(&format!("\nErrors:\n{}", stderr_trimmed));
            } else if !stdout_trimmed.is_empty() {
                msg.push_str(&format!("\nOutput:\n{}", stdout_trimmed));
            } else {
                msg.push_str("\n(no output from assembler)\n");
            }

            let _ = fs::remove_file(&asm_path);
            let _ = fs::remove_file(&out_path);
            return Err(AsmError::Asm(msg));
        }

        // Ensure output exists
        if !out_path.exists() {
            let _ = fs::remove_file(&asm_path);
            return Err(AsmError::Asm(
                "VASM reported success but no output file was produced.".into()
            ));
        }

        // Read binary
        let mut bytes = Vec::new();
        File::open(&out_path)
            .and_then(|mut f| f.read_to_end(&mut bytes))
            .map_err(|e| {
                AsmError::Asm(format!("Failed to read output file {:?}: {}", out_path, e))
            })?;

        if bytes.is_empty() {
            let _ = fs::remove_file(&asm_path);
            let _ = fs::remove_file(&out_path);
            return Err(AsmError::Asm("VASM produced an empty output file.".into()));
        }

        // Cleanup
        let _ = fs::remove_file(&asm_path);
        let _ = fs::remove_file(&out_path);

        Ok(bytes)
    }

    /// Assemble VASM-syntax source into a C64 PRG file (with $0801 load address)
    pub fn assemble_prg(&mut self, src: &str) -> Result<Vec<u8>, AsmError> {
        let binary = self.assemble_bytes(src)?;

        // Prepend PRG header ($01 $08 - load address $0801)
        let mut prg = vec![0x01, 0x08];
        prg.extend_from_slice(&binary);

        Ok(prg)
    }
}

/* ======================= Helper functions ======================= */

#[cfg(windows)]
const EXE_NAME: &str = "vasm6502_std.exe";

#[cfg(not(windows))]
const EXE_NAME: &str = "vasm6502_std";

fn resolve_vasm_exe(config: &Config) -> Result<PathBuf, AsmError> {
    // 1) Config util_path
    let candidate = config.util_path.join(EXE_NAME);
    if candidate.exists() {
        return Ok(candidate);
    }

    // 2) VASM_UTIL_PATH environment variable
    if let Ok(p) = std::env::var("VASM_UTIL_PATH") {
        let candidate = Path::new(&p).join(EXE_NAME);
        if candidate.exists() {
            return Ok(candidate);
        }
    }

    // 3) PATH lookup
    if let Ok(path_env) = std::env::var("PATH") {
        let separator = if cfg!(windows) { ';' } else { ':' };
        for dir in path_env.split(separator) {
            let candidate = Path::new(dir).join(EXE_NAME);
            if candidate.exists() {
                return Ok(candidate);
            }
        }
    }

    // 4) Current working directory
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let candidate = cwd.join(EXE_NAME);
    if candidate.exists() {
        return Ok(candidate);
    }

    Err(AsmError::Asm(format!(
        "Could not locate {}. Set config.util_path, VASM_UTIL_PATH, or add it to PATH.",
        EXE_NAME
    )))
}

fn resolve_work_dir(config: &Config) -> Result<PathBuf, AsmError> {
    // 1) Config work_path
    if config.work_path.exists() || config.work_path.parent().is_some() {
        return Ok(config.work_path.clone());
    }

    // 2) VASM_WORK_PATH environment variable
    if let Ok(p) = std::env::var("VASM_WORK_PATH") {
        return Ok(PathBuf::from(p));
    }

    // 3) OS temp directory
    Ok(std::env::temp_dir())
}

fn unique_stamp() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let tid = std::thread::current().id();
    format!("{}_{:?}", now.as_millis(), tid)
}
