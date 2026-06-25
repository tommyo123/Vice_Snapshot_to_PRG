//! KERNAL SAVE/LOAD hooking for the EasyFlash libefs filesystem.
//!
//! Generates a small RAM-resident trampoline that intercepts the KERNAL LOAD
//! ($0330) and SAVE ($0332) vectors and routes them to libefs (`EFS_load` /
//! `EFS_save`). The trampoline is written into the snapshot RAM image before
//! compression (like the EasyFlash/Magic Desk LOAD hooks), so it and the hooked
//! vectors come back automatically when RAM is decompressed at boot.
//!
//! On each call the trampoline banks the cartridge in (16K mode), (re)initializes
//! libefs + EAPI, sets the filename, performs the operation, and banks the
//! cartridge back out. libefs keeps its code/variables in EF-RAM ($DF00-$DF7F);
//! only EAPI needs a page-aligned 768-byte buffer in C64 RAM, supplied by the
//! converter via `eapi_page_hi`.
//!
//! Entry layout: the blob begins with a 2-entry jump table
//!   blob+0: JMP save   (hook $0332 here)
//!   blob+3: JMP load   (hook $0330 here)
//
// Copyright (c) 2025-2026 Tommy Olsen
// Licensed under the MIT License.

use crate::asm_wrapper::assemble_to_bytes;
use crate::ef_save::{EFS_INIT, EFS_INIT_EAPI, EFS_LOAD, EFS_SAVE, EFS_SETNAM, EFS_UTIL};

pub const LOAD_VECTOR: usize = 0x0330;
pub const SAVE_VECTOR: usize = 0x0332;

/// Generates the libefs SAVE/LOAD trampoline blob.
pub struct EfSaveHook {
    blob_address: u16,
    eapi_page_hi: u8,
    temp_filename_addr: u16,
    binary: Vec<u8>,
}

impl EfSaveHook {
    /// `blob_address`: where the trampoline is placed in C64 RAM.
    /// `eapi_page_hi`: high byte of the page-aligned 768-byte EAPI buffer.
    pub fn new(blob_address: u16, eapi_page_hi: u8) -> Self {
        Self { blob_address, eapi_page_hi, temp_filename_addr: 0, binary: Vec::new() }
    }

    /// Address to hook into the SAVE vector ($0332).
    pub fn save_entry(&self) -> u16 {
        self.blob_address
    }

    /// Address to hook into the LOAD vector ($0330).
    pub fn load_entry(&self) -> u16 {
        self.blob_address + 3
    }

    pub fn get_binary(&self) -> &[u8] {
        &self.binary
    }

    fn generate_asm(&self, temp_addr: u16) -> String {
        format!(
            r#"*=${blob:04X}

    JMP save_tramp        ; +0  hook SAVE ($0332) here
    JMP load_tramp        ; +3  hook LOAD ($0330) here

; ---- SAVE: KERNAL ISAVE -> EFS_save ----
; The KERNAL SAVE routine ($F5DD) has already placed the start address in $C1/$C2
; and the end address in $AE/$AF before JMP ($0332); A/X/Y are NOT the parameters.
; Capture them before libefs runs (its init clobbers zero page).
save_tramp:
    LDA $C1
    STA save_start
    LDA $C2
    STA save_start+1
    LDA $AE
    STA save_end
    LDA $AF
    STA save_end+1
    SEI
    JSR copy_filename_save
    JSR bank_in
    JSR ${efs_init:04X}        ; EFS_init
    LDA #${eapi:02X}
    JSR ${efs_init_eapi:04X}   ; EFS_init_eapi
    LDA name_len
    LDX #<${temp:04X}
    LDY #>${temp:04X}
    JSR ${efs_setnam:04X}      ; EFS_setnam
    LDA save_start         ; place start in $FB/$FC for EFS_save (A = $FB)
    STA $FB
    LDA save_start+1
    STA $FC
    LDA #$FB
    LDX save_end
    LDY save_end+1
    JSR ${efs_save:04X}        ; EFS_save
    PHP
    JSR bank_out
    PLP
    CLI
    RTS

; ---- LOAD: KERNAL ILOAD -> EFS_load ----
; entry: A = 0 load / 1 verify, X/Y = relocation address, $B9 = secondary addr
load_tramp:
    STA load_a
    STX load_x
    STY load_y
    LDA $B9
    STA load_sa
    SEI
    JSR copy_filename
    JSR bank_in
    JSR ${efs_init:04X}        ; EFS_init
    LDA #${eapi:02X}
    JSR ${efs_init_eapi:04X}   ; EFS_init_eapi
    LDA #$00                ; EFS_util: setlfs ($0x)
    LDY load_sa            ; secondary address (0 relocate / 1 file address)
    JSR ${efs_util:04X}
    LDA name_len
    LDX #<${temp:04X}
    LDY #>${temp:04X}
    JSR ${efs_setnam:04X}      ; EFS_setnam
    LDA load_a
    LDX load_x
    LDY load_y
    JSR ${efs_load:04X}        ; EFS_load
    STX end_lo
    STY end_hi
    PHA
    PHP
    JSR bank_out
    PLP
    PLA
    LDX end_lo
    LDY end_hi
    CLI
    RTS

; ---- helpers ----
; Copy the KERNAL filename to temp and record its length (both survive libefs).
copy_filename:
    LDA $B7
    STA name_len
    BEQ cf_done
    LDY name_len
    DEY
cf_loop:
    LDA ($BB),Y
    STA ${temp:04X},Y
    DEY
    BPL cf_loop
cf_done:
    RTS

; Filename copy for SAVE. This cartridge is persistent storage, so a plain
; SAVE"NAME" should replace any existing file (libefs, like a 1541, otherwise
; reports "file exists"). We therefore auto-prepend the "@0:" replace command,
; unless the program already supplied its own "@..." command.
copy_filename_save:
    LDA $B7
    STA name_len
    BEQ cfs_done
    LDY #$00
    LDA ($BB),Y
    CMP #$40            ; '@' -> already a command, copy unchanged
    BEQ cfs_plain
    LDA #$40            ; '@'
    STA ${t0:04X}
    LDA #$30            ; '0'
    STA ${t1:04X}
    LDA #$3A            ; ':'
    STA ${t2:04X}
    LDY #$00
cfs_pre:
    LDA ($BB),Y
    STA ${t3:04X},Y
    INY
    CPY $B7
    BNE cfs_pre
    LDA $B7
    CLC
    ADC #$03
    STA name_len
    RTS
cfs_plain:
    LDY name_len
    DEY
cfs_ploop:
    LDA ($BB),Y
    STA ${temp:04X},Y
    DEY
    BPL cfs_ploop
cfs_done:
    RTS

bank_in:
    LDA #$37
    STA $01
    LDA #$87           ; LED + 16K mode
    STA $DE02
    LDA #$00           ; bank 0 (libefs)
    STA $DE00
    RTS

bank_out:
    LDA #$04           ; EXROM/GAME high = cartridge off
    STA $DE02
    LDA #$37
    STA $01
    RTS

save_start:
    .byte $00
    .byte $00
save_end:
    .byte $00
    .byte $00
load_a:
    .byte $00
load_x:
    .byte $00
load_y:
    .byte $00
load_sa:
    .byte $00
name_len:
    .byte $00
end_lo:
    .byte $00
end_hi:
    .byte $00
"#,
            blob = self.blob_address,
            efs_init = EFS_INIT,
            efs_init_eapi = EFS_INIT_EAPI,
            efs_setnam = EFS_SETNAM,
            efs_save = EFS_SAVE,
            efs_load = EFS_LOAD,
            efs_util = EFS_UTIL,
            eapi = self.eapi_page_hi,
            temp = temp_addr,
            t0 = temp_addr,
            t1 = temp_addr + 1,
            t2 = temp_addr + 2,
            t3 = temp_addr + 3,
        )
    }

    /// Assemble the trampoline; the temp filename buffer is placed right after
    /// the code (16 bytes reserved by the caller).
    pub fn generate_binary(&mut self) -> Result<Vec<u8>, String> {
        // First pass with a placeholder temp address (size is independent of the
        // temp address value).
        let first = assemble_to_bytes(&self.generate_asm(self.blob_address + 0xF0))?;
        self.temp_filename_addr = self.blob_address + first.len() as u16;
        let bytes = assemble_to_bytes(&self.generate_asm(self.temp_filename_addr))?;
        let bytes = if bytes.len() != first.len() {
            self.temp_filename_addr = self.blob_address + bytes.len() as u16;
            let retry = assemble_to_bytes(&self.generate_asm(self.temp_filename_addr))?;
            if retry.len() != bytes.len() {
                return Err("EF save trampoline size unstable".to_string());
            }
            retry
        } else {
            bytes
        };
        self.binary = bytes.clone();
        Ok(bytes)
    }

    /// Total RAM footprint to reserve: trampoline code + temp filename buffer.
    /// The buffer holds up to a 16-char name plus the auto-prepended "@0:".
    pub fn reserved_len(&self) -> usize {
        self.binary.len() + 24
    }

    pub fn temp_filename_addr(&self) -> u16 {
        self.temp_filename_addr
    }

    /// Write the trampoline into RAM and hook the LOAD/SAVE vectors.
    pub fn hook(&mut self, ram: &mut [u8]) -> Result<(), String> {
        let bin = self.generate_binary()?;
        let addr = self.blob_address as usize;
        if addr + bin.len() + 24 > ram.len() {
            return Err("EF save trampoline exceeds RAM bounds".to_string());
        }
        ram[addr..addr + bin.len()].copy_from_slice(&bin);

        let load = self.load_entry();
        let save = self.save_entry();
        ram[LOAD_VECTOR] = (load & 0xFF) as u8;
        ram[LOAD_VECTOR + 1] = (load >> 8) as u8;
        ram[SAVE_VECTOR] = (save & 0xFF) as u8;
        ram[SAVE_VECTOR + 1] = (save >> 8) as u8;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trampoline_assembles_and_entries_resolve() {
        let mut hook = EfSaveHook::new(0x0334, 0xC0);
        let bin = hook.generate_binary().expect("assembles");
        assert!(!bin.is_empty());
        // jump table at the front: JMP save (4C ..), JMP load (4C ..)
        assert_eq!(bin[0], 0x4C);
        assert_eq!(bin[3], 0x4C);
        assert_eq!(hook.save_entry(), 0x0334);
        assert_eq!(hook.load_entry(), 0x0337);
        // temp filename sits just past the code
        assert_eq!(hook.temp_filename_addr(), 0x0334 + bin.len() as u16);
    }

    #[test]
    fn hook_writes_vectors() {
        let mut ram = vec![0u8; 0x10000];
        let mut hook = EfSaveHook::new(0x0334, 0xC0);
        hook.hook(&mut ram).unwrap();
        // SAVE vector -> save entry, LOAD vector -> load entry
        assert_eq!(ram[SAVE_VECTOR] as u16 | ((ram[SAVE_VECTOR + 1] as u16) << 8), 0x0334);
        assert_eq!(ram[LOAD_VECTOR] as u16 | ((ram[LOAD_VECTOR + 1] as u16) << 8), 0x0337);
    }
}
