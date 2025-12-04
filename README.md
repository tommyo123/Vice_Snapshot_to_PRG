# VICE Snapshot → PRG / EasyFlash CRT Converter

Converts VICE x64sc (C64SC) snapshots into self-restoring PRG files or EasyFlash CRT cartridges that boot directly on a real Commodore 64.

The converter reconstructs the full machine state: CPU registers, RAM, Color RAM, VIC-II, SID, CIA1/CIA2, stack pointer, zero-page, vectors, I/O mode – everything needed to return to the exact snapshot moment.

## Status & License

- **Version:** 1.90-Beta
- **License:** MIT

## What it does

- Reads VICE snapshot format 2.0 taken with x64sc (C64SC).
- Restores the machine state faithfully on real hardware.
- Produces:
  - Self-extracting **PRG**, or
  - **EasyFlash CRT** with optional LOAD-intercept for embedded PRG files.

Snapshot parsing is intentionally strict: anything other than format 2.0 / C64SC is rejected to avoid undefined results.

Tested with VICE 3.6–3.9.

## Downloads

See [Releases](https://github.com/tommyo123/Vice_Snapshot_to_PRG/releases) for prebuilt binaries.

Available as:
- **Windows:** MSI installer + portable ZIP
- **Linux/macOS:** tar.gz archives

(Executables are unsigned; Windows will show a warning.)

## Requirements and limitations

### Snapshot requirements

- Must be from VICE **x64sc** (the C64SC model).
- Must be snapshot **format 2.0**.
- Other formats and models (x64, xscpu64, etc.) are intentionally unsupported.

### Clear RAM before taking the snapshot

To ensure good compression and reliable free-area detection, RAM should be filled with a single byte before loading your program.

In the VICE monitor:
```
f 0000 ffff 00
reset
```

This produces large uniform regions that the converter can use for restore code and compressed blocks. Without this, memory becomes fragmented and the converter may fail to allocate space.

### About Smart Attach

Smart Attach uses VICE's realistic C64-style memory initialization, not a uniform fill. This results in a patchy, patterned RAM layout that:

- dramatically reduces compressible regions,
- reduces compression ratios,
- increases the chance of restore-block allocation failures.

You can use Smart Attach, but only if you manually clear RAM first.

### Stack considerations

If the original program leaves the stack unusually low, the converter automatically switches to an alternative restore trampoline. This works for both PRG and CRT output.

### Manual RAM blocks

If conversion fails due to insufficient free memory, the GUI offers to add RAM blocks manually. Specify an address range (e.g., `$0800` to `$08FF`) for memory you know is unused. The region will be zeroed and made available for allocation.

## Output formats

### PRG

- Self-restoring executable.
- Uses LZSA1-compressed segments (RAM, VIC, Color RAM).
- Small, efficient restore stub.
- Returns to the snapshot PC/flags exactly.

### EasyFlash CRT

- Boots directly from cartridge.
- Restore code and compressed data live in ROM.
- Can embed PRG files and intercept `LOAD "NAME",8,1`.
- Automatically picks trampoline address (`$0100` or `$0334`) based on stack position.

**ROM layout:**
- **ROML** (`$8000–$9FFF`): Restore code, decompressor, compressed blocks
- **ROMH** (`$A000–$BFFF`): Startup vectors, LOAD/SAVE hook, file metadata

## Usage

### CLI

```bash
# PRG
vice-snapshot-to-prg-converter-cli input.vsf output.prg

# CRT
vice-snapshot-to-prg-converter-cli input.vsf output.crt

# CRT with custom name and embedded PRGs
vice-snapshot-to-prg-converter-cli --crt --name "My Game" --include-dir ./prg input.vsf output.crt
```

**Options:**
- `--prg` / `--crt` – Force format (optional, auto-detected from extension)
- `--name <name>` – Cartridge name (max 32 chars, CRT only)
- `--include-dir <dir>` – Embed PRG files from directory (CRT only)

Output files are overwritten without prompting.

### GUI

The GUI provides the same functionality with file browsers and a CRT options tab. If conversion fails, a dialog offers to add manual RAM blocks.

### Recommended workflow

1. In VICE monitor (`Alt+H`):
   ```
   f 0000 ffff 00
   reset
   x
   ```
2. Load your program (avoid Smart Attach unless RAM was cleared).
3. Create a `.vsf` snapshot.
4. Run the converter.
5. Transfer and run the resulting PRG, or flash the CRT.

## Restore engine

1. BASIC stub transfers control to the restore loader.
2. Restores Color RAM, VIC-II and SID registers.
3. Restores CIA state without triggering timers prematurely.
4. Restores zero page and switches I/O mode.
5. Decompresses LZSA blocks into RAM.
6. Restores page 1, stack and system vectors.
7. Executes RTI back to the snapshot's PC and flags.

Compression uses LZSA1, which approaches LZ4-level decoding speed on 6502 while keeping the decompressor compact.

## Troubleshooting

**"Failed to allocate block …"**
RAM was not uniform. Clear RAM with `f 0000 ffff 00` and retry. Alternatively, use the GUI to add manual RAM blocks.

**Restore boots but crashes**
The snapshot was taken with fragmented memory or odd stack state. Clear RAM, avoid Smart Attach, reload and try again.

**CRT LOAD-hook doesn't find files**
Check that filenames (in `--include-dir`) are PETSCII-safe and 16 chars or fewer.

## Building from source

Requires the Rust toolchain (2024 edition).

```bash
# CLI only
cargo build --release --bin vice-snapshot-to-prg-converter-cli

# GUI + CLI
cargo build --release
```

## Credits

- Emmanuel Marty – LZSA
- The VICE team
- Various freezer cartridges for historical inspiration
