# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
### Changed
### Fixed
### Removed

## [1.0.0] - 2025-10-22

### Added
- **Block 10 restoration stage** - New intermediate restoration block for improved memory allocation
    - Splits restoration into three stages: Block 9 → Block 10 → Final restore code
    - Significantly improves success rate by reducing Block 9 size requirements
    - Makes it easier to allocate restoration code in fragmented memory
- **Verified VICE version support** - Tested and confirmed working with VICE 3.6, 3.7, 3.8, and 3.9
    - Most extensive testing performed with VICE 3.9
    - Older versions (3.6-3.8) confirmed functional but less tested

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
- Improved reliability across different VICE versions

### Technical Improvements
- Three-stage restoration architecture improves modularity
- Better separation of concerns in restoration process
- Reduced code complexity in individual restoration stages
- More predictable memory requirements

### Known Limitations
- Only supports VICE 3.6-3.9 x64sc snapshots (snapshot format changes between VICE versions)
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
- GUI application for converting VICE 3.6-3.9 x64sc snapshots to PRG files
- LZSA1 compression for efficient snapshot compression
- Automatic memory patching and restoration code generation
- MSI installer with WiX
- Smart vcruntime140.dll bundling (only if VC++ Redistributable not installed)
- Desktop and Start Menu shortcuts
- Complete documentation in README.md

### Known Limitations
- Only supports VICE 3.6-3.9 x64sc snapshots
- Requires memory initialization (`f 0000 ffff 00` + `reset`) before snapshot
- Stack pointer placement may be risky in edge cases
- "Smart attach..." feature in VICE should be avoided

[Unreleased]: https://github.com/tommyo123/Vice_Snapshot_to_PRG/compare/v1.0.0...HEAD
[1.0.0]: https://github.com/tommyo123/Vice_Snapshot_to_PRG/compare/v0.9.1...v1.0.0
[0.9.1]: https://github.com/tommyo123/Vice_Snapshot_to_PRG/compare/v0.9.0...v0.9.1
[0.9.0]: https://github.com/tommyo123/Vice_Snapshot_to_PRG/releases/tag/v0.9.0
