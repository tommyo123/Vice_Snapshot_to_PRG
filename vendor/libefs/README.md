# Vendored EasyFlash file-system binaries

These prebuilt 6502 blobs are embedded into generated EasyFlash **save** cartridges.

| File | Origin | Load addr | Purpose |
|------|--------|-----------|---------|
| `lib-efs.prg` | [Drunella/libefs](https://github.com/Drunella/libefs) @ `42e5570837619dd0489592a0c89496dc0a8a7299` **+ local patch** | $8000 | EasyFlash filesystem library (read/write, dual-area GC) |
| `eapi-am29f040.prg` | EasyFlash EAPI (AM29F040), via libefs | page-aligned | Flash program/erase driver, copied to C64 RAM for writes |

### Local patch (defragment crash fix)

`lib-efs.prg` is built from upstream `42e5570` with one fix to `src/lib/lib-efs-rw.s`:
the three `jsr rom_config_call_defragment_warning` / `..._allclear` calls inside
`rom_defragment_copy` (and `rom_defragment_copy_data_destinc`) are removed.

Those callbacks read the configuration block (bank 0 HIROM, `$bb18`) via
`rom_config_get_value`, which does **not** select bank 0 — but they run from the
defragment copy loop with the *source* area's bank selected, so they read the
`dfcall` flag (and the callback vector) from the wrong bank, get garbage, and
`jmp` to a bogus address (CPU jams at `$0008`). This crashes every defragment.

We always build with the defragment callbacks disabled (`dfcall = 0`), so the
calls are dead weight anyway; removing them makes garbage collection work
(verified in VICE: dozens of saves across multiple defrag cycles, data
preserved, no reset needed). Upstream-worthy: `rom_config_get_value` should
select bank 0 before reading the config.

`lib-efs.prg` rebuild (cc65 V2.19 — note: this ca65 rejects `-O`):
```
cd libefs && CC65_HOME=<cc65>
  ca65 -t c64 -D BANKMODE=1 -g -o build/lib/<each>.o src/lib/<each>.s   # 6 modules
  ld65 -m build/lib/lib-efs.map -o build/lib-efs.prg -C src/lib/lib-efs.cfg c64.lib build/lib/*.o
```
(BANKMODE=1 = "ll". Revisit if the writable-area banking mode changes.)

libefs is Apache-2.0 (see LICENSE). EAPI is part of the EasyFlash project.
