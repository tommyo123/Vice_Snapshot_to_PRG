# VICE Snapshot → PRG / CRT Converter

Converts VICE snapshots into self-restoring PRG files, EasyFlash CRT or Magic Desk CRT cartridges that boot directly on a real Commodore 64.

The converter reconstructs the full machine state: CPU registers, RAM, Color RAM, VIC-II, SID, CIA1/CIA2, stack pointer, zero-page, vectors, I/O mode – everything needed to return to the exact snapshot moment.

## Status & License

- **Version:** 2.2.0
- **License:** MIT

## What it does

- Reads VICE snapshot files (VSF).
- Restores the machine state faithfully on real hardware.
- Produces:
  - Self-extracting **PRG**, or
  - **EasyFlash CRT** with optional LOAD-intercept for embedded PRG files, or
  - **Magic Desk CRT** (8K cart mode, ROML only) with the same optional LOAD-intercept.

Not every snapshot converts cleanly — some programs rely on hardware state that can't be faithfully reproduced from the file alone.

## Downloads

See [Releases](https://github.com/tommyo123/Vice_Snapshot_to_PRG/releases) for prebuilt binaries.

Available as:
- **Windows:** MSI installer + portable ZIP
- **Linux/macOS:** tar.gz archives

(Executables are unsigned; Windows will show a warning.)

## Requirements and limitations

### Clear RAM before taking the snapshot

To ensure good compression and reliable free-area detection, RAM should be filled with a single byte before loading your program.

In the VICE monitor:
```
f 0000 ffff 00
reset
```

This produces large uniform regions that the converter can use for restore code and compressed blocks. Without this, memory becomes fragmented and the converter may fail to allocate space.

### Automatic power-on pattern clearing

A freshly powered C64 (and VICE's default / Smart Attach RAM init) does not come up
all-zero — it comes up in a fixed pattern of `$00`/`$FF` bytes in short (4-byte) runs.
Those runs are too short for the free-area scan, so untouched RAM looks "used".

The converter now detects this power-on pattern and automatically zeroes the regions
that still hold it (an exact, conservative match — only memory the program never wrote
is cleared). This recovers that RAM as free space without a manual fill, so snapshots
taken **without** the `f 0000 ffff 00` step convert far more reliably.

Manual clearing is still the most thorough option (it also flattens program-touched
scratch areas), but is no longer required for the common case.

### About Smart Attach

Smart Attach uses VICE's realistic C64-style memory initialization, not a uniform fill.
The automatic power-on pattern clearing (above) now handles the bulk of this, so Smart
Attach snapshots usually convert without a manual fill. If a particular snapshot still
fails to allocate, clear RAM manually (`f 0000 ffff 00`) and retry.

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
- Ultimax mode: ROML + ROMH.
- Restore code and compressed data live in ROM.
- Can embed PRG files and intercept `LOAD "NAME",8,1`.
- Automatically picks trampoline address (`$0100` or `$0334`) based on stack position.

**ROM layout:**
- **ROML** (`$8000–$9FFF`): Restore code, decompressor, compressed blocks
- **ROMH** (`$A000–$BFFF`): Startup vectors, LOAD/SAVE hook, file metadata

### Magic Desk CRT

- Boots directly from cartridge via CBM80 signature.
- 8K cart mode: ROML only (`$8000–$9FFF`), no ROMH.
- Banked out at runtime via `$DE00` bit 7. This is reversible (it only drives
  EXROM), so the cartridge can be banked back in — which is what makes the LOAD
  hook possible.
- Minimum 8 banks, maximum 64 banks (512 KB).
- Can embed PRG files and intercept `LOAD "NAME",8,1`, identical to EasyFlash.

**ROM layout (no embedded files):**
- **Bank 0 ROML**: Boot code (CBM80) + payload start
- **Banks 0–N ROML**: Restore code + relocated decompressor + compressed RAM

**ROM layout (with embedded files):**
- **Bank 0 ROML** (directory): Boot code (`$8000`), LOAD handler (`$8400`),
  file metadata (`$9000`), filenames (`$9800`)
- **Banks 1–N ROML**: Restore code + relocated decompressor + compressed RAM
- **Banks N+1… ROML**: Embedded PRG file data

Magic Desk has no ROMH, so (unlike EasyFlash, which keeps the handler/metadata in
ROMH at `$A000–$BFFF`) the directory lives in bank 0. Only a small trampoline sits
in C64 RAM, in the cassette buffer (`$0334`); during a LOAD it banks the directory
bank in, copies the requested file straight from ROM, and banks the cartridge back
out.

## Usage

### CLI

```bash
# PRG
vice-snapshot-to-prg-converter-cli input.vsf output.prg

# EasyFlash CRT
vice-snapshot-to-prg-converter-cli input.vsf output.crt

# EasyFlash CRT with custom name and embedded PRGs
vice-snapshot-to-prg-converter-cli --crt --name "My Game" --include-dir ./prg input.vsf output.crt

# EasyFlash CRT with custom hook address
vice-snapshot-to-prg-converter-cli --crt --include-dir ./prg --hook-addr $0334 input.vsf output.crt

# Magic Desk CRT
vice-snapshot-to-prg-converter-cli --magic-desk --name "My Game" input.vsf output.crt

# Magic Desk CRT with embedded PRGs
vice-snapshot-to-prg-converter-cli --magic-desk --include-dir ./prg input.vsf output.crt

# EasyFlash SAVE: persistent flash filesystem
#   ./ro       = read-only files (never change)
#   ./defaults = rewritable files seeded with defaults (e.g. a high-score table)
vice-snapshot-to-prg-converter-cli --ef-save --include-dir ./ro --rw-dir ./defaults input.vsf output.crt
```

**Options:**
- `--prg` / `--crt` / `--magic-desk` / `--ef-save` – Force format (optional, auto-detected from extension for PRG/CRT)
- `--name <name>` – Cartridge name (max 32 chars, CRT only)
- `--include-dir <dir>` – Embed PRG files from directory (EasyFlash, Magic Desk, or read-only area of `--ef-save`)
- `--rw-dir <dir>` – Seed the rewritable area with default files (`--ef-save` only)
- `--trampoline <hexaddr>` – Force the LOAD/SAVE trampoline location (`--ef-save` only; default: auto-placed in free RAM)
- `--eapi-buffer <auto|screen|hexaddr>` – Flash-driver buffer placement (`--ef-save` only; default: auto, falls back to screen RAM)
- `--hook-addr <hex>` – Override LOAD/SAVE hook address (EasyFlash only; Magic Desk uses a fixed trampoline)

Output files are overwritten without prompting.

### EasyFlash SAVE (`--ef-save`)

Produces an EasyFlash cartridge that restores the snapshot **and** gives the program a
read/write flash filesystem (drunella's [libefs](https://github.com/Drunella/libefs)). The
KERNAL vectors for file access and channel I/O (`LOAD`, `SAVE`, `OPEN`, `CLOSE`, `CHKIN`, `CKOUT`, `CLRCHN`, `CHRIN`, `CHROUT`) are hooked, allowing transparent sequential file access (reading and writing character-by-character) and standard operations with no program changes.

- **Read-only area** (`--include-dir`) – files that never change (up to roughly a full D81's worth).
- **Rewritable area** (`--rw-dir`) – seeded with default files (e.g. a starting high-score table);
  `LOAD` searches both areas.
- A plain `SAVE"NAME",8` over an existing file **overwrites** it (auto-promoted to the CBM `@0:`
  replace command): the old entry is invalidated and the new data appended to free flash, so
  high-scores and save-games rotate in place. A program that supplies its own `@...` command is
  left untouched.
- **Garbage collection is automatic**: the rewritable area is two ping-pong halves; when one fills
  with live + invalidated files, libefs copies the live files to the other half and erases the full
  sector during a `SAVE`. Saving keeps working indefinitely with no data loss.
- **C64 RAM use**: the running engine needs a little free RAM — ~300 bytes (below `$8000`) for the
  LOAD/SAVE trampoline plus a page-aligned ~1 KB buffer in `$0000-$0FFF` for the flash driver (the
  AM29F040 EAPI must run from RAM, reachable in Ultimax mode, during a write). Both are placed
  automatically in free RAM.
  - `--trampoline <hexaddr>` forces the trampoline location if a game leaves no usable gap where it
    lands by default (`tape`/`stack` are accepted names but are too small for the ~300-byte trampoline).
  - `--eapi-buffer <auto|screen|hexaddr>` controls the flash buffer. Default `auto` uses free RAM in
    `$0900-$0FFF` and, if a game leaves none, **falls back to the screen RAM** — only clobbered during
    the LOAD/SAVE and redrawn by the program afterward, so even a RAM-full game can save. `screen`
    forces it; a hex address must be page-aligned in `$0400-$0C00` (VIC bank 0).
- Run VICE with **`-easyflashcrtwrite`** to persist flash changes back to the `.crt` on a clean exit.

### EasyFlash SAVE Directory Viewer

A standalone Python utility is included to parse and view the directory structure of an EasyFlash persistent save cartridge (`.crt`), including deleted and overwritten files:

```bash
python show_save_dir.py <path_to_crt> [--show-ro]
```

This tool:
* Inspects bank 0 HIROM to automatically extract the cartridge name and `libefs` storage configuration.
* Detects whether directories reside in LOROM or HIROM dynamically based on the configuration's `dir_high` value, supporting both the old alternating format and HIROM-only layout.
* Decodes PETSCII filenames into readable ASCII.
* Deduces the status of each file slot: **Active**, **Overwritten** (superseded by a later entry of the same name), or **Deleted** (explicitly scratched or deleted by a later version).

### GUI

The GUI provides the same functionality with file browsers and a CRT options tab. Select cartridge type — **EasyFlash**, **Magic Desk**, or **EasyFlash SAVE** — from the dropdown. LOAD/SAVE hooking with an include directory works for all three; the custom hook-address controls apply to EasyFlash (Magic Desk uses a fixed trampoline). Choosing **EasyFlash SAVE** reveals its extra controls — a rewritable-defaults directory and the flash-buffer placement (Auto / Screen RAM / Custom address) — mirroring the CLI's `--rw-dir`, `--trampoline`, and `--eapi-buffer`. If conversion fails, a dialog offers to add manual RAM blocks.

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
