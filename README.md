# VICE Snapshot to PRG / CRT Converter

Converts VICE snapshots into self-restoring PRG files, EasyFlash CRT or Magic Desk CRT cartridges that boot directly on a real Commodore 64.

The converter reconstructs the full machine state: CPU registers, RAM, Color RAM, VIC-II, SID, CIA1/CIA2, stack pointer, zero-page, vectors, I/O mode. Everything needed to return to the exact snapshot moment.

## Status & License

- **Version:** 2.3.0
- **License:** MIT

## What it does

- Reads VICE snapshot files (VSF).
- Restores the machine state faithfully on real hardware.
- Produces:
  - Self-extracting **PRG**, or
  - **EasyFlash CRT** with optional LOAD-intercept for embedded PRG files, or
  - **Magic Desk CRT** (8K cart mode, ROML only) with the same optional LOAD-intercept.

Not every snapshot converts cleanly. Some programs rely on hardware state that can't be faithfully reproduced from the file alone.

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

### Power-on pattern clearing (experimental, off by default)

A freshly powered C64 (and VICE's default / Smart Attach RAM init) does not come up
all-zero. It comes up in a fixed pattern of `$00`/`$FF` bytes in short (4-byte) runs.
Those runs are too short for the free-area scan, so untouched RAM looks "used".

The converter can optionally detect this power-on pattern and zero the regions that
still hold it (a strict, byte-exact match over maximal spans of 64+ bytes; program-written
bytes are never touched, they only split a span). This recovers that RAM as free space
without a manual fill, so snapshots taken without the `f 0000 ffff 00` step may convert
more reliably. The number of cleared bytes is reported after each conversion.

**This is highly experimental and off by default.** A misdetected span would zero real
program data. Enable it via the GUI "Clear power-on RAM pattern" checkbox (which asks
for confirmation) or the CLI `--clear-poweron-ram` flag, only if you understand the risk.

The manual `f 0000 ffff 00` step remains the reliable way to prepare a snapshot.

### About Smart Attach

Smart Attach uses VICE's realistic C64-style memory initialization, not a uniform fill,
so snapshots taken with it are more likely to need the manual `f 0000 ffff 00` step (or
the experimental power-on pattern clearing above) before they convert reliably.

### Stack considerations

If the original program leaves the stack unusually low, the converter automatically switches to an alternative restore trampoline. This works for both PRG and CRT output.

### Manual RAM blocks

If conversion fails due to insufficient free memory, the GUI offers to add RAM blocks manually. Specify an address range (e.g., `$0800` to `$08FF`) for memory you know is unused. The region will be zeroed and made available for allocation.

## Output formats

### PRG

- Self-restoring executable.
- Packed segments (RAM, VIC, Color RAM, zero page) in the selected format.
- Small restore stub.
- Returns to the snapshot PC/flags exactly.

### EasyFlash CRT

- Boots directly from cartridge.
- Ultimax mode: ROML + ROMH.
- Restore code and compressed data live in ROM.
- Can embed PRG files and intercept `LOAD "NAME",8,1`.
- Automatically picks trampoline address (`$0100` or `$0334`) based on stack position.

**ROM layout:**
- **ROML** (`$8000-$9FFF`): Restore code, decompressor, compressed blocks
- **ROMH** (`$A000-$BFFF`): Startup vectors, LOAD/SAVE hook, file metadata

### Magic Desk CRT

- Boots directly from cartridge via CBM80 signature.
- 8K cart mode: ROML only (`$8000-$9FFF`), no ROMH.
- Banked out at runtime via `$DE00` bit 7. This is reversible (it only drives
  EXROM), so the cartridge can be banked back in, which is what the LOAD
  hook needs.
- Minimum 8 banks, maximum 64 banks (512 KB).
- Can embed PRG files and intercept `LOAD "NAME",8,1`, identical to EasyFlash.
- Automatically picks trampoline address (`$0100` or `$0334`) based on stack position.

**ROM layout (no embedded files):**
- **Bank 0 ROML**: Boot code (CBM80) + payload start
- **Banks 0-N ROML**: Restore code + relocated decompressor + compressed RAM

**ROM layout (with embedded files):**
- **Bank 0 ROML** (directory): Boot code (`$8000`), LOAD handler (`$8400`),
  file metadata (`$9000`), filenames (`$9800`)
- **Banks 1-N ROML**: Restore code + relocated decompressor + compressed RAM
- **Banks N+1 and up ROML**: Embedded PRG file data

Magic Desk has no ROMH, so (unlike EasyFlash, which keeps the handler/metadata in
ROMH at `$A000-$BFFF`) the directory lives in bank 0. Only a small trampoline sits
in C64 RAM; its address is picked with the same mechanism as EasyFlash (`$0100` or
the cassette buffer `$0334`, based on the snapshot's stack pointer, or a manual
`--hook-addr`). During a LOAD it banks the directory bank in, copies the requested
file straight from ROM, and banks the cartridge back out.

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
```

**Options:**
- `--prg` / `--crt` / `--magic-desk`: force format (optional, auto-detected from extension for PRG/CRT)
- `--name <name>`: cartridge name (max 32 chars, CRT only)
- `--include-dir <dir>`: embed PRG files from directory (EasyFlash or Magic Desk)
- `--hook-addr <hex>`: override LOAD/SAVE hook address (EasyFlash or Magic Desk; overrides the automatic `$0100`/`$0334` placement)
- `--format <fmt>`: compression format, one of `lzsa1`, `lzsa2`, `zx0`, `zx02`, `lzan-min`, `bolt`, `bb2` (default `lzsa1`, see Compression below)
- `--vsf`: force VSF snapshot input instead of auto-detecting a cartridge freeze
- `--freezer <type>`: convert a cartridge freeze; type is `auto`, `ar`, `isepic` or `fc3`
- `--clear-poweron-ram`: experimental, off by default: zero RAM regions still holding the C64 power-on pattern (see above)

Output files are overwritten without prompting.

### GUI

The GUI provides the same functionality with file browsers and a CRT options tab. Select cartridge type (EasyFlash or Magic Desk) from the dropdown. LOAD/SAVE hooking with an include directory and the hook-address controls (auto location or a manual address) work for both cartridge types. The Compression dropdown selects the pack format. The experimental "Clear power-on RAM pattern" checkbox (off by default, with a confirmation dialog) is available for all output formats. If conversion fails, a dialog offers to add manual RAM blocks.

Conversion runs in the background, so the window stays responsive. A dialog shows the
current step and offers Cancel. Cancelling takes effect once the step in progress
finishes, which with the slower formats can mean waiting for the RAM block to finish
packing. Temporary files are removed whether the run succeeds, fails or is cancelled.

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
5. Decompresses the packed blocks into RAM.
6. Restores page 1, stack and system vectors.
7. Executes RTI back to the snapshot's PC and flags.

## Compression

Packing is done by [lzan](https://github.com/tommyo123/lzan), which produces the raw
stream for each format. The matching 6502 decruncher is assembled into the output, so
nothing beyond the converter is needed to run the result.

Pick a format with `--format` on the CLI or the Compression dropdown in the GUI:

| Format | Notes |
| --- | --- |
| `lzsa1` | Default. Fast to pack and fast to decrunch. |
| `lzsa2` | Smaller output, very slow to pack. |
| `zx0` | Smaller output, very slow to pack. |
| `zx02` | Smaller output, very slow to pack. |
| `lzan-min` | Smaller output, very slow to pack. |
| `bolt` | Fastest decrunch on the C64. |
| `bb2` | ByteBoozer2. |

Packing time is a one-off cost on the PC. Decrunch time is what the C64 spends on
startup. The formats marked very slow can take minutes on a full 64K snapshot.

All formats are checked against the same limits: the relocated decruncher has to fit in
page 1 alongside the stack, and the packed RAM block has to leave enough headroom for
in-place decompression. A snapshot that does not fit is rejected with an error rather
than producing a broken file.

## Troubleshooting

**"Failed to allocate block ..."**
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

## Third-party code

- [lzan](https://github.com/tommyo123/lzan) provides the packers for every supported
  format. It is a normal cargo git dependency, pinned in `Cargo.lock`.
- [asm6502](https://github.com/tommyo123/asm6502) assembles the generated 6502 source.
- The embedded LZSA1 and LZSA2 6502 decrunchers derive from Emmanuel Marty's reference
  decompressors, used under the zlib license. See `LICENSE.zlib-lzsa.md`.
- The ZX0, ZX02 and ByteBoozer2 decrunchers derive from their upstream 6502
  implementations. Each decruncher source in `src/decrunchers/` carries its own
  attribution and license header.
