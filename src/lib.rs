//! VICE Snapshot to PRG Converter Library
//!
//! This library provides the core functionality for converting VICE 3.6-3.9 x64sc
//! snapshot files to self-restoring C64 PRG files.
//!
//! This program is unlicensed and dedicated to the public domain.
//! Developed by Tommy Olsen.

pub mod asm_wrapper;
pub mod config;
pub mod convert_snapshot;
pub mod find_ram;
pub mod make_prg_asm;
pub mod parse_vsf;
pub mod patch_mem;
