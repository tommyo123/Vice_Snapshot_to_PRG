//! EasyFlash ROMH @ $E000 code generator
//!
//! ROMH copies restore code from ROML to $0340 and jumps there.
//! The main restore code at $0340 handles all complex data copying.
//! Also copies LOAD/SAVE trampoline to RAM if files are embedded.
//!
// Copyright (c) 2025 Tommy Olsen
// Licensed under the MIT License.

use crate::asm_wrapper::assemble_to_bytes;
use crate::crt_builder::BANK_SIZE_8K;

/// EasyFlash ROMH code generator
pub struct MakeROMHAsm {
    restore_code_size: usize,
    load_save_code: Option<Vec<u8>>,
    metadata: Option<Vec<u8>>,
    filenames: Option<Vec<u8>>,
}

impl MakeROMHAsm {
    /// Create a new ROMH generator
    pub fn new(
        restore_code_size: usize,
        load_save_code: Option<Vec<u8>>,
        metadata: Option<Vec<u8>>,
        filenames: Option<Vec<u8>>,
    ) -> Self {
        Self {
            restore_code_size,
            load_save_code,
            metadata,
            filenames,
        }
    }

    /// Generate complete ROMH bank @ $E000 (8KB)
    pub fn generate_romh(&self) -> Result<[u8; BANK_SIZE_8K], String> {
        let asm_source = self.generate_romh_asm();
        let assembled = assemble_to_bytes(&asm_source)?;

        let mut romh = [0u8; BANK_SIZE_8K];

        // Copy assembled code
        let copy_len = assembled.len().min(BANK_SIZE_8K);
        romh[..copy_len].copy_from_slice(&assembled[..copy_len]);

        // Set interrupt vectors at $FFFA-$FFFF (offsets $1FFA-$1FFF in 8KB bank)
        // NMI vector @ $FFFA/$FFFB -> $E000 (RTI)
        romh[0x1FFA] = 0x00;
        romh[0x1FFB] = 0xE0;

        // RESET vector @ $FFFC/$FFFD -> $E001 (start)
        romh[0x1FFC] = 0x01;
        romh[0x1FFD] = 0xE0;

        // IRQ vector @ $FFFE/$FFFF -> $E000 (RTI)
        romh[0x1FFE] = 0x00;
        romh[0x1FFF] = 0xE0;

        // Write LOAD/SAVE code at offset $0600 if provided (will be @ $A600 in 16K mode)
        if let Some(ref code) = self.load_save_code {
            let code_offset = 0x0600;
            let copy_size = code.len().min(0x0A00); // Max ~2.5KB
            romh[code_offset..code_offset + copy_size].copy_from_slice(&code[..copy_size]);
        }

        // Write metadata at offset $1000 if provided (will be @ $B000 in 16K mode)
        if let Some(ref meta) = self.metadata {
            let meta_offset = 0x1000;
            let copy_size = meta.len().min(0x0800); // Max 2KB
            romh[meta_offset..meta_offset + copy_size].copy_from_slice(&meta[..copy_size]);
        }

        // Write filenames at offset $1800 if provided (will be @ $B800 in 16K mode)
        if let Some(ref names) = self.filenames {
            let names_offset = 0x1800;
            let copy_size = names.len().min(0x07FC); // Max ~2KB, avoid vectors
            romh[names_offset..names_offset + copy_size].copy_from_slice(&names[..copy_size]);
        }

        Ok(romh)
    }

    fn generate_romh_asm(&self) -> String {
        let boot_trampoline_asm = self.generate_boot_trampoline_asm();

        // NOTE: LOAD/SAVE trampoline is NOT copied here!
        // It is written to RAM at $0334 before compression, and gets decompressed
        // back to $0334 when RAM.lzsa is decompressed. This is necessary because:
        // - $0100 is used by boot trampoline during startup
        // - $0100 is used by relocated LZSA decompressor
        // - Only RAM from $0200-$FFEF is compressed to RAM.lzsa

        format!(
            r#"; C64 EasyFlash ROMH @ $E000
; Boot routine that copies trampoline to $100 and jumps there
*=$E000

EASYFLASH_ROML = $DE00
EASYFLASH_CONTROL = $DE02

; $E000: RTI for NMI/IRQ vectors
    RTI

; $E001: RESET entry point
start:
    SEI
    CLD

    LDA #$37
    STA $00
    LDA #$37
    STA $01

    LDA $DC0D
    LDA $DD0D
    LDA #$7F
    STA $DC0D
    STA $DD0D
    LDA #$00
    STA $D01A
    LDA #$FF
    STA $D019

    LDX #$FF
    TXS

    LDX #$00
copy_boot_trampoline:
    LDA boot_trampoline_code,X
    STA $0100,X
    INX
    CPX #BOOT_TRAMPOLINE_SIZE
    BNE copy_boot_trampoline

    JMP $0100

    JMP after_trampoline

boot_trampoline_code:
{}
boot_trampoline_end:

after_trampoline:
BOOT_TRAMPOLINE_SIZE = boot_trampoline_end - boot_trampoline_code

*=$FFFA
    .word $E000    ; NMI vector
    .word $E001    ; RESET vector
    .word $E000    ; IRQ vector
"#,
            boot_trampoline_asm
        )
    }

    fn generate_boot_trampoline_asm(&self) -> String {
        let roml_restore_code_start = 0x8000;
        let src_hi = (roml_restore_code_start >> 8) & 0xFF;
        let src_lo = roml_restore_code_start & 0xFF;
        let pages = (self.restore_code_size + 255) / 256;

        format!(
            r#"    ; Trampoline @ $0100 (MINIMAL)

    LDA #$37
    STA $01

    LDA #$00
    STA EASYFLASH_ROML

    LDA #$06
    STA EASYFLASH_CONTROL

    LDA #$33
    STA $01

    ; Copy restore code from ROML ${:04X} to RAM $0340
    LDA #${:02X}
    STA $FC
    LDA #${:02X}
    STA $FB

    LDA #$03
    STA $FE
    LDA #$40
    STA $FD

    LDA #${:02X}
    STA $F8

copy_restore:
    LDA $F8
    BEQ restore_done
    LDY #$00
copy_restore_byte:
    LDA ($FB),Y
    STA ($FD),Y
    INY
    BNE copy_restore_byte
    INC $FC
    INC $FE
    DEC $F8
    BNE copy_restore

restore_done:
    JMP $0340
"#,
            roml_restore_code_start, src_hi, src_lo, pages
        )
    }
}
