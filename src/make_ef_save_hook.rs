//! KERNAL SAVE/LOAD and channel hooking for the EasyFlash libefs filesystem.
//!
//! Generates a small RAM-resident trampoline that intercepts KERNAL LOAD/SAVE and
//! channel-based vectors (OPEN, CLOSE, CHKIN, CKOUT, CLRCHN, CHRIN, CHROUT) and routes
//! them to their libefs equivalents. The trampoline is written into the snapshot RAM
//! image before compression, so it and the hooked vectors come back automatically
//! when RAM is decompressed at boot.
//!
//! On each call the trampoline banks the cartridge in (16K mode), initializes/runs
//! the requested operation, and banks the cartridge back out.
//
// Copyright (c) 2025-2026 Tommy Olsen
// Licensed under the MIT License.

use crate::asm_wrapper::assemble_to_bytes;
use crate::ef_save::{
    EFS_CLOSE, EFS_CHRIN, EFS_CHROUT, EFS_INIT, EFS_INIT_EAPI, EFS_LOAD, EFS_OPEN, EFS_SAVE,
    EFS_SETNAM, EFS_UTIL,
};

// KERNAL vectors on page 3
pub const OPEN_VECTOR: usize = 0x031C;
pub const CLOSE_VECTOR: usize = 0x031E;
pub const CHKIN_VECTOR: usize = 0x0320;
pub const CKOUT_VECTOR: usize = 0x0322;
pub const CLRCHN_VECTOR: usize = 0x0324;
pub const CHRIN_VECTOR: usize = 0x0326;
pub const CHROUT_VECTOR: usize = 0x0328;
pub const LOAD_VECTOR: usize = 0x0330;
pub const SAVE_VECTOR: usize = 0x0332;

/// Generates the libefs SAVE/LOAD/channel trampoline blob.
pub struct EfSaveHook {
    blob_address: u16,
    eapi_page_hi: u8,
    temp_filename_addr: u16,
    binary: Vec<u8>,
    stash_address: Option<u16>,
    screen_address: u16,
    blank_screen: bool,

    // Original KERNAL vectors backed up from the snapshot
    open_orig: u16,
    close_orig: u16,
    chkin_orig: u16,
    ckout_orig: u16,
    clrchn_orig: u16,
    chrin_orig: u16,
    chrout_orig: u16,
    load_orig: u16,
    save_orig: u16,
}

impl EfSaveHook {
    /// `blob_address`: where the trampoline is placed in C64 RAM.
    /// `eapi_page_hi`: high byte of the page-aligned 768-byte EAPI buffer.
    pub fn new(blob_address: u16, eapi_page_hi: u8) -> Self {
        Self {
            blob_address,
            eapi_page_hi,
            temp_filename_addr: 0,
            binary: Vec::new(),
            stash_address: None,
            screen_address: 0,
            blank_screen: false,
            open_orig: 0xF34A,
            close_orig: 0xF291,
            chkin_orig: 0xF214,
            ckout_orig: 0xF250,
            clrchn_orig: 0xF32F,
            chrin_orig: 0xF157,
            chrout_orig: 0xF1CA,
            load_orig: 0xF49E,
            save_orig: 0xF5DD,
        }
    }

    /// Set screen stashing addresses
    pub fn with_stash(mut self, stash_address: u16, screen_address: u16) -> Self {
        self.stash_address = Some(stash_address);
        self.screen_address = screen_address;
        self
    }

    /// Set screen blanking option
    pub fn with_blank(mut self, blank_screen: bool) -> Self {
        self.blank_screen = blank_screen;
        self
    }

    pub fn save_entry(&self) -> u16 {
        self.blob_address
    }

    pub fn load_entry(&self) -> u16 {
        self.blob_address + 3
    }

    pub fn open_entry(&self) -> u16 {
        self.blob_address + 6
    }

    pub fn close_entry(&self) -> u16 {
        self.blob_address + 9
    }

    pub fn chkin_entry(&self) -> u16 {
        self.blob_address + 12
    }

    pub fn ckout_entry(&self) -> u16 {
        self.blob_address + 15
    }

    pub fn clrchn_entry(&self) -> u16 {
        self.blob_address + 18
    }

    pub fn chrin_entry(&self) -> u16 {
        self.blob_address + 21
    }

    pub fn chrout_entry(&self) -> u16 {
        self.blob_address + 24
    }

    pub fn get_binary(&self) -> &[u8] {
        &self.binary
    }

    fn generate_asm(&self, temp_addr: u16) -> String {
        let (stash_call, stash_restore, stash_sub) = if let Some(stash) = self.stash_address {
            (
                "    JSR swap_to_stash\n",
                "    JSR swap_from_stash\n",
                format!(
                    r#"
swap_to_stash:
    LDA #<${screen:04X}
    STA $FB
    LDA #>${screen:04X}
    STA $FC
    LDA #<${stash:04X}
    STA $FD
    LDA #>${stash:04X}
    STA $FE
    JMP copy_1k

swap_from_stash:
    LDA #<${stash:04X}
    STA $FB
    LDA #>${stash:04X}
    STA $FC
    LDA #<${screen:04X}
    STA $FD
    LDA #>${screen:04X}
    STA $FE
    ; fall through to copy_1k

copy_1k:
    LDX #$04
    LDY #$00
copy_loop_1k:
    LDA ($FB),Y
    STA ($FD),Y
    INY
    BNE copy_loop_1k
    INC $FC
    INC $FE
    DEX
    BNE copy_loop_1k
    RTS
"#,
                    screen = self.screen_address,
                    stash = stash
                ),
            )
        } else {
            ("", "", "".to_string())
        };

        let (blank_call, blank_restore, blank_sub, blank_var) = if self.blank_screen {
            (
                "    JSR do_blank\n",
                "    JSR do_restore\n",
                r#"
do_blank:
    LDA $D011
    STA d011_save
    AND #$EF
    STA $D011
    RTS

do_restore:
    LDA d011_save
    STA $D011
    RTS
"#,
                "d011_save:\n    .byte $00\n",
            )
        } else {
            ("", "", "", "")
        };

        format!(
            r#"*=${blob:04X}

    JMP save_tramp        ; +0  hook SAVE ($0332) here
    JMP load_tramp        ; +3  hook LOAD ($0330) here
    JMP open_tramp        ; +6  hook OPEN ($031C) here
    JMP close_tramp       ; +9  hook CLOSE ($031E) here
    JMP chkin_tramp       ; +12 hook CHKIN ($0320) here
    JMP ckout_tramp       ; +15 hook CKOUT ($0322) here
    JMP clrchn_tramp      ; +18 hook CLRCHN ($0324) here
    JMP chrin_tramp       ; +21 hook CHRIN ($0326) here
    JMP chrout_tramp      ; +24 hook CHROUT ($0328) here

; ---- SAVE: KERNAL ISAVE -> EFS_save ----
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
{blank_call}{stash_call}    JSR bank_in
    JSR ${efs_init:04X}        ; EFS_init
    LDA #${eapi:02X}
    JSR ${efs_init_eapi:04X}   ; EFS_init_eapi
    LDA name_len
    LDX #<${temp:04X}
    LDY #>${temp:04X}
    JSR ${efs_setnam:04X}      ; EFS_setnam
    LDA save_start
    STA $FB
    LDA save_start+1
    STA $FC
    LDA #$FB
    LDX save_end
    LDY save_end+1
    JSR ${efs_save:04X}        ; EFS_save
    PHA
    PHP
    JSR bank_out
{stash_restore}{blank_restore}    PLP
    PLA
    CLI
    RTS

; ---- LOAD: KERNAL ILOAD -> EFS_load ----
load_tramp:
    STA load_a
    STX load_x
    STY load_y
    LDA $B9
    STA load_sa
    SEI
    JSR copy_filename
{blank_call}{stash_call}    JSR bank_in
    JSR ${efs_init:04X}        ; EFS_init
    LDA #${eapi:02X}
    JSR ${efs_init_eapi:04X}   ; EFS_init_eapi
    LDA #$00
    LDY load_sa
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
{stash_restore}{blank_restore}    PLP
    PLA
    LDX end_lo
    LDY end_hi
    CLI
    RTS

; ---- OPEN: KERNAL IOPEN -> EFS_open ----
open_tramp:
    LDA $BA             ; check device number
    CMP #$08            ; is it EFS device (8)?
    BEQ open_efs
    JMP (open_orig)     ; pass through to original vector
open_efs:
    SEI                 ; keep interrupts disabled while file is open
    JSR copy_filename
{blank_call}{stash_call}    JSR bank_in
    JSR ${efs_init:04X}        ; EFS_init
    LDA #${eapi:02X}
    JSR ${efs_init_eapi:04X}   ; EFS_init_eapi
    LDA name_len
    LDX #<${temp:04X}
    LDY #>${temp:04X}
    JSR ${efs_setnam:04X}      ; EFS_setnam
    JSR ${efs_open:04X}        ; EFS_open
    JSR bank_out
    LDA $B8             ; logical file number
    STA $0259           ; set LAT[0]
    LDA #$08
    STA $0263           ; set FAT[0] (device 8)
    LDA $B9             ; secondary address
    STA $026D           ; set SAT[0]
    LDA #$01
    STA $98             ; set LDTND = 1
    CLC                 ; success
    RTS

; ---- CLOSE: KERNAL ICLOSE -> EFS_close ----
close_tramp:
    CMP $0259           ; matches our logical file?
    BEQ close_efs
    JMP (close_orig)
close_efs:
    JSR bank_in
    JSR ${efs_close:04X}       ; EFS_close
    JSR bank_out
    LDA #$00
    STA $98             ; set LDTND = 0
{stash_restore}{blank_restore}    CLI                 ; restore interrupts
    CLC
    RTS

; ---- CHKIN: KERNAL ICHKIN ----
chkin_tramp:
    TXA                 ; logical file number in X
    CMP $0259           ; matches ours?
    BEQ chkin_efs
    JMP (chkin_orig)
chkin_efs:
    LDA #$08
    STA $99             ; input channel = 8
    CLC
    RTS

; ---- CKOUT: KERNAL ICKOUT ----
ckout_tramp:
    TXA                 ; logical file number in X
    CMP $0259           ; matches ours?
    BEQ ckout_efs
    JMP (ckout_orig)
ckout_efs:
    LDA #$08
    STA $9A             ; output channel = 8
    CLC
    RTS

; ---- CLRCHN: KERNAL ICLRCHN ----
clrchn_tramp:
    LDA $99
    CMP #$08
    BEQ clrchn_efs
    LDA $9A
    CMP #$08
    BEQ clrchn_efs
    JMP (clrchn_orig)
clrchn_efs:
    LDA #$00
    STA $99             ; keyboard
    LDA #$03
    STA $9A             ; screen
    CLC
    RTS

; ---- CHRIN: KERNAL ICHRIN -> EFS_chrin ----
chrin_tramp:
    LDA $99
    CMP #$08            ; is EFS channel active?
    BEQ chrin_efs
    JMP (chrin_orig)
chrin_efs:
    JSR bank_in
    JSR ${efs_chrin:04X}       ; EFS_chrin
    PHA
    JSR bank_out
    PLA
    RTS

; ---- CHROUT: KERNAL ICHROUT -> EFS_chrout ----
chrout_tramp:
    PHA
    LDA $9A
    CMP #$08            ; is EFS channel active?
    BEQ chrout_efs
    PLA
    JMP (chrout_orig)
chrout_efs:
    JSR bank_in
    PLA
    JSR ${efs_chrout:04X}      ; EFS_chrout
    JSR bank_out
    RTS

; ---- helpers ----
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

copy_filename_save:
    LDA $B7
    STA name_len
    BEQ cfs_done
    LDY #$00
    LDA ($BB),Y
    CMP #$40
    BEQ cfs_plain
    LDA #$40
    STA ${t0:04X}
    LDA #$30
    STA ${t1:04X}
    LDA #$3A
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
    LDA $01
    STA port_01_save
    LDA #$37
    STA $01
    LDA #$87
    STA $DE02
    LDA #$00
    STA $DE00
    RTS

bank_out:
    LDA #$04
    STA $DE02
    LDA port_01_save
    STA $01
    RTS

{stash_sub}{blank_sub}

; ---- state data ----
save_start:
    .byte $00, $00
save_end:
    .byte $00, $00
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
port_01_save:
    .byte $00
{blank_var}

; ---- original vectors (updated dynamically on hook) ----
open_orig:
    .word ${open_orig:04X}
close_orig:
    .word ${close_orig:04X}
chkin_orig:
    .word ${chkin_orig:04X}
ckout_orig:
    .word ${ckout_orig:04X}
clrchn_orig:
    .word ${clrchn_orig:04X}
chrin_orig:
    .word ${chrin_orig:04X}
chrout_orig:
    .word ${chrout_orig:04X}
load_orig:
    .word ${load_orig:04X}
save_orig:
    .word ${save_orig:04X}
"#,
            blob = self.blob_address,
            efs_init = EFS_INIT,
            efs_init_eapi = EFS_INIT_EAPI,
            efs_setnam = EFS_SETNAM,
            efs_save = EFS_SAVE,
            efs_load = EFS_LOAD,
            efs_open = EFS_OPEN,
            efs_close = EFS_CLOSE,
            efs_chrin = EFS_CHRIN,
            efs_chrout = EFS_CHROUT,
            efs_util = EFS_UTIL,
            eapi = self.eapi_page_hi,
            temp = temp_addr,
            t0 = temp_addr,
            t1 = temp_addr + 1,
            t2 = temp_addr + 2,
            t3 = temp_addr + 3,
            stash_call = stash_call,
            stash_restore = stash_restore,
            stash_sub = stash_sub,
            blank_call = blank_call,
            blank_restore = blank_restore,
            blank_sub = blank_sub,
            blank_var = blank_var,
            open_orig = self.open_orig,
            close_orig = self.close_orig,
            chkin_orig = self.chkin_orig,
            ckout_orig = self.ckout_orig,
            clrchn_orig = self.clrchn_orig,
            chrin_orig = self.chrin_orig,
            chrout_orig = self.chrout_orig,
            load_orig = self.load_orig,
            save_orig = self.save_orig,
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

    /// Write the trampoline into RAM and hook all 9 KERNAL vectors.
    pub fn hook(&mut self, ram: &mut [u8]) -> Result<(), String> {
        // 1. Read and backup the existing vectors from the snapshot RAM
        self.open_orig = ram[OPEN_VECTOR] as u16 | ((ram[OPEN_VECTOR + 1] as u16) << 8);
        self.close_orig = ram[CLOSE_VECTOR] as u16 | ((ram[CLOSE_VECTOR + 1] as u16) << 8);
        self.chkin_orig = ram[CHKIN_VECTOR] as u16 | ((ram[CHKIN_VECTOR + 1] as u16) << 8);
        self.ckout_orig = ram[CKOUT_VECTOR] as u16 | ((ram[CKOUT_VECTOR + 1] as u16) << 8);
        self.clrchn_orig = ram[CLRCHN_VECTOR] as u16 | ((ram[CLRCHN_VECTOR + 1] as u16) << 8);
        self.chrin_orig = ram[CHRIN_VECTOR] as u16 | ((ram[CHRIN_VECTOR + 1] as u16) << 8);
        self.chrout_orig = ram[CHROUT_VECTOR] as u16 | ((ram[CHROUT_VECTOR + 1] as u16) << 8);
        self.load_orig = ram[LOAD_VECTOR] as u16 | ((ram[LOAD_VECTOR + 1] as u16) << 8);
        self.save_orig = ram[SAVE_VECTOR] as u16 | ((ram[SAVE_VECTOR + 1] as u16) << 8);

        // 2. Re-assemble binary with actual vector values compiled in
        let bin = self.generate_binary()?;
        let addr = self.blob_address as usize;
        if addr + bin.len() + 24 > ram.len() {
            return Err("EF save trampoline exceeds RAM bounds".to_string());
        }
        ram[addr..addr + bin.len()].copy_from_slice(&bin);

        // 3. Write entry points to vectors
        let save = self.save_entry();
        let load = self.load_entry();
        let open = self.open_entry();
        let close = self.close_entry();
        let chkin = self.chkin_entry();
        let ckout = self.ckout_entry();
        let clrchn = self.clrchn_entry();
        let chrin = self.chrin_entry();
        let chrout = self.chrout_entry();

        ram[SAVE_VECTOR] = (save & 0xFF) as u8;
        ram[SAVE_VECTOR + 1] = (save >> 8) as u8;
        ram[LOAD_VECTOR] = (load & 0xFF) as u8;
        ram[LOAD_VECTOR + 1] = (load >> 8) as u8;
        ram[OPEN_VECTOR] = (open & 0xFF) as u8;
        ram[OPEN_VECTOR + 1] = (open >> 8) as u8;
        ram[CLOSE_VECTOR] = (close & 0xFF) as u8;
        ram[CLOSE_VECTOR + 1] = (close >> 8) as u8;
        ram[CHKIN_VECTOR] = (chkin & 0xFF) as u8;
        ram[CHKIN_VECTOR + 1] = (chkin >> 8) as u8;
        ram[CKOUT_VECTOR] = (ckout & 0xFF) as u8;
        ram[CKOUT_VECTOR + 1] = (ckout >> 8) as u8;
        ram[CLRCHN_VECTOR] = (clrchn & 0xFF) as u8;
        ram[CLRCHN_VECTOR + 1] = (clrchn >> 8) as u8;
        ram[CHRIN_VECTOR] = (chrin & 0xFF) as u8;
        ram[CHRIN_VECTOR + 1] = (chrin >> 8) as u8;
        ram[CHROUT_VECTOR] = (chrout & 0xFF) as u8;
        ram[CHROUT_VECTOR + 1] = (chrout >> 8) as u8;

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
        // verify jump table at the front: 9 entries of JMP (4C xx xx)
        assert_eq!(bin[0], 0x4C);
        assert_eq!(bin[3], 0x4C);
        assert_eq!(bin[6], 0x4C);
        assert_eq!(bin[9], 0x4C);
        assert_eq!(bin[12], 0x4C);
        assert_eq!(bin[15], 0x4C);
        assert_eq!(bin[18], 0x4C);
        assert_eq!(bin[21], 0x4C);
        assert_eq!(bin[24], 0x4C);

        assert_eq!(hook.save_entry(), 0x0334);
        assert_eq!(hook.load_entry(), 0x0337);
        assert_eq!(hook.open_entry(), 0x033A);
        assert_eq!(hook.close_entry(), 0x033D);
        assert_eq!(hook.chkin_entry(), 0x0340);
        assert_eq!(hook.ckout_entry(), 0x0343);
        assert_eq!(hook.clrchn_entry(), 0x0346);
        assert_eq!(hook.chrin_entry(), 0x0349);
        assert_eq!(hook.chrout_entry(), 0x034C);

        // temp filename sits just past the code
        assert_eq!(hook.temp_filename_addr(), 0x0334 + bin.len() as u16);
    }

    #[test]
    fn hook_writes_vectors() {
        let mut ram = vec![0u8; 0x10000];
        // Initialize original vectors in page 3 with mock values
        ram[SAVE_VECTOR] = 0x11; ram[SAVE_VECTOR + 1] = 0x22;
        ram[LOAD_VECTOR] = 0x33; ram[LOAD_VECTOR + 1] = 0x44;
        ram[OPEN_VECTOR] = 0x55; ram[OPEN_VECTOR + 1] = 0x66;
        ram[CLOSE_VECTOR] = 0x77; ram[CLOSE_VECTOR + 1] = 0x88;
        ram[CHKIN_VECTOR] = 0x99; ram[CHKIN_VECTOR + 1] = 0xAA;
        ram[CKOUT_VECTOR] = 0xBB; ram[CKOUT_VECTOR + 1] = 0xCC;
        ram[CLRCHN_VECTOR] = 0xDD; ram[CLRCHN_VECTOR + 1] = 0xEE;
        ram[CHRIN_VECTOR] = 0x12; ram[CHRIN_VECTOR + 1] = 0x34;
        ram[CHROUT_VECTOR] = 0x56; ram[CHROUT_VECTOR + 1] = 0x78;

        let mut hook = EfSaveHook::new(0x0334, 0xC0);
        hook.hook(&mut ram).unwrap();

        // Check vectors pointed to trampoline entries
        assert_eq!(ram[SAVE_VECTOR] as u16 | ((ram[SAVE_VECTOR + 1] as u16) << 8), 0x0334);
        assert_eq!(ram[LOAD_VECTOR] as u16 | ((ram[LOAD_VECTOR + 1] as u16) << 8), 0x0337);
        assert_eq!(ram[OPEN_VECTOR] as u16 | ((ram[OPEN_VECTOR + 1] as u16) << 8), 0x033A);
        assert_eq!(ram[CLOSE_VECTOR] as u16 | ((ram[CLOSE_VECTOR + 1] as u16) << 8), 0x033D);
        assert_eq!(ram[CHKIN_VECTOR] as u16 | ((ram[CHKIN_VECTOR + 1] as u16) << 8), 0x0340);
        assert_eq!(ram[CKOUT_VECTOR] as u16 | ((ram[CKOUT_VECTOR + 1] as u16) << 8), 0x0343);
        assert_eq!(ram[CLRCHN_VECTOR] as u16 | ((ram[CLRCHN_VECTOR + 1] as u16) << 8), 0x0346);
        assert_eq!(ram[CHRIN_VECTOR] as u16 | ((ram[CHRIN_VECTOR + 1] as u16) << 8), 0x0349);
        assert_eq!(ram[CHROUT_VECTOR] as u16 | ((ram[CHROUT_VECTOR + 1] as u16) << 8), 0x034C);

        // Verify that the original vectors were written to the end of the trampoline binary.
        // The original vectors table starts at (bin_len - 18) within the binary.
        let bin = hook.generate_binary().unwrap();
        let start_offset = bin.len() - 18;
        // Check order of saved vectors: open, close, chkin, ckout, clrchn, chrin, chrout, load, save
        assert_eq!(bin[start_offset] as u16 | ((bin[start_offset + 1] as u16) << 8), 0x6655); // open
        assert_eq!(bin[start_offset + 2] as u16 | ((bin[start_offset + 3] as u16) << 8), 0x8877); // close
        assert_eq!(bin[start_offset + 4] as u16 | ((bin[start_offset + 5] as u16) << 8), 0xAA99); // chkin
        assert_eq!(bin[start_offset + 6] as u16 | ((bin[start_offset + 7] as u16) << 8), 0xCCBB); // ckout
        assert_eq!(bin[start_offset + 8] as u16 | ((bin[start_offset + 9] as u16) << 8), 0xEEDD); // clrchn
        assert_eq!(bin[start_offset + 10] as u16 | ((bin[start_offset + 11] as u16) << 8), 0x3412); // chrin
        assert_eq!(bin[start_offset + 12] as u16 | ((bin[start_offset + 13] as u16) << 8), 0x7856); // chrout
        assert_eq!(bin[start_offset + 14] as u16 | ((bin[start_offset + 15] as u16) << 8), 0x4433); // load
        assert_eq!(bin[start_offset + 16] as u16 | ((bin[start_offset + 17] as u16) << 8), 0x2211); // save
    }

    #[test]
    fn trampoline_with_stash_assembles() {
        let mut hook = EfSaveHook::new(0x0334, 0xC0).with_stash(0x2000, 0x0400);
        let bin = hook.generate_binary().expect("assembles with stash");
        assert!(!bin.is_empty());
        // Verify size is larger than without stashing
        let mut hook_no_stash = EfSaveHook::new(0x0334, 0xC0);
        let bin_no_stash = hook_no_stash.generate_binary().expect("assembles");
        assert!(bin.len() > bin_no_stash.len());
        // Verify that entry points still resolve correctly
        assert_eq!(hook.save_entry(), 0x0334);
        assert_eq!(hook.load_entry(), 0x0337);
        assert_eq!(hook.open_entry(), 0x033A);
        assert_eq!(hook.temp_filename_addr(), 0x0334 + bin.len() as u16);
    }

    #[test]
    fn trampoline_with_blank_assembles() {
        let mut hook = EfSaveHook::new(0x0334, 0xC0).with_blank(true);
        let bin = hook.generate_binary().expect("assembles with blank");
        assert!(!bin.is_empty());
        let mut hook_no_blank = EfSaveHook::new(0x0334, 0xC0);
        let bin_no_blank = hook_no_blank.generate_binary().expect("assembles");
        assert!(bin.len() > bin_no_blank.len());
    }
}
