# VICE Snapshot to PRG Converter

A utility that converts VICE 3.6-3.9 x64sc emulator snapshots into self-restoring PRG files that run on real Commodore 64 hardware.

## Overview

This tool takes a VICE snapshot (`.vsf` file) and transforms it into a standalone PRG file that restores the complete machine state on a real C64—including CPU registers, memory, VIC-II graphics, SID audio, CIA timers, color RAM, and zero page—exactly as it was when the snapshot was taken.

**Inspired by the Action Replay cartridge's "BACKUP" feature**, but works with VICE emulator snapshots and produces files that run independently on any C64 without special hardware.

## Features

- Complete machine state restoration (CPU, RAM, VIC-II, SID, CIA1, CIA2, color RAM, zero page)
- LZSA1 compression for fast decompression on 6502
- Self-contained PRG files—no cartridge or loader required
- Minimal memory overhead with intelligent free-space detection
- GUI and CLI versions included

## Download

**Latest release:** [Download from GitHub Releases](https://github.com/tommyo123/Vice_Snapshot_to_PRG/releases/latest)

### Windows
- **MSI Installer** (recommended): Installs both GUI and CLI with shortcuts
- **Portable ZIP**: Extract and run anywhere

### Linux
- **tar.gz**: Pre-compiled binaries for Ubuntu 24.04+, Debian 12+, and compatible distributions

### macOS
- **tar.gz**: Pre-compiled binaries (untested)

### Security Warning

Windows may show security warnings because the installer is not code-signed (code signing certificates cost hundreds of dollars per year, which is not sustainable for a free hobby project).

**The file is safe.** To run:

1. **Browser download warning:** Click "Keep"
2. **Windows SmartScreen:** Click "More info" → "Run anyway"

Alternatively, build from source to verify the code yourself.

## System Requirements

### Windows
- **Tested:** Windows 11 (64-bit)
- **Expected to work:** Windows 8, 10 (64-bit)
- **Extended/Unofficial Support:** Windows 7 (Requires VxKex for API compatibility, https://github.com/i486/VxKex)
- **Not supported:** 32-bit Windows, Windows XP/Vista

Requires Visual C++ Redistributable 2015-2022 or bundled runtime.

### Linux
- **Tested:** Ubuntu 24.04
- **Expected to work:** Debian 12+, other modern distributions with compatible glibc

### macOS
- **Untested:** Binaries provided but not verified

## Important Limitations

### VICE Version Compatibility

**Only works with VICE 3.6-3.9 x64sc snapshots.**

VICE's snapshot format changes between versions. This converter has been developed and tested against VICE 3.6-3.9 x64sc snapshots, with most testing done on VICE 3.9. No guarantee it will work with other versions.

### Required Pre-Snapshot Preparation

Before taking a snapshot in VICE, initialize memory:

```
f 0000 ffff 00
reset
```

**Why?** The converter needs large contiguous blocks of identical bytes in RAM to place restoration code and data. Without initialization, conversion may fail with allocation errors.

### Avoid "Smart Attach" (Unless Configured)

**Do not use "Smart attach..."** when loading programs before snapshots, unless VICE is configured to initialize memory to zeros on reset.

Smart attach can fragment memory, causing allocation failures. Instead:
- Use standard `LOAD "*",8,1` commands, or
- Configure VICE: Settings → C64 → RAM reset pattern → All zeros

### Stack Pointer Considerations

The restoration code places its final stage between `$0100-$01FF` (the 6502 stack area), ideally just below the current stack pointer with a safety margin.

If insufficient space exists below the stack pointer, the code is placed at the top of the stack area (`$01FF` and below). This is risky if the original program had pushed the stack very high, as restoration code may be overwritten.

Despite this risk, the approach has been successfully tested with various programs. The converter will always attempt conversion, but success is not guaranteed in all edge cases.

## Quick Start

### GUI Version

1. **Prepare VICE 3.6-3.9 x64sc:**
   ```
   Alt+H (enter monitor)
   f 0000 ffff 00
   reset
   x (exit monitor)
   ```

2. **Load your program** (avoid "Smart attach..." unless configured)

3. **Take snapshot:** File → Save snapshot image (.vsf)

4. **Run converter:**
    - Launch GUI application
    - Select `.vsf` input file
    - Choose `.prg` output file
    - Click "Convert"

5. **Transfer to C64:**
    - Transfer `.prg` to C64 (disk, SD card, etc.)
    - `LOAD "yourfile.prg",8,1`
    - `RUN`

### CLI Version

Perfect for automation and batch processing:

```bash
vice-snapshot-to-prg-converter-cli input.vsf output.prg
```

**Note:** CLI version automatically overwrites output files without prompting.

## Technical Details

### Compression Algorithm

Uses **LZSA1** (Lempel-Ziv-Style Algorithm) by Emmanuel Marty, specifically engineered for fast decompression on 8-bit systems:

- **Fast decompression:** ~90% of LZ4 speed on 6502
- **Good compression ratio:** Better than LZ4 while maintaining excellent speed
- **Small decompression code:** Minimal memory footprint
- **Far superior to Action Replay's RLE:** Much more efficient than simple Run-Length Encoding

### Memory Layout Strategy

The converter scans `$0200-$FFEF` for sequences of 32+ consecutive identical bytes, allocating these free blocks for:

- **Blocks 1-8:** Preservation of stack area (`$0100-$01FF`) and critical zero page (`$F8-$FF`)
- **Block 9:** Core restoration code (restores blocks 1-8, cleans up blocks 1-8, jumps to block 10)
- **Block 10:** Final setup code (wipes block 9, restores `$F8-$FF`, prepares registers, jumps to `$01xx`)
- **Compressed data:** LZSA1-compressed segments for different memory regions

### Restoration Process

1. BASIC stub at `$0801` executes SYS to `$080D`
2. Decompress Color RAM, VIC-II, SID (while I/O enabled)
3. Setup VIC raster position and clear interrupts
4. Restore CIA1 and CIA2 registers completely (without starting timers)
5. Decompress zero page (`$02-$F7`)
6. Switch to RAM-only mode (`$01 = $34`)
7. Copy compressed main RAM data and relocated decompressor to top of memory
8. Copy relocated decompressor to `$0100-$01FF`
9. Jump to relocated decompressor which decompresses main RAM (`$0200-$FFEF`)
10. **Block 9** executes:
    - Restores original page 1 (`$0100-$01FF`) from blocks 1-8
    - Restores vectors (`$FFF0-$FFFF`)
    - Restores stack pointer
    - Cleans up blocks 1-8
    - Jumps to block 10
11. **Block 10** executes:
    - Wipes block 9
    - Restores zero page (`$F8-$FF`)
    - Preloads A, X, Y registers
    - Jumps to final restore code (now in restored `$01xx`)
12. **Final restore code** executes:
    - Wipes block 10
    - Restores CPU port DDR (`$00`)
    - Restores CPU port data (`$01 = $35`)
    - Configures VIC IRQ and CIA interrupts (without starting timers)
    - Starts CIA timers with original control register values
    - Builds RTI frame on stack with original PC and status
    - Executes RTI to resume at original PC

### Assembly and Compression

The converter uses:
- **[asm6502](https://github.com/tommyo123/asm6502):** Embedded Rust 6502 assembler for generating restoration code
- **[lzsa-sys](https://github.com/tommyo123/lzsa-sys):** Rust wrapper around Emmanuel Marty's LZSA compression

Both are integrated directly, eliminating the need for external tools.

## Installation

### Windows - Using the Installer (Recommended)

1. Download `.msi` installer from [releases page](https://github.com/tommyo123/Vice_Snapshot_to_PRG/releases/latest)
2. Run installer and follow instructions
3. Installs to `Program Files\vice-snapshot-to-prg-converter\` (customizable)
4. Includes both GUI and CLI versions
5. Creates desktop and Start Menu shortcuts

### Windows - Portable Installation

1. Download `.zip` package from [releases page](https://github.com/tommyo123/Vice_Snapshot_to_PRG/releases/latest)
2. Extract to any directory
3. Run:
    - `vice-snapshot-to-prg-converter.exe` (GUI)
    - `vice-snapshot-to-prg-converter-cli.exe` (CLI)

### Linux / macOS

1. Download `.tar.gz` from [releases page](https://github.com/tommyo123/Vice_Snapshot_to_PRG/releases/latest)
2. Extract: `tar -xzf vice-snapshot-to-prg-converter-*.tar.gz`
3. Make executable: `chmod +x vice-snapshot-to-prg-converter*`
4. Run:
    - `./vice-snapshot-to-prg-converter` (GUI)
    - `./vice-snapshot-to-prg-converter-cli input.vsf output.prg` (CLI)

## CLI Usage

```bash
# Basic usage
vice-snapshot-to-prg-converter-cli input.vsf output.prg

# Show help
vice-snapshot-to-prg-converter-cli --help
```

**Windows:**
```cmd
cd "C:\Program Files\vice-snapshot-to-prg-converter"
vice-snapshot-to-prg-converter-cli.exe snapshot.vsf output.prg
```

**Linux/macOS:**
```bash
./vice-snapshot-to-prg-converter-cli snapshot.vsf output.prg
```

## Building from Source

Requirements:
- Rust toolchain (2024 edition or later)
- Platform-specific dependencies:
    - **Windows:** Visual Studio build tools or MinGW
    - **Linux:** X11, Cairo, Pango, FLTK dependencies
    - **macOS:** Xcode command-line tools

```bash
# Build both GUI and CLI (release)
cargo build --release

# Build only GUI
cargo build --release --bin vice-snapshot-to-prg-converter

# Build only CLI
cargo build --release --bin vice-snapshot-to-prg-converter-cli
```

Binaries created in `target/release/`.

## Troubleshooting

### "Failed to allocate block X"
Insufficient contiguous free memory. Run `f 0000 ffff 00` and `reset` in VICE monitor before loading the program and taking the snapshot.

### "Stack too low" error
The program's stack pointer is in an unusual position. The converter may still attempt placement at `$01FF`, but success is not guaranteed.

### Crashes on restore
Can happen if:
- The original program uses unusual memory configurations
- Stack pointer positioning conflicts with restoration code
- The program modifies memory in ways not captured by the snapshot

### VICE version mismatch
If you get parsing errors or corrupted output, verify you're using **VICE 3.6-3.9 x64sc** for creating the snapshot.

### Linux: Missing library errors
Install required dependencies:
```bash
# Ubuntu/Debian
sudo apt-get install libx11-6 libxext6 libxft2 libxinerama1 libcairo2 libpango-1.0-0
```

Pre-built binaries are compiled on Ubuntu 24.04. For older distributions, build from source.

## Project Status

This is a **hobby project** developed for fun and educational purposes. Primary goals:

1. Explore whether a VICE-to-PRG converter was technically feasible
2. Implement a solution inspired by Action Replay cartridge
3. Experiment with modern compression techniques on vintage hardware

**No guarantees or warranties provided.** The tool works with VICE 3.6-3.9 snapshots (most testing done on 3.9). No commitment to support future VICE versions, as the snapshot format changes frequently.

## License

MIT License - Copyright (c) 2025 Tommy Olsen

See LICENSE.md for full license text.

You are free to use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies of the software.

## Credits

**Development:** Tommy Olsen

**Compression Algorithm:**
- LZSA by Emmanuel Marty - Fast compression for 8-bit systems

**Inspiration:**
- Action Replay cartridge series by Datel Electronics
- VICE development team

## Version History

**Version 1.0** - Initial release
- Complete machine state restoration
- LZSA1 compression
- Optimized two-block architecture (Block 9 + Block 10)
- GUI and CLI versions
- Windows MSI installer
- Linux and macOS support
