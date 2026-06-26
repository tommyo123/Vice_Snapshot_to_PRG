# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- **EasyFlash SAVE - persistent flash filesystem** (`--ef-save`) - produces an EasyFlash cartridge that restores the snapshot *and* gives the running program a read/write flash filesystem, by embedding drunella's [libefs](https://github.com/Drunella/libefs) (Apache-2.0) and the AM29F040 EAPI driver
    - The KERNAL `LOAD`/`SAVE` vectors are hooked to libefs through a small RAM trampoline; `LOAD"NAME",8,1` and `SAVE"NAME",8` read and write flash with no program changes
    - **Two directories** - a read-only area (`--include-dir`, e.g. a full D81's worth of files that never change) plus a rewritable area pre-seeded with default files (`--rw-dir`, e.g. a starting high-score table). `LOAD` transparently searches both
    - **Persistent overwrites** - a plain `SAVE"NAME",8` over an existing file is auto-promoted to the CBM replace command (`@0:`), so high-scores / save-games rotate in place: the old directory entry is invalidated and the new file is appended to free flash. Programs that pass their own `@...` command are left untouched
    - **Automatic garbage collection** - the rewritable area is two ping-pong halves; when one fills with live + invalidated files, libefs copies the live files to the other half and erases the full sector, transparently during a `SAVE`. Both halves live in HIROM (chip 1), keeping the sector erase off the chip libefs executes from. Verified across many save/defragment cycles with no data loss or session reset
    - Reserves 128 KB of the 1 MB flash for the rewritable area (two 64 KB ping-pong halves); the rest holds the restore payload and the read-only files
    - **C64 RAM use** - the cartridge needs a little free C64 RAM for the running engine: ~300 bytes (below `$8000`) for the LOAD/SAVE trampoline and a page-aligned ~1 KB buffer (in `$0000-$0FFF`, the only RAM reachable in Ultimax mode) for the AM29F040 flash driver, which must execute from RAM during a write. Both are auto-placed in free RAM
        - `--trampoline <addr>` overrides where the trampoline goes (e.g. a fixed scratch region for a RAM-hungry game); `tape`/`stack` are accepted names but are too small for the ~300-byte trampoline
        - `--eapi-buffer <auto|screen|addr>` controls the flash buffer. **`auto`** (default) uses free RAM in `$0900-$0FFF`, and if there is none **falls back to the running program's screen RAM** — the screen is only clobbered during the LOAD/SAVE and the program redraws it afterward, so a RAM-full game can still save. `screen` forces that; an explicit address must be page-aligned in `$0400-$0C00` (VIC bank 0)
    - To persist changes back to the `.crt` on disk, run VICE with `-easyflashcrtwrite` (writes the image back on a clean exit/detach)
    - Embeds a locally-patched libefs (upstream `42e5570`'s defragment path jumps to a bogus address even with its callbacks disabled — it reads the config from the wrong flash bank; see `vendor/libefs/defragment-callback-fix.patch`)
    - **GUI** - "EasyFlash SAVE" is now a cartridge type in the dropdown; selecting it reveals the rewritable-defaults directory picker and the flash-buffer placement control (Auto / Screen RAM / Custom address), at full parity with the CLI
    - **EasyFlash SAVE Directory Viewer** - added a standalone Python utility `show_save_dir.py` that lists files in the save areas of persistent save cartridges, displaying status (Active, Overwritten, Deleted), sizes, banks, offsets, and flags
    - **Screen RAM Stashing & Swapping** (`--force-stash`) - implemented stashing and swapping of screen matrix data to a designated free RAM region during save/load operations when the EAPI buffer falls back to the screen RAM
    - **Screen Blanking Option** (`--force-blank`) - added option to temporarily blank the screen display during write/erase operations to improve write stability
    - **$C000-$CFFF RAM Range Search** - extended auto-placement search to utilize the `$C000-$CFFF` range for the trampoline and screen stash
    - **GUI Settings for Stash & Blank** - added "Force screen stash" and "Force screen blank" checkboxes, and automated prepopulation of the LOAD/SAVE directories in the GUI when selecting a snapshot
    - **Complete KERNAL I/O Channel Hooking** - hooked vectors for `OPEN` ($031C), `CLOSE` ($031E), `CHKIN` ($0320), `CKOUT` ($0322), `CLRCHN` ($0324), `CHRIN` ($0326), and `CHROUT` ($0328) to point to the EFS trampoline, allowing transparent character-by-character and sequential channel file operations. Includes backup and restore of original vectors to maintain pass-through compatibility for other devices.
- **Magic Desk LOAD/SAVE hooking** - Magic Desk CRTs can now embed PRG files and intercept `LOAD "NAME",8,1`, identical to the EasyFlash feature
    - `--include-dir` now works with `--magic-desk`; the GUI LOAD/SAVE hook option is enabled for both cartridge types
    - Bank 0 becomes a directory bank: boot code (`$8000`), LOAD handler (`$8400`), file metadata (`$9000`), filenames (`$9800`); the restore payload moves to banks 1+ and file data follows
    - Only a small trampoline lives in C64 RAM (cassette buffer `$0334`); during a LOAD it banks the directory bank in, copies the file straight from ROM, and banks the cartridge back out
- **Automatic power-on RAM pattern clearing** - Detects the C64/VICE power-on RAM pattern (4-byte `$00`/`$FF` runs, inverted every 8 KB) and zeroes the regions still holding it, recovering them as free space
    - Snapshots taken without a manual `f 0000 ffff 00` (e.g. Smart Attach) now convert far more reliably
    - Strict, conservative match: only memory the program never wrote is cleared; a wrong match can only fail to clear, never corrupt data
    - Applied to PRG, EasyFlash and Magic Desk output

### Changed
- Magic Desk `$DE00` bit 7 is now treated as a reversible cartridge bank-out (it only drives EXROM), correcting the earlier assumption that it was a permanent disable - this is what makes the Magic Desk LOAD hook possible
- README updated to document Magic Desk LOAD hooking and automatic power-on pattern clearing
- **Highest-First Memory Allocation** - changed `FindRam` range allocator to pick the highest available free blocks, minimizing collision risks with BASIC programs starting at `$0801`
- **EAPI Buffer Optimization** - reduced EAPI buffer size to 768 bytes and offset the screen fallback to `$0500`, leaving the top of the screen display (`$0400-$04FF`) untouched
- **CPU Port `$01` Preservation** - trampoline templates now save and restore register `$01` state to prevent KERNAL memory map corruption
- **Noise-Tolerant Power-On RAM Clearing** - matches power-on RAM patterns with up to 3 consecutive anomalies/noise bytes, consolidating uninitialized free blocks

### Fixed
- `find_ram` unit tests (6) asserted behaviour that contradicted the whole-RAM free-block scan (they used an all-zero background); rewritten to use a realistic varied background, with added tests for the power-on pattern detection

### Technical Details
- New module: `make_magic_desk_load_save` (RAM trampoline + cart-resident LOAD handler, `$DE00`-only banking)
- `make_magic_desk_boot_asm` / `make_magic_desk_crt_asm`: restore payload starts at bank 1 when files are embedded; bank-correct data-copy source address
- `file_system_manager`: configurable filename base address (`$9800` for Magic Desk)
- `find_ram`: `poweron_pattern_byte` + `clear_poweron_pattern`

## [2.2.0] - 2026-05-29

### Added
- Extended VSF compatibility to file-format version 1.0 (VICE 1.15–2.3 era snapshots)

## [2.1.0] - 2026-04-22

### Added
- **Broader VSF compatibility** - Accept both VSF file-format versions 1.1 and 2.0
- **Non-cycle-accurate VIC-II support** - VSFs from the plain-C64 emulator are now parsed alongside the cycle-accurate variant
- **Per-module version dispatch** - MAINCPU, C64MEM, SID and VIC-II parsers pick their layout from the module header (`major.minor`) rather than the file-level version, handling format shifts across VICE releases

### Changed
- Header parser no longer requires the "VICE Version" block that was introduced in newer VICE builds; absence is detected and handled
- User-facing text (window title, CLI help, installer metadata, README) no longer claims a fixed VICE version range
- Error messages for unsupported format/machine fields shortened

### Fixed
- "Module '' beyond EOF" when loading older VSFs that lacked the "VICE Version" header block

### UI
- FLTK scheme switched from `gtk+` to `oxy` for a lighter look; colors unchanged

## [2.0.0] - 2026-02-27

### Added
- **Magic Desk CRT output** - New cartridge format option
    - 8K cart mode: ROML only (`$8000–$9FFF`), no ROMH
    - CBM80 boot signature with trampoline-based restore
    - Permanent cartridge disable via `$DE00` bit 7
    - Minimum 8 banks, maximum 64 banks (512 KB)
    - Hardware type 19, EXROM=0, GAME=1
- **Cartridge type selector in GUI** - Dropdown to choose between EasyFlash and Magic Desk
    - LOAD/SAVE hooking automatically disabled for Magic Desk
- **CLI `--magic-desk` flag** - Force Magic Desk CRT format output
- **Broader VICE compatibility** - Tested across additional VICE releases

### Changed
- **GUI improvements**
    - CRT tab renamed from "CRT Output (EasyFlash)" to "CRT Output"
    - Output filename now defaults to the snapshot filename with the correct extension (.prg/.crt)
    - Output label updated from "EasyFlash CRT" to generic "CRT"
- **CLI help text** updated to document all three output formats and Magic Desk options
- **Documentation and UI** updated with current compatibility notes

### Technical Details
- New modules: `convert_snapshot_magic_desk_crt`, `make_magic_desk_boot_asm`, `make_magic_desk_crt_asm`
- `CartridgeType` enum extended with `MagicDesk` variant (chip type 0 = ROM vs EasyFlash chip type 2 = Flash)
- Magic Desk uses byte-level copy loop from ROML with bank switching at `$A000` boundary
- Two-pass assembly for boot code and restore code size calculation

## [1.9.0] - 2025-12-04

### Added
- **EasyFlash CRT output** - Convert snapshots to bootable EasyFlash cartridges
    - Boots directly from cartridge without loading
    - Same full machine state restoration as PRG
- **LOAD/SAVE hooking for CRT** - Embed PRG files in cartridge ROM
    - Intercepts KERNAL LOAD vector to serve files from ROM
    - SAVE is silently ignored (ROM is read-only)
    - Trampoline auto-placed at `$0100` or `$0334` based on stack pointer
    - Files indexed with 16-char PETSCII filenames
- **Manual RAM block specification** - GUI dialog to add free blocks when auto-detection fails
    - Specify address range for unused memory
    - Region is zeroed before compression
- **CLI CRT support** - New options for CRT generation
    - `--crt` / `--prg` flags (auto-detected from extension)
    - `--name <name>` for cartridge name (max 32 chars)
    - `--include-dir <dir>` to embed PRG files

### Changed
- CLI renamed conceptually to PRG/CRT converter
- README rewritten for clarity

## [1.0.0] - 2025-10-22

### Added
- **Block 10 restoration stage** - New intermediate restoration block for improved memory allocation
    - Splits restoration into three stages: Block 9 → Block 10 → Final restore code
    - Significantly improves success rate by reducing Block 9 size requirements
    - Makes it easier to allocate restoration code in fragmented memory
- **VICE compatibility verified** - Tested across multiple VICE releases

### Changed
- **Optimized final restore code** - Reduced memory footprint of restoration code in `$01xx`
    - More efficient register handling
    - Streamlined interrupt configuration
    - Smaller code size allows for better stack pointer placement
- **Improved memory allocation strategy** - Two-block architecture (Block 9 + Block 10) instead of single large block
    - Block 9: Core restore + wipe blocks 1-8 + jump to Block 10
    - Block 10: Wipe Block 9 + restore `$F8-$FF` + setup registers + jump to `$01xx`
    - Reduces maximum contiguous memory requirement
- **Enhanced CIA timer restoration** - More robust timer initialization sequence
    - Timers configured but not started until final stage
    - Prevents premature interrupt generation during restoration
- **License change** - Changed from CC0 (public domain) to MIT License
    - Provides better legal clarity
    - Maintains open source spirit with minimal restrictions

### Fixed
- Allocation failures in snapshots with fragmented memory
- Edge cases where large restoration blocks couldn't be allocated
- Improved reliability across VICE releases

### Technical Improvements
- Three-stage restoration architecture improves modularity
- Better separation of concerns in restoration process
- Reduced code complexity in individual restoration stages
- More predictable memory requirements

### Known Limitations
- Requires memory initialization (`f 0000 ffff 00` + `reset`) before snapshot
- Stack pointer placement may be risky in edge cases with unusual stack configurations
- "Smart attach..." should be avoided unless VICE is configured to initialize memory to zeros on reset
- macOS version is untested (no access to macOS hardware for verification)
- Linux binaries require Ubuntu 24.04+, Debian 12+, or compatible distributions
- Windows 7 is not supported (requires Windows 8 or later)

## [0.9.1] - 2024-10-19

### Added
- **CLI version** (`vice-snapshot-to-prg-converter-cli`) for command-line automation and scripting
    - Simple syntax: `vice-snapshot-to-prg-converter-cli input.vsf output.prg`
    - Automatically overwrites output files without prompting
    - Included in all distribution packages (Windows MSI, portable, Linux, macOS)
- **Portable Windows package** (ZIP) - no installation required, includes both GUI and CLI
- **Pre-compiled Linux binaries** (x86_64, built on Ubuntu 24.04)
    - Compatible with Ubuntu 24.04+ and Debian 12+
    - Complete dependency bundling
- **Pre-compiled macOS binaries** (x86_64, untested)
- **Customizable installation path** in Windows MSI installer
- Comprehensive platform-specific README files in all packages

### Changed
- **Replaced external vasm assembler** with embedded [asm6502](https://github.com/tommyo123/asm6502) Rust library
    - Eliminates external dependencies
    - Improved error reporting with line-level assembly diagnostics
    - Cross-platform assembly without external tools
- **Replaced external LZSA client** with [lzsa-sys](https://github.com/tommyo123/lzsa-sys) Rust wrapper
    - C library wrapper around Emmanuel Marty's LZSA compression code
    - Native LZSA1 compression without spawning external processes
    - Better integration and error handling
- **Refactored codebase** to be platform-independent
    - Removed Windows-specific code paths
    - Unified temporary directory handling across platforms
    - Library structure (`src/lib.rs`) enables code reuse between GUI and CLI
- Build process simplified - no external assembler or compression tools needed
- GitHub Actions workflow streamlined without verbose output

### Fixed
- Cross-platform compatibility issues with path handling
- Temporary file cleanup now consistent across all platforms
- Assembly error messages now include line numbers and context

### Technical Improvements
- Modular project structure with separate GUI (`src/main.rs`) and CLI (`src/cli/main.rs`)
- Shared core library for both GUI and CLI versions
- Improved error messages with contextual information
- Cleaner build output in CI/CD pipeline

### Distribution Packages
All packages now include both GUI and CLI versions:
- **Windows MSI**: Installer with customizable path, shortcuts for both GUI and CLI
- **Windows Portable**: ZIP archive, run from anywhere, no installation
- **Linux tar.gz**: Self-contained binaries with all dependencies
- **macOS tar.gz**: Self-contained binaries (untested on actual hardware)

## [0.9.0] - 2024-10-14

### Added
- Initial beta release
- GUI application for converting VICE snapshots to PRG files
- LZSA1 compression for efficient snapshot compression
- Automatic memory patching and restoration code generation
- MSI installer with WiX
- Smart vcruntime140.dll bundling (only if VC++ Redistributable not installed)
- Desktop and Start Menu shortcuts
- Complete documentation in README.md

### Known Limitations
- Requires memory initialization (`f 0000 ffff 00` + `reset`) before snapshot
- Stack pointer placement may be risky in edge cases
- "Smart attach..." feature in VICE should be avoided

[Unreleased]: https://github.com/tommyo123/Vice_Snapshot_to_PRG/compare/v2.2.0...HEAD
[2.2.0]: https://github.com/tommyo123/Vice_Snapshot_to_PRG/compare/v2.1.0...v2.2.0
[2.1.0]: https://github.com/tommyo123/Vice_Snapshot_to_PRG/compare/v2.0.0...v2.1.0
[2.0.0]: https://github.com/tommyo123/Vice_Snapshot_to_PRG/compare/v1.9.0...v2.0.0
[1.9.0]: https://github.com/tommyo123/Vice_Snapshot_to_PRG/compare/v1.0.0...v1.9.0
[1.0.0]: https://github.com/tommyo123/Vice_Snapshot_to_PRG/compare/v0.9.1...v1.0.0
[0.9.1]: https://github.com/tommyo123/Vice_Snapshot_to_PRG/compare/v0.9.0...v0.9.1
[0.9.0]: https://github.com/tommyo123/Vice_Snapshot_to_PRG/releases/tag/v0.9.0
