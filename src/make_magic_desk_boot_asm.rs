//! Magic Desk boot code generator
//!
//! Generates ROML bank 0 boot code with CBM80 signature for Magic Desk cartridge.
//! On RESET, KERNAL checks for "CBM80" at $8004 and does JMP ($8000).
//! Boot code copies trampoline to $0100 which copies restore code to $0340.
//!
// Copyright (c) 2025-2026 Tommy Olsen
// Licensed under the MIT License.

use crate::asm_wrapper::assemble_to_bytes;

/// Magic Desk boot code generator
/// Generates code at $8000 with CBM80 signature that boots the restore process
pub struct MakeMagicDeskBootAsm {
    restore_code_size: usize,
    /// The ROM bank where the restore payload (restore code + decompressor +
    /// RAM.lzsa) begins. 0 for plain snapshots (payload follows the boot code in
    /// bank 0). 1 when files are embedded — bank 0 is then reserved for the boot
    /// code plus the LOAD directory (handler/metadata/filenames).
    restore_start_bank: usize,
}

impl MakeMagicDeskBootAsm {
    pub fn new(restore_code_size: usize) -> Self {
        Self { restore_code_size, restore_start_bank: 0 }
    }

    /// Create a boot generator whose restore payload starts at the given bank.
    pub fn with_restore_start_bank(restore_code_size: usize, restore_start_bank: usize) -> Self {
        Self { restore_code_size, restore_start_bank }
    }

    /// Generate complete boot code binary (placed at offset 0 in bank 0 ROML)
    /// Returns raw binary starting at $8000
    pub fn generate_boot_code(&self) -> Result<Vec<u8>, String> {
        let asm_source = self.generate_boot_asm();
        assemble_to_bytes(&asm_source)
    }

    fn generate_boot_asm(&self) -> String {
        let trampoline_asm = self.generate_trampoline_asm();

        format!(
            r#"; Magic Desk Boot Code @ $8000
; CBM80 signature enables KERNAL autostart: JMP ($8000) on RESET
*=$8000

; =============================================================================
; Standard C64 cartridge header (9 bytes)
; =============================================================================
    ; Cold start vector (points to cold_start below)
    .word cold_start
    ; Warm start vector (same as cold start)
    .word cold_start
    ; CBM80 signature: $C3, $C2, $CD, $38, $30
    .byte $C3, $C2, $CD, $38, $30

; =============================================================================
; cold_start: Initialize CPU and copy trampoline to $0100
; =============================================================================
cold_start:
    SEI
    CLD

    ; Set up CPU port for I/O access
    ; CRITICAL: Set PORT ($01) BEFORE DDR ($00)!
    ; If DDR is set first, bits 0-2 become outputs using the OLD PORT value,
    ; which may have LORAM=0, causing ROML to unmap mid-execution.
    LDA #$37
    STA $01
    LDA #$2F
    STA $00

    ; Clear all pending interrupts
    LDA $DC0D
    LDA $DD0D
    LDA #$7F
    STA $DC0D
    STA $DD0D
    LDA #$00
    STA $D01A
    LDA #$FF
    STA $D019

    ; Set stack pointer to $FF
    LDX #$FF
    TXS

    ; Copy trampoline code to $0100
    LDX #$00
copy_trampoline:
    LDA trampoline_code,X
    STA $0100,X
    INX
    CPX #TRAMPOLINE_SIZE
    BNE copy_trampoline

    ; Jump to trampoline in RAM @ $0100
    JMP $0100

; =============================================================================
; Trampoline code (will be copied to $0100)
; Copies restore code from ROML to $0340, handles bank boundaries
; =============================================================================
trampoline_code:
{}
trampoline_end:

TRAMPOLINE_SIZE = trampoline_end - trampoline_code
"#,
            trampoline_asm
        )
    }

    /// Generate trampoline assembly that copies restore code from ROML to $0340
    /// This runs at $0100 after being copied from boot code area
    fn generate_trampoline_asm(&self) -> String {
        let pages = (self.restore_code_size + 255) / 256;

        if pages > 255 {
            panic!(
                "Restore code too large: {} bytes = {} pages (max 255 pages)",
                self.restore_code_size, pages
            );
        }

        // Bank select + source pointer for the restore code.
        // - restore_start_bank == 0: payload follows the boot code in bank 0, so
        //   the source pointer is the `trampoline_end` label.
        // - restore_start_bank == 1: bank 0 is the directory bank; the restore
        //   payload lives in bank 1 starting at $8000.
        let bank_and_source = if self.restore_start_bank == 0 {
            r#"    ; Select bank 0 via $DE00 (I/O already enabled from boot code)
    LDA #$00
    STA $DE00
    STA $F7           ; Bank counter in $F7

    ; Switch to ROML+RAM mode (ROML visible for reads, RAM for writes)
    LDA #$33
    STA $01

    ; Source pointer: payload starts right after boot code in bank 0
    LDA #>trampoline_end
    STA $FC
    LDA #<trampoline_end
    STA $FB"#.to_string()
        } else {
            format!(
                r#"    ; Select restore bank via $DE00 (I/O already enabled from boot code)
    LDA #${:02X}
    STA $DE00
    STA $F7           ; Bank counter in $F7

    ; Switch to ROML+RAM mode (ROML visible for reads, RAM for writes)
    LDA #$33
    STA $01

    ; Source pointer: restore payload starts at $8000 of the restore bank
    LDA #$80
    STA $FC
    LDA #$00
    STA $FB"#,
                self.restore_start_bank
            )
        };

        format!(
            r#"    ; Trampoline @ $0100 (MINIMAL - copy restore code from ROML to $0340)

{}

    ; Destination: $0340
    LDA #$03
    STA $FE
    LDA #$40
    STA $FD

    ; Pages to copy
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
    ; Check for bank boundary ($A000 = end of ROML window)
    LDA $FC
    CMP #$A0
    BNE no_bank_switch
    ; Switch to next bank
    LDA #$37
    STA $01
    INC $F7
    LDA $F7
    STA $DE00
    LDA #$33
    STA $01
    LDA #$80
    STA $FC           ; Reset source to $8000
    LDA #$00
    STA $FB
no_bank_switch:
    DEC $F8
    BNE copy_restore

restore_done:
    ; Jump to main restore code in RAM @ $0340
    JMP $0340"#,
            bank_and_source, pages
        )
    }
}
