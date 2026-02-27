//! VICE Snapshot to PRG/CRT Converter Library
//!
//! This library provides the core functionality for converting VICE 3.6-3.10 x64sc
//! snapshot files to self-restoring C64 PRG files, EasyFlash CRT or Magic Desk CRT cartridges.
//!
// Copyright (c) 2025 Tommy Olsen
// Licensed under the MIT License.

pub mod asm_wrapper;
pub mod config;
pub mod convert_snapshot;
pub mod find_ram;
pub mod make_prg_asm;
pub mod parse_vsf;
pub mod patch_mem;

// CRT/EasyFlash modules
pub mod convert_snapshot_crt;
pub mod crt_builder;
pub mod file_system_manager;
pub mod load_save_hook;
pub mod make_crt_asm;
pub mod make_romh_asm;

// CRT/Magic Desk modules
pub mod convert_snapshot_magic_desk_crt;
pub mod make_magic_desk_boot_asm;
pub mod make_magic_desk_crt_asm;
