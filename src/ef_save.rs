//! EasyFlash SAVE support — integration of drunella's libefs.
//!
//! This module embeds the prebuilt EasyFlash filesystem library (libefs) and the
//! EAPI flash driver, and generates the on-flash configuration block that tells
//! libefs where its read-only and rewritable areas live.
//!
//! libefs (Apache-2.0, https://github.com/Drunella/libefs) provides a KERNAL-like
//! file API over EasyFlash flash, with a read-only area plus two rewritable areas
//! that ping-pong for garbage collection. We point the KERNAL SAVE/LOAD vectors at
//! its `EFS_save`/`EFS_load` entry points.
//!
//! On-flash layout of the writable filesystem (see vendor/libefs):
//!   - libefs code: bank 0 LOROM ($8000), copied to EF-RAM $DF00 by EFS_init
//!   - EAPI:        bank 0 HIROM $B800 (768 bytes)
//!   - config:      bank 0 HIROM $BB00 (24-byte EF name + 40-byte LIBEFS config)
//!   - read-only files: area 0 (low banks)
//!   - rewritable files: areas 1 (LOROM) + 2 (HIROM) of the reserved top banks
//
// Copyright (c) 2025-2026 Tommy Olsen
// Licensed under the MIT License.

#![allow(dead_code)]

/// Prebuilt libefs library (`.prg`, loads at $8000). First 2 bytes are the load
/// address; [`lib_efs_code`] returns the code without that header.
const LIB_EFS_PRG: &[u8] = include_bytes!("../vendor/libefs/lib-efs.prg");

/// Prebuilt EAPI for AM29F040 (`.prg`, page-aligned, 768 bytes of code).
const EAPI_PRG: &[u8] = include_bytes!("../vendor/libefs/eapi-am29f040.prg");

// ---- libefs init entry points (bank 0 LOROM) ----
pub const EFS_INIT: u16 = 0x8000;
pub const EFS_INIT_EAPI: u16 = 0x8003;
pub const EFS_INIT_MINIEAPI: u16 = 0x8006;
pub const EFS_DEFRAGMENT: u16 = 0x8009;
pub const EFS_FORMAT: u16 = 0x800C;
pub const EFS_VALIDATE: u16 = 0x800F;

// ---- libefs runtime API (EF-RAM jump table, valid after EFS_init) ----
pub const EFS_UTIL: u16 = 0xDF00;
pub const EFS_SETNAM: u16 = 0xDF06;
pub const EFS_LOAD: u16 = 0xDF0C;
pub const EFS_OPEN: u16 = 0xDF12;
pub const EFS_CLOSE: u16 = 0xDF18;
pub const EFS_CHRIN: u16 = 0xDF1E;
pub const EFS_SAVE: u16 = 0xDF24;
pub const EFS_CHROUT: u16 = 0xDF2A;
pub const EFS_READST: u16 = 0xDF30;

/// Where the 24-byte EF name + 40-byte config block lives in bank 0 HIROM
/// (offset $1B00; address $BB00 when ROMH is at $A000 in 16K mode).
pub const EFS_NAME_OFFSET_IN_BANK: usize = 0x1B00;
/// Where EAPI lives in bank 0 HIROM (offset $1800; address $B800).
pub const EAPI_OFFSET_IN_BANK: usize = 0x1800;

/// libefs banking modes (per-area, from the EAPI banking conventions).
pub const MODE_LHLH: u8 = 0xD0; // alternate LOROM/HIROM, 16K per bank pair
pub const MODE_LLLL: u8 = 0xB0; // LOROM only, 8K per bank
pub const MODE_HHHH: u8 = 0xD4; // HIROM only, 8K per bank

/// One libefs storage area (6 config bytes).
#[derive(Clone, Copy, Debug)]
pub struct EfsArea {
    pub dir_bank: u8,
    pub dir_high: u8,
    pub files_bank: u8,
    pub files_high: u8,
    /// Number of 8K banks; 0 = auto/ignore (read-only area only). Must be a
    /// multiple of 8 (one 64K erase sector) for rewritable areas.
    pub num_banks: u8,
    pub mode: u8,
}

impl EfsArea {
    fn to_bytes(self) -> [u8; 6] {
        [
            self.dir_bank,
            self.dir_high,
            self.files_bank,
            self.files_high,
            self.num_banks,
            self.mode,
        ]
    }
}

/// libefs storage configuration: one read-only area plus two rewritable areas
/// (areas 1 and 2 ping-pong during garbage collection).
#[derive(Clone, Copy, Debug)]
pub struct EfsConfig {
    pub area0: EfsArea,
    pub area1: EfsArea,
    pub area2: EfsArea,
}

impl EfsConfig {
    /// Default layout: read-only area 0 (directory bank 0 HIROM, files from bank
    /// 1 LOROM), and two 64K rewritable areas that ping-pong for garbage
    /// collection, at `first_rw_bank..+8` and `first_rw_bank+8..+16`.
    ///
    /// Both rewritable areas live in **HIROM (chip 1)**. libefs's code executes
    /// from chip 0 (bank 0 LOROM); an AM29F040 sector erase makes the *entire
    /// chip* unreadable until it finishes, so if a rewritable area shared chip 0
    /// with libefs, the defragment erase would make the running code vanish
    /// mid-execution and crash the CPU. Keeping both rw areas on chip 1 means the
    /// erase never touches the chip libefs runs from.
    pub fn with_top_rw_sector(first_rw_bank: u8) -> Self {
        Self {
            area0: EfsArea { dir_bank: 0, dir_high: 0xA0, files_bank: 1, files_high: 0x80, num_banks: 0, mode: MODE_LHLH },
            area1: EfsArea { dir_bank: first_rw_bank, dir_high: 0xA0, files_bank: first_rw_bank, files_high: 0xA0, num_banks: 8, mode: MODE_HHHH },
            area2: EfsArea { dir_bank: first_rw_bank + 8, dir_high: 0xA0, files_bank: first_rw_bank + 8, files_high: 0xA0, num_banks: 8, mode: MODE_HHHH },
        }
    }

    /// Create layout with custom starting bank and area sizes (in banks) for the save areas.
    pub fn with_rw_layout(first_rw_bank: u8, area_size_banks: u8) -> Self {
        Self {
            area0: EfsArea { dir_bank: 0, dir_high: 0xA0, files_bank: 1, files_high: 0x80, num_banks: 0, mode: MODE_LHLH },
            area1: EfsArea { dir_bank: first_rw_bank, dir_high: 0xA0, files_bank: first_rw_bank, files_high: 0xA0, num_banks: area_size_banks, mode: MODE_HHHH },
            area2: EfsArea { dir_bank: first_rw_bank + area_size_banks, dir_high: 0xA0, files_bank: first_rw_bank + area_size_banks, files_high: 0xA0, num_banks: area_size_banks, mode: MODE_HHHH },
        }
    }
}

/// libefs code (bank 0 LOROM contents) without the 2-byte `.prg` load address.
pub fn lib_efs_code() -> &'static [u8] {
    &LIB_EFS_PRG[2..]
}

/// EAPI code (768 bytes) without the 2-byte `.prg` load address.
pub fn eapi_code() -> &'static [u8] {
    &EAPI_PRG[2..]
}

/// PETSCII-ish name byte (uppercase ASCII passes through).
fn name_byte(c: u8) -> u8 {
    match c {
        b'a'..=b'z' => c - 0x20,
        0x20..=0x7E => c,
        _ => 0x20,
    }
}

/// Generate the 64-byte block at bank 0 HIROM offset $1B00: a 24-byte EF name
/// followed by the 40-byte LIBEFS configuration block (at $1B18).
///
/// Defragmentation callbacks are left disabled here (Phase 1); they can be added
/// later by pointing the two vectors at a routine in bank 0.
pub fn generate_efs_name_and_config(cart_name: &str, cfg: &EfsConfig) -> [u8; 64] {
    let mut block = [0u8; 64];

    // --- 24-byte EF name ($1B00-$1B17) ---
    // 8-byte tag used by drunella's tooling, then a 16-byte name field.
    block[0..8].copy_from_slice(&[0x65, 0x66, 0x2D, 0x6E, 0x41, 0x4D, 0x45, 0x3A]);
    for (i, c) in cart_name.bytes().take(16).enumerate() {
        block[8 + i] = name_byte(c);
    }

    // --- 40-byte LIBEFS config ($1B18-$1B3F) ---
    // Signature is "LIBEFS" in PETSCII (uppercase). drunella's source writes
    // `.byte "libefs"` but ca65's c64 charmap uppercases it; libefs checks for
    // $4C,$49,$42,$45,$46,$53. Writing lowercase ASCII here makes libefs fall
    // back to its read-only default config (-> "device not present" on save).
    let cfg_off = 0x18;
    block[cfg_off..cfg_off + 6].copy_from_slice(&[0x4C, 0x49, 0x42, 0x45, 0x46, 0x53]);
    // [1E-20] reserved zeros (already 0)
    block[cfg_off + 9] = 0x03; // mode: read-only area + two rewritable areas
    block[cfg_off + 10..cfg_off + 16].copy_from_slice(&cfg.area0.to_bytes());
    block[cfg_off + 16..cfg_off + 22].copy_from_slice(&cfg.area1.to_bytes());
    block[cfg_off + 22..cfg_off + 28].copy_from_slice(&cfg.area2.to_bytes());
    // [34] defrag callback enabled = 0 (disabled for now)
    // [35-38] defrag vectors = 0
    // [39-3F] unused = 0

    block
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blobs_have_expected_load_addresses() {
        // lib-efs.prg loads at $8000, EAPI is page-aligned.
        assert_eq!(LIB_EFS_PRG[0], 0x00);
        assert_eq!(LIB_EFS_PRG[1], 0x80);
        assert_eq!(eapi_code().len(), 768);
        // libefs jump table: EFS_init = JMP (4C ...)
        assert_eq!(lib_efs_code()[0], 0x4C);
    }

    #[test]
    fn config_block_matches_libefs_layout() {
        let cfg = EfsConfig::with_top_rw_sector(48);
        let block = generate_efs_name_and_config("VICE SNAPSHOT", &cfg);

        // EF name tag.
        assert_eq!(&block[0..8], &[0x65, 0x66, 0x2D, 0x6E, 0x41, 0x4D, 0x45, 0x3A]);
        // LIBEFS signature at $1B18 (PETSCII uppercase).
        assert_eq!(&block[0x18..0x1E], &[0x4C, 0x49, 0x42, 0x45, 0x46, 0x53]);
        // mode byte at $1B21.
        assert_eq!(block[0x21], 0x03);
        // area 0 (read-only) at $1B22: dir bank 0 @ $A0, files bank 1 @ $80, lhlh.
        assert_eq!(&block[0x22..0x28], &[0x00, 0xA0, 0x01, 0x80, 0x00, MODE_LHLH]);
        // area 1 (rw HIROM) at $1B28: banks 48-55, 8 banks, hhhh.
        assert_eq!(&block[0x28..0x2E], &[48, 0xA0, 48, 0xA0, 8, MODE_HHHH]);
        // area 2 (rw HIROM) at $1B2E: banks 56-63, 8 banks, hhhh.
        assert_eq!(&block[0x2E..0x34], &[56, 0xA0, 56, 0xA0, 8, MODE_HHHH]);
    }
}
