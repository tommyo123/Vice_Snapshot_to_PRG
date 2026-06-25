# Vendored EasyFlash file-system binaries

These prebuilt 6502 blobs are embedded into generated EasyFlash **save** cartridges.

| File | Origin | Load addr | Purpose |
|------|--------|-----------|---------|
| `lib-efs.prg` | [Drunella/libefs](https://github.com/Drunella/libefs) @ `42e5570837619dd0489592a0c89496dc0a8a7299` | $8000 | EasyFlash filesystem library (read/write, dual-area GC) |
| `eapi-am29f040.prg` | EasyFlash EAPI (AM29F040), via libefs | page-aligned | Flash program/erase driver, copied to C64 RAM for writes |

`lib-efs.prg` rebuild (cc65 V2.19):
```
cd libefs && CC65_HOME=<cc65>   ca65 -t c64 -I . -D BANKMODE=1 -g -o build/lib/<each>.o src/lib/<each>.s   # 6 modules
  ld65 -m build/lib/lib-efs.map -o build/lib-efs.prg -C src/lib/lib-efs.cfg c64.lib build/lib/*.o
```
(BANKMODE=1 = "ll". Revisit if the writable-area banking mode changes.)

libefs is Apache-2.0 (see LICENSE). EAPI is part of the EasyFlash project.
