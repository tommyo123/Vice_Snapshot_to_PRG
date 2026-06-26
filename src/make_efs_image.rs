//! Builds EasyFlash filesystem (EFS) area images for libefs.
//!
//! A Rust port of the directory/file layout produced by drunella's
//! `tools/mkefs.py`, so the converter can pre-seed both the read-only area and
//! the rewritable area (with default files) without the Python toolchain.
//!
//! An area is two byte streams:
//!   - `dir`   : the 6 KB ($1800) directory (24-byte entries, $FF = empty/terminator)
//!   - `files` : the concatenated file payloads (each a full PRG incl. load address)
//!
//! The directory entry (24 bytes), matching `lib-efs.i`:
//!   +0  name (16 bytes, PETSCII-uppercase, NUL padded)
//!   +16 flags  ($60 | hidden($80) | type)
//!   +17 bank
//!   +18 bank-high (0)
//!   +19 start offset within the bank window (word, little-endian)
//!   +21 file size (3 bytes, little-endian)
//
// Copyright (c) 2025-2026 Tommy Olsen
// Licensed under the MIT License.

#![allow(dead_code)]

use std::fs;
use std::path::Path;

pub const EFS_DIR_SIZE: usize = 0x1800;

/// Banking divisor: 8 KB per bank window for ll/hh, 16 KB for lh.
pub const DIVISOR_8K: usize = 0x2000;
pub const DIVISOR_16K: usize = 0x4000;

/// A file to place in an EFS area (full PRG bytes, including the load address).
pub struct EfsFile {
    pub name: String,
    pub data: Vec<u8>,
}

/// A built EFS area: its directory and concatenated file data.
pub struct EfsArea {
    pub dir: Vec<u8>,
    pub files: Vec<u8>,
}

impl EfsArea {
    /// Number of 8 KB bank windows the file data occupies, given the placement
    /// `offset` (e.g. $1800 when the directory shares the leading banks).
    pub fn file_banks(&self, offset: usize, divisor: usize) -> usize {
        if self.files.is_empty() {
            return 0;
        }
        (offset + self.files.len()).div_ceil(divisor)
    }
}

/// Read PRG files from a directory into [`EfsFile`]s (name = filename without the
/// `.prg` extension, uppercased; data = the full file including load address).
pub fn read_prg_dir(dir: &str) -> Result<Vec<EfsFile>, String> {
    let path = Path::new(dir);
    if !path.is_dir() {
        return Err(format!("Not a directory: {}", dir));
    }
    let mut out = Vec::new();
    let mut entries: Vec<_> = fs::read_dir(path)
        .map_err(|e| format!("Failed to read {}: {}", dir, e))?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.extension().is_some_and(|x| x.eq_ignore_ascii_case("prg")))
        .collect();
    entries.sort();
    for p in entries {
        let data = fs::read(&p).map_err(|e| format!("Failed to read {}: {}", p.display(), e))?;
        if data.len() < 2 {
            return Err(format!("PRG too small: {}", p.display()));
        }
        let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("FILE");
        out.push(EfsFile { name: stem.to_uppercase(), data });
    }
    Ok(out)
}

/// Build an EFS area image (port of mkefs.py).
///
/// - `start_bank`: the bank the file data begins in.
/// - `offset`: logical byte offset of the first file within the area window
///   (0 for the read-only area; $1800 for a rewritable area whose directory
///   occupies the leading banks).
/// - `divisor`: [`DIVISOR_8K`] or [`DIVISOR_16K`].
pub fn build_efs_area(
    files: &[EfsFile],
    start_bank: u8,
    offset: usize,
    divisor: usize,
) -> Result<EfsArea, String> {
    let mut dir = vec![0xFFu8; EFS_DIR_SIZE];
    let mut dir_ptr = 0usize;
    let mut data: Vec<u8> = Vec::new();

    for f in files {
        let file_off = data.len();
        data.extend_from_slice(&f.data);

        let off2 = file_off + offset;
        let bank = off2 / divisor + start_bank as usize;
        let start = off2 % divisor;
        if bank > 0xFF {
            return Err("EFS area too large (bank > 255)".to_string());
        }
        if dir_ptr + 24 > EFS_DIR_SIZE {
            return Err("Too many files for the EFS directory".to_string());
        }

        let name = name_to_petscii(&f.name);
        let sz = f.data.len();
        let e = &mut dir[dir_ptr..dir_ptr + 24];
        e.fill(0); // entries are zero-padded (over the $FF background)
        let n = name.len().min(16);
        e[..n].copy_from_slice(&name[..n]);
        e[16] = 0x60 | 0x01; // normal PRG file
        e[17] = bank as u8;
        e[18] = 0;
        e[19] = (start & 0xFF) as u8;
        e[20] = ((start >> 8) & 0xFF) as u8;
        e[21] = (sz & 0xFF) as u8;
        e[22] = ((sz >> 8) & 0xFF) as u8;
        e[23] = ((sz >> 16) & 0xFF) as u8;
        dir_ptr += 24;
    }
    // The $FF background after the last entry is the directory terminator.

    Ok(EfsArea { dir, files: data })
}

/// ASCII -> PETSCII for filenames (lowercase a-z -> $41-$5A so it matches a
/// C64 `LOAD"NAME"`; other printables pass through).
fn name_to_petscii(name: &str) -> Vec<u8> {
    name.bytes()
        .map(|c| match c {
            b'a'..=b'z' => c - 0x20,
            0x20..=0x7E => c,
            _ => 0x20,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_directory_entry() {
        let files = vec![EfsFile { name: "score".into(), data: vec![0x01, 0x08, 0xAA, 0xBB] }];
        // rewritable-style: dir occupies the leading $1800, files after it.
        let area = build_efs_area(&files, 56, 0x1800, DIVISOR_8K).unwrap();
        assert_eq!(area.files, vec![0x01, 0x08, 0xAA, 0xBB]);
        // entry 0
        assert_eq!(&area.dir[0..5], b"SCORE");
        assert_eq!(area.dir[16], 0x61); // type
        assert_eq!(area.dir[17], 56); // bank ( $1800/$2000 = 0 + 56 )
        assert_eq!(area.dir[19], 0x00); // start lo
        assert_eq!(area.dir[20], 0x18); // start hi ($1800)
        assert_eq!(area.dir[21], 0x04); // size = 4
        // terminator: next entry's type byte is $FF
        assert_eq!(area.dir[24 + 16], 0xFF);
    }

    #[test]
    fn read_only_area_offset_zero() {
        let files = vec![EfsFile { name: "data".into(), data: vec![0u8; 0x2500] }];
        let area = build_efs_area(&files, 5, 0, DIVISOR_8K).unwrap();
        assert_eq!(area.dir[17], 5); // first file in start_bank
        assert_eq!(area.file_banks(0, DIVISOR_8K), 2); // 0x2500 -> 2 banks
    }
}
