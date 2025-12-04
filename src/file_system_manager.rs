//! File system manager for CRT banks
//!
//! Reads PRG files from a directory and allocates them to unused banks.
//! Generates metadata for file directory at $B000-$B7FF and filenames at $B800+
//!
// Copyright (c) 2025 Tommy Olsen
// Licensed under the MIT License.

use std::fs;
use std::path::Path;
use crate::crt_builder::{CRTBuilder, BANK_SIZE_8K};

pub const METADATA_START: u16 = 0xB000;
pub const METADATA_END: u16 = 0xB7FF;
pub const FILENAME_START: u16 = 0xB800;
pub const FILENAME_END: u16 = 0xBFFF;
pub const MAX_BANKS_PER_FILE: usize = 8;
pub const MAX_FILE_SIZE: usize = 64 * 1024; // 64KB
pub const METADATA_ENTRY_SIZE: usize = 16;

/// Represents a PRG file with its metadata
#[derive(Debug, Clone)]
pub struct PRGFile {
    pub filename: String,
    pub load_address: u16,
    pub data: Vec<u8>,
    pub total_size: usize,
}

/// Represents file allocation in banks
#[derive(Debug, Clone)]
pub struct FileAllocation {
    pub file: PRGFile,
    pub banks: Vec<usize>,
    pub start_offset: usize,
    pub filename_offset: usize,
}

/// Manages file system in CRT cartridge
pub struct FileSystemManager {
    include_dir: String,
}

impl FileSystemManager {
    /// Create a new file system manager
    pub fn new(include_dir: &str) -> Self {
        Self {
            include_dir: include_dir.to_string(),
        }
    }

    /// Read all PRG files from directory
    pub fn read_prg_files(&self) -> Result<Vec<PRGFile>, String> {
        let dir = Path::new(&self.include_dir);
        if !dir.exists() || !dir.is_dir() {
            return Err(format!("Include directory does not exist: {}", self.include_dir));
        }

        let entries = fs::read_dir(dir)
            .map_err(|e| format!("Failed to read directory: {}", e))?;

        let mut files = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|e| format!("Failed to read entry: {}", e))?;
            let path = entry.path();

            if path.is_file() {
                if let Some(ext) = path.extension() {
                    if ext.to_ascii_lowercase() == "prg" {
                        files.push(self.parse_prg_file(&path)?);
                    }
                }
            }
        }

        Ok(files)
    }

    /// Parse a PRG file
    fn parse_prg_file(&self, path: &Path) -> Result<PRGFile, String> {
        let bytes = fs::read(path)
            .map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;

        if bytes.len() < 2 {
            return Err(format!(
                "PRG file too small: {} ({} bytes)",
                path.display(),
                bytes.len()
            ));
        }

        // First 2 bytes are load address (little-endian)
        let load_address = (bytes[0] as u16) | ((bytes[1] as u16) << 8);
        let data = bytes[2..].to_vec();

        if data.len() > MAX_FILE_SIZE {
            return Err(format!(
                "File too large: {} ({} bytes, max {})",
                path.display(),
                data.len(),
                MAX_FILE_SIZE
            ));
        }

        let filename = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_string();

        Ok(PRGFile {
            filename,
            load_address,
            data,
            total_size: bytes.len(),
        })
    }

    /// Allocate files to banks
    pub fn allocate_files(
        &self,
        files: &[PRGFile],
        unused_banks: &[usize],
    ) -> Result<Vec<FileAllocation>, String> {
        if files.is_empty() {
            return Ok(Vec::new());
        }

        let mut allocations = Vec::new();
        let mut bank_usage: std::collections::HashMap<usize, usize> = std::collections::HashMap::new();
        let available_banks: Vec<usize> = unused_banks.to_vec();
        let mut filename_offset = 0;

        for file in files {
            let allocation = self.allocate_file(file, &mut bank_usage, filename_offset, &available_banks)?;

            // Calculate filename offset for next file
            let stripped_name = strip_prg_extension(&file.filename);
            filename_offset += stripped_name.len() + 1; // +1 for null terminator

            allocations.push(allocation);
        }

        Ok(allocations)
    }

    /// Allocate a single file to banks
    fn allocate_file(
        &self,
        file: &PRGFile,
        bank_usage: &mut std::collections::HashMap<usize, usize>,
        filename_offset: usize,
        available_banks: &[usize],
    ) -> Result<FileAllocation, String> {
        let file_size = file.data.len();
        let mut banks = Vec::new();
        let mut remaining_size = file_size;

        // Find a bank with enough space or allocate a new one
        let current_bank = bank_usage
            .iter()
            .filter(|(bank, used)| available_banks.contains(bank) && **used < BANK_SIZE_8K && (BANK_SIZE_8K - **used) > 0)
            .min_by_key(|(bank, _)| *bank)
            .map(|(bank, _)| *bank);

        let current_bank = match current_bank {
            Some(bank) => bank,
            None => {
                // Find first available bank not yet in use
                let next_bank = available_banks
                    .iter()
                    .find(|bank| !bank_usage.contains_key(bank))
                    .ok_or_else(|| format!("No more banks available for file: {}", file.filename))?;
                bank_usage.insert(*next_bank, 0);
                *next_bank
            }
        };

        // Record start offset in first bank
        let start_offset = *bank_usage.get(&current_bank).unwrap_or(&0);
        banks.push(current_bank);

        // Calculate how much fits in first bank
        let space_in_bank = BANK_SIZE_8K - start_offset;
        if remaining_size <= space_in_bank {
            // File fits entirely in current bank
            *bank_usage.get_mut(&current_bank).unwrap() += remaining_size;
        } else {
            // File spans multiple banks
            bank_usage.insert(current_bank, BANK_SIZE_8K);
            remaining_size -= space_in_bank;

            // Allocate additional banks
            while remaining_size > 0 && banks.len() < MAX_BANKS_PER_FILE {
                let next_bank = available_banks
                    .iter()
                    .find(|bank| !bank_usage.contains_key(bank))
                    .ok_or_else(|| format!("No more banks available for file: {}", file.filename))?;

                banks.push(*next_bank);
                let chunk_size = remaining_size.min(BANK_SIZE_8K);
                bank_usage.insert(*next_bank, chunk_size);
                remaining_size -= chunk_size;
            }

            if remaining_size > 0 {
                return Err(format!(
                    "File too large to fit in {} banks: {}",
                    MAX_BANKS_PER_FILE, file.filename
                ));
            }
        }

        Ok(FileAllocation {
            file: file.clone(),
            banks,
            start_offset,
            filename_offset,
        })
    }

    /// Get set of all allocated banks
    pub fn get_allocated_banks(&self, allocations: &[FileAllocation]) -> std::collections::HashSet<usize> {
        allocations.iter().flat_map(|a| a.banks.iter().copied()).collect()
    }

    /// Generate metadata block for $B000+ area
    /// Format per entry (16 bytes):
    /// - 2 bytes: pointer to filename
    /// - 8 bytes: bank list (up to 8 banks, $00 = no more banks)
    /// - 2 bytes: start offset in first bank
    /// - 2 bytes: file length
    /// - 2 bytes: load address
    pub fn generate_metadata(&self, allocations: &[FileAllocation]) -> Result<Vec<u8>, String> {
        let metadata_size = (METADATA_END - METADATA_START + 1) as usize;
        let mut metadata = vec![0u8; metadata_size];
        let mut offset = 0;

        for allocation in allocations {
            if offset + METADATA_ENTRY_SIZE > metadata.len() {
                return Err("Too many files - metadata area full".to_string());
            }

            let filename_ptr = FILENAME_START + allocation.filename_offset as u16;

            // Pointer to filename (little-endian)
            metadata[offset] = (filename_ptr & 0xFF) as u8;
            metadata[offset + 1] = ((filename_ptr >> 8) & 0xFF) as u8;
            offset += 2;

            // Bank list (8 bytes)
            for i in 0..MAX_BANKS_PER_FILE {
                if i < allocation.banks.len() {
                    metadata[offset] = allocation.banks[i] as u8;
                } else {
                    metadata[offset] = 0x00;
                }
                offset += 1;
            }

            // Start offset in first bank (little-endian)
            metadata[offset] = (allocation.start_offset & 0xFF) as u8;
            metadata[offset + 1] = ((allocation.start_offset >> 8) & 0xFF) as u8;
            offset += 2;

            // File length (little-endian)
            let file_len = allocation.file.data.len();
            metadata[offset] = (file_len & 0xFF) as u8;
            metadata[offset + 1] = ((file_len >> 8) & 0xFF) as u8;
            offset += 2;

            // Load address (little-endian)
            metadata[offset] = (allocation.file.load_address & 0xFF) as u8;
            metadata[offset + 1] = ((allocation.file.load_address >> 8) & 0xFF) as u8;
            offset += 2;
        }

        Ok(metadata)
    }

    /// Generate filename block for $B800+ area
    /// Filenames are stored as PETSCII, null-terminated, WITHOUT .prg extension
    pub fn generate_filenames(&self, allocations: &[FileAllocation]) -> Result<Vec<u8>, String> {
        let max_size = (FILENAME_END - FILENAME_START + 1) as usize;
        let mut filenames = vec![0u8; max_size];
        let mut offset = 0;

        for allocation in allocations {
            let name_without_ext = strip_prg_extension(&allocation.file.filename);
            let petscii_bytes: Vec<u8> = name_without_ext.bytes().map(ascii_to_petscii).collect();

            if offset + petscii_bytes.len() + 1 > max_size {
                return Err("Filename area full".to_string());
            }

            filenames[offset..offset + petscii_bytes.len()].copy_from_slice(&petscii_bytes);
            offset += petscii_bytes.len();
            filenames[offset] = 0x00; // Null terminator
            offset += 1;
        }

        Ok(filenames)
    }

    /// Write file data to banks in CRTBuilder
    pub fn write_files_to_banks(
        &self,
        crt: &mut CRTBuilder,
        allocations: &[FileAllocation],
    ) -> Result<(), String> {
        for allocation in allocations {
            let file = &allocation.file;
            let mut data_offset = 0;
            let mut remaining_size = file.data.len();

            for (bank_index, &bank_number) in allocation.banks.iter().enumerate() {
                let bank = crt.get_bank_mut(bank_number)?;
                let start_offset = if bank_index == 0 {
                    allocation.start_offset
                } else {
                    0
                };
                let chunk_size = remaining_size.min(BANK_SIZE_8K - start_offset);

                bank[start_offset..start_offset + chunk_size]
                    .copy_from_slice(&file.data[data_offset..data_offset + chunk_size]);

                data_offset += chunk_size;
                remaining_size -= chunk_size;
            }
        }

        Ok(())
    }
}

/// Strip .prg/.PRG extension from filename if present
fn strip_prg_extension(filename: &str) -> String {
    if filename.len() > 4 && filename[filename.len() - 4..].eq_ignore_ascii_case(".prg") {
        filename[..filename.len() - 4].to_string()
    } else {
        filename.to_string()
    }
}

/// Convert ASCII character to PETSCII uppercase
fn ascii_to_petscii(ascii: u8) -> u8 {
    match ascii {
        // ASCII lowercase a-z (0x61-0x7A) → PETSCII uppercase A-Z (0x41-0x5A)
        0x61..=0x7A => ascii - 0x20,
        // Everything else stays the same
        _ => ascii,
    }
}
