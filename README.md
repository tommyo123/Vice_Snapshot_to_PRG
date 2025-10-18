# VICE Snapshot to PRG Converter

A utility that converts VICE 3.9 x64sc emulator snapshots into self-restoring PRG files that can run on real Commodore 64 hardware.

## Overview

This tool takes a VICE snapshot (`.vsf` file) and transforms it into a standalone PRG file that, when loaded on a real C64, will restore the complete machine state—including all registers, memory, VIC-II graphics, SID audio, CIA timers, and the zero page—exactly as it was when the snapshot was taken.

The concept is inspired by the classic **Action Replay cartridge's "BACKUP" feature**, which allowed users to freeze a running program, compress the entire machine state, and save it to disk or tape. However, instead of requiring special cartridge hardware, this converter works with VICE emulator snapshots and produces files that run independently on any C64.

## Features

- **Complete machine state restoration**: CPU registers, memory, VIC-II, SID, CIA1, CIA2, color RAM, and zero page
- **Efficient compression**: Uses LZSA1 compression algorithm for fast decompression and good compression ratios
- **Self-contained PRG files**: No special cartridge or loader required—just load and run
- **Small decompression footprint**: Minimal memory overhead with intelligent free-space detection
- **GUI application**: Easy-to-use graphical interface
- **CLI tool**: Command-line version for automation and batch processing

## Download

**Latest release:** [Download from GitHub Releases](https://github.com/tommyo123/Vice_Snapshot_to_PRG/releases/latest)

### Windows
- **MSI Installer** (recommended): Installs both GUI and CLI with shortcuts
- **Portable ZIP**: Extract and run anywhere, no installation required

### Linux
- **tar.gz**: Pre-compiled binaries for Ubuntu 24.04+, Debian 12+, and compatible distributions

### macOS
- **tar.gz**: Pre-compiled binaries (untested, no access to Mac hardware)

### Security Warning on Download

**Windows and your browser may show security warnings for the MSI installer.**

This is normal for unsigned software from new publishers. The warnings occur because the installer is not code-signed (code signing certificates cost $75-400/year, which is not sustainable for a free hobby project).

**The file is safe.** You may encounter:

**1. Browser download warning:**
- Click "Keep" (or similar) to complete the download

**2. Windows SmartScreen warning when running the installer:**
- Click "More info"
- Click "Run anyway"

Alternatively, you can build from source to verify the code yourself.

## System Requirements

### Windows
**Tested on:**
- Windows 11 (64-bit)

**Expected to work on:**
- Windows 7, 8, 10 (64-bit)
- Visual C++ Redistributable 2015-2022 or bundled runtime

**Not supported:**
- 32-bit Windows
- Windows XP/Vista

### Linux
**Tested on:**
- Ubuntu 24.04

**Expected to work on:**
- Debian 12+
- Other modern distributions with compatible glibc

### macOS
**Untested:**
- macOS binaries are provided but not verified on actual hardware

## Important Limitations

### VICE Version Compatibility

**This tool is specifically designed for VICE 3.9 x64sc snapshots only.**

VICE's snapshot format changes frequently between versions. This converter has been developed and tested exclusively against snapshots created by `x64sc.exe` from VICE 3.9. There is **no guarantee** that it will work with snapshots from other VICE versions (earlier or later).

### Required Pre-Snapshot Preparation

Before taking a snapshot in VICE, you **must** execute the following commands in the VICE monitor to initialize memory:

```
f 0000 ffff 00
reset
```

**Why is this necessary?**  
The converter relies on finding large contiguous blocks of identical byte values in RAM to place its restoration code and data. Without this memory initialization, there may not be enough suitable free space, causing the conversion to fail.

### Avoid "Smart Attach" (Unless Configured)

**Do not use VICE's "Smart attach..." feature** when loading programs before taking a snapshot, unless VICE is configured to initialize memory to zeros on reset.

Smart attach can leave memory in a fragmented state without sufficient contiguous free blocks, which will likely cause the converter to fail with allocation errors. Instead:
- Use standard `LOAD "*",8,1` commands or manual file loading, or
- Configure VICE to initialize memory to zeros on reset (Settings → Settings → C64 → RAM reset pattern → All zeros), then you can use smart attach safely

### Stack Pointer Placement Considerations

The restoration code needs to place its final stage somewhere between `$0100` and `$01FF` (the 6502 stack area). The converter attempts to place this code **just below the current stack pointer** with a safety margin.

**In most cases, this works perfectly.** However, if there isn't enough space below the stack pointer, the converter will place the restoration code at the very top of the stack area (`$01FF` and below).

**This fallback placement is risky** because:
- If the original program had pushed the stack very high, the restoration code may be overwritten during the restore process
- This can cause crashes or unpredictable behavior

**Despite the risk**, this approach has been successfully tested with various programs, including some games where the stack pointer had been moved to unusual positions. The converter will always attempt the conversion, but be aware that success is not guaranteed in edge cases.

## Technical Details

### Compression Algorithm

This tool uses **LZSA1** (Lempel-Ziv-Style Algorithm), a modern byte-aligned compression format specifically engineered for fast decompression on 8-bit systems. LZSA1 was chosen because:

- **Fast decompression**: Approximately 90% of LZ4 speed on 6502, which is significantly faster than most alternatives
- **Good compression ratio**: Better than LZ4 while maintaining excellent decompression speed
- **Simple decompression code**: Small memory footprint, critical for fitting within C64 constraints
- **Far superior to Action Replay's RLE**: Much more efficient than the simple Run-Length Encoding used by Action Replay cartridges

The LZSA compression is powered by **Emmanuel Marty's LZSA algorithm**, integrated through a custom Rust wrapper library.

### Memory Layout Strategy

The converter scans the entire C64 memory space (`$0200-$FFEF`) looking for sequences of 32 or more consecutive identical bytes. It then allocates these free blocks to store:

1. **Blocks 1-8**: Preservation of the original stack area (`$0100-$01FF`) and critical zero page locations (`$F8-$FF`)
2. **Block 9**: Final restoration code that runs after RAM decompression
3. **Compressed data**: LZSA1-compressed segments for different memory regions

The restoration process works in stages:
1. BASIC stub at `$0801` executes SYS to `$080D` where main loader begins
2. Decompresses Color RAM, VIC-II, SID (while I/O is enabled)
3. Restores CIA1 and CIA2 registers directly
4. Decompresses zero page (`$02-$F7`)
5. Switches to RAM-only mode (`$01 = $34`)
6. Copies compressed main RAM data and relocated decompressor to top of memory, then copies relocated decompressor to `$0100`
7. Jumps to relocated decompressor which decompresses main RAM (`$0200-$FFEF`), including the preprogrammed final restore code in page 1
8. Block 9 restoration code executes:
    - Restores original page 1 (`$0100-$01FF`) from preserved blocks 1-8
    - Restores vectors (`$FFF0-$FFFF`)
    - Cleans up temporary blocks
    - Restores zero page locations (`$F8-$FF`)
    - Jumps to final restore code (now in restored page 1)
9. Final restore code executes:
    - Wipes block 9
    - Restores CPU port and stack pointer
    - Re-enables I/O (`$01 = $35`)
    - Configures VIC IRQ and CIA interrupts
    - Builds RTI frame on stack with original PC and status
    - Loads final A, X, Y registers and executes RTI to resume at original PC

### Assembly and Compression

The converter uses:
- **[asm6502](https://github.com/tommyo123/asm6502)**: An embedded Rust 6502 assembler library for generating restoration code
- **[lzsa-sys](https://github.com/tommyo123/lzsa-sys)**: A Rust wrapper around Emmanuel Marty's LZSA compression code

Both are integrated directly into the converter, eliminating the need for external tools.

## Installation

### Windows - Using the Installer (Recommended)

1. Download the latest `.msi` installer from the [releases page](https://github.com/tommyo123/Vice_Snapshot_to_PRG/releases/latest)
2. Run the installer and follow the on-screen instructions
3. The installer will:
    - Install to `Program Files\vice-snapshot-to-prg-converter\` (customizable)
    - Include both GUI and CLI versions
    - Create desktop and Start Menu shortcuts
    - Bundle Visual C++ runtime if not already installed

### Windows - Portable Installation

1. Download the latest `.zip` package from the [releases page](https://github.com/tommyo123/Vice_Snapshot_to_PRG/releases/latest)
2. Extract to a directory of your choice
3. Run either:
    - `vice-snapshot-to-prg-converter.exe` (GUI)
    - `vice-snapshot-to-prg-converter-cli.exe` (CLI)

### Linux / macOS

1. Download the appropriate `.tar.gz` from the [releases page](https://github.com/tommyo123/Vice_Snapshot_to_PRG/releases/latest)
2. Extract: `tar -xzf vice-snapshot-to-prg-converter-*.tar.gz`
3. Make executable: `chmod +x vice-snapshot-to-prg-converter*`
4. Run either:
    - `./vice-snapshot-to-prg-converter` (GUI)
    - `./vice-snapshot-to-prg-converter-cli input.vsf output.prg` (CLI)

The converter will automatically:
- Create temporary work directories in your system temp folder
- Clean up all temporary files after conversion

## Usage

### GUI Version

1. **Prepare your program in VICE 3.9 x64sc:**
   ```
   Enter monitor (Alt+H)
   f 0000 ffff 00
   reset
   x (exit monitor)
   ```

2. **Load your program normally** (avoid "Smart attach..." unless configured for zero memory)

3. **Take a snapshot:**
    - File → Save snapshot image
    - Save as a `.vsf` file

4. **Run the converter:**
    - Launch the GUI application
    - Browse to select your `.vsf` snapshot file
    - Choose output location for the `.prg` file
    - Click "Convert"

5. **Transfer to real C64:**
    - Transfer the resulting PRG file to your C64 via disk, SD card reader, or other means
    - `LOAD "yourfile.prg",8,1` (or device 8)
    - `RUN`

### CLI Version

The command-line version is perfect for automation and batch processing:

```bash
# Basic usage
vice-snapshot-to-prg-converter-cli input.vsf output.prg

# Show help
vice-snapshot-to-prg-converter-cli --help
```

**Note:** The CLI version automatically overwrites output files without prompting.

**Windows example:**
```cmd
cd "C:\Program Files\vice-snapshot-to-prg-converter"
vice-snapshot-to-prg-converter-cli.exe snapshot.vsf output.prg
```

**Linux/macOS example:**
```bash
./vice-snapshot-to-prg-converter-cli snapshot.vsf output.prg
```

The program will restore the complete machine state and resume execution exactly where the snapshot was taken.

## Project Status and Philosophy

This is a **hobby project** and **proof-of-concept** developed for fun and educational purposes. The primary goals were:

1. To explore whether a VICE-to-PRG converter was technically feasible
2. To implement a solution inspired by the Action Replay cartridge concept
3. To experiment with modern compression techniques on vintage hardware

**No guarantees or warranties are provided.** This tool works with VICE 3.9 snapshots, but there is no commitment to support future VICE versions. The snapshot format changes frequently, and maintaining compatibility would require ongoing maintenance that may not be sustainable as a hobby project.

### Open Source

The complete source code is freely available under a public domain dedication. If you find this tool useful or interesting, you are encouraged to:

- Study the code to understand the restoration techniques
- Modify it for your own purposes
- Update it to work with newer VICE versions
- Create derivative works

The code is released openly in the spirit of the retro computing community, where knowledge sharing and experimentation are valued over commercial protection.

## Building from Source

Requirements:
- Rust toolchain (edition 2024 or later)
- Platform-specific dependencies:
    - **Windows**: Visual Studio build tools or MinGW
    - **Linux**: X11, Cairo, Pango, and FLTK dependencies
    - **macOS**: Xcode command-line tools

Build commands:
```bash
# Build both GUI and CLI (release)
cargo build --release

# Build only GUI
cargo build --release --bin vice-snapshot-to-prg-converter

# Build only CLI
cargo build --release --bin vice-snapshot-to-prg-converter-cli

# Debug build
cargo build
```

The release build creates optimized binaries in `target/release/`.

## Known Issues and Troubleshooting

### "Failed to allocate block X"
Your snapshot likely has insufficient contiguous free memory. Make sure you ran `f 0000 ffff 00` and `reset` before loading the program and taking the snapshot.

### "Stack too low" error
The program's stack pointer is in an unusual position. The converter may still attempt to place code at `$01FF`, but success is not guaranteed.

### Crashes on restore
This can happen if:
- The original program uses unusual memory configurations
- The stack pointer was positioned in a way that conflicts with restoration code
- The program modifies memory in ways not captured by the snapshot

### VICE version mismatch
If you get parsing errors or corrupted output, verify you're using **VICE 3.9 x64sc** for both creating the snapshot and (if testing in emulator) loading the resulting PRG.

### Linux: Missing library errors
If you get "error while loading shared libraries", install the required dependencies:
```bash
# Ubuntu/Debian
sudo apt-get install libx11-6 libxext6 libxft2 libxinerama1 libcairo2 libpango-1.0-0

# The pre-built binaries are compiled on Ubuntu 24.04
# For older distributions, build from source
```

## Credits

**Converter Development**: Tommy Olsen

**Compression Algorithm**:
- **LZSA**: Emmanuel Marty - Fast compression specifically designed for 8-bit systems

**Inspiration**:
- Action Replay cartridge series by Datel Electronics
- The VICE development team for the excellent C64 emulator

## License

This software is released under the CC0 1.0 Universal Public Domain Dedication. You are free to use, copy, modify, and distribute it for any purpose without restrictions or attribution. See https://creativecommons.org/publicdomain/zero/1.0/ for details.
