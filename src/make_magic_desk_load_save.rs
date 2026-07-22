//! LOAD/SAVE vector hooking for Magic Desk file system
//!
//! Magic Desk equivalent of `load_save_hook` (EasyFlash). It intercepts the
//! KERNAL LOAD vector and serves embedded PRG files from cartridge ROM banks.
//!
//! Magic Desk has no ROMH, so the EasyFlash layout (handler @ $A600, metadata
//! @ $B000, filenames @ $B800, all in ROMH) cannot be used. Instead the
//! cartridge reserves bank 0 as a "directory" bank holding:
//!   - boot code      @ $8000
//!   - LOAD handler   @ $8400  (this module)
//!   - metadata       @ $9000
//!   - filenames      @ $9800
//!
//! Only the small trampoline lives in C64 RAM. Its address is computed by the
//! converter with the same mechanism as EasyFlash: auto-placed at $0100 when the
//! snapshot's stack pointer allows it, otherwise in the cassette buffer at $0334,
//! with a manual override via the hook-address option. During
//! a LOAD it banks the directory bank in via $DE00 (bit 7 = 0) and JSRs the
//! handler at $8400. The handler searches metadata, then for each ROM bank of
//! the file it calls the RAM-resident `copy_data` routine, which banks the data
//! bank in, copies its bytes, and banks the directory bank back in. When the
//! handler returns, the trampoline banks the whole cartridge out (bit 7 = 1).
//!
//! Unlike EasyFlash there is no $DE02 control register and no ROMH; banking is
//! done entirely through $DE00. Magic Desk's bit-7 disable is reversible (it just
//! drives EXROM), so the cartridge can be banked in and out repeatedly.
//!
// Copyright (c) 2025-2026 Tommy Olsen
// Licensed under the MIT License.

use crate::asm_wrapper::assemble_to_bytes;

// KERNAL vectors on page 3
pub const LOAD_VECTOR: usize = 0x0330;
pub const SAVE_VECTOR: usize = 0x0332;

/// LOAD handler entry point inside the directory bank (ROML window).
pub const HANDLER_ADDRESS: u16 = 0x8400;

/// Metadata and filename tables inside the directory bank (ROML window).
pub const METADATA_ADDRESS: u16 = 0x9000;
pub const FILENAMES_ADDRESS: u16 = 0x9800;

/// The cartridge bank that holds boot code + directory (handler/metadata/names).
/// Fixed at 0 so the trampoline never needs to know the (size-dependent) restore
/// bank count: bank 0 is always present and always selected on reset.
pub const DIRECTORY_BANK: u8 = 0;

/// Fallback trampoline address (the cassette buffer) when the caller does not
/// supply one. Like EasyFlash, the converter normally picks the address from the
/// snapshot's stack pointer: $0100 when SP >= 242 (page 1 content, including a
/// trampoline there, is preserved and restored via PatchMem blocks 1-8),
/// otherwise $0334.
pub const DEFAULT_TRAMPOLINE_ADDR: u16 = 0x0334;

/// Manages LOAD/SAVE vector hooking for a Magic Desk cartridge file system.
pub struct MagicDeskLoadSaveHook {
    has_files: bool,
    trampoline_address: u16,
    dir_bank: u8,
    copy_data_addr: u16,
    save_trampoline_addr: u16,
    temp_filename_addr: u16,
    trampoline_binary: Vec<u8>,
}

impl MagicDeskLoadSaveHook {
    /// Create a new Magic Desk LOAD/SAVE hook manager.
    pub fn new(has_files: bool, trampoline_address: Option<u16>) -> Self {
        Self {
            has_files,
            trampoline_address: trampoline_address.unwrap_or(DEFAULT_TRAMPOLINE_ADDR),
            dir_bank: DIRECTORY_BANK,
            copy_data_addr: 0,
            save_trampoline_addr: 0,
            temp_filename_addr: 0,
            trampoline_binary: Vec::new(),
        }
    }

    /// Get the trampoline address.
    pub fn get_trampoline_address(&self) -> u16 {
        self.trampoline_address
    }

    /// Generate trampoline assembly code.
    ///
    /// The trampoline is the only part that lives in C64 RAM. It hooks the LOAD
    /// vector, copies the requested filename to a temp area, banks in the
    /// directory bank, and calls the handler at $8400. `copy_data` is also here
    /// because the byte-copy loop must run from RAM: switching $DE00 to a data
    /// bank while executing from the cartridge window would swap the running code
    /// out from under the CPU.
    fn generate_trampoline_asm(&self, temp_addr: u16) -> String {
        format!(
            r#"*=${trampoline:04X}

load_trampoline:
    STA $93              ; save LOAD/VERIFY flag (KERNAL semantics)
    SEI
    LDA $01
    STA port_01_save

    ; Copy requested filename to temp area (readable while cart is banked in)
    LDY $B7
    BEQ no_filename
    DEY
copy_filename_loop:
    LDA ($BB),Y
    STA ${temp:04X},Y
    DEY
    BPL copy_filename_loop
no_filename:

    ; ROM + I/O visible, bank in directory bank, call handler in cart
    LDA #$37
    STA $01
    LDA #${dir_bank:02X}
    STA $DE00            ; directory bank in (bit 7 = 0 -> cart enabled)
    JSR ${handler:04X}   ; handler @ $8400 in directory bank

    STX $AE
    STY $AF
    PHA
    PHP
    LDA #$80
    STA $DE00            ; bank cartridge OUT (bit 7 = 1)
    LDA port_01_save
    STA $01
    PLP
    PLA
    LDX $AE
    LDY $AF
    CLI
    RTS

save_trampoline:
    CLC
    RTS

copy_data:
    STX $DE00            ; bank in data bank X (bit 7 = 0); I/O still on from caller
    LDA #$33
    STA $01             ; ROML readable, RAM writable

copy_loop:
    LDA $A3
    CMP $90
    BNE not_done
    LDA $A4
    CMP $91
    BEQ copy_done

not_done:
    LDY #$00
    LDA ($A3),Y
    STA ($AE),Y
    INC $A3
    BNE no_carry_src
    INC $A4

    ; Check for $A000 (bank boundary)
    LDA $A4
    CMP #$A0
    BCS bank_boundary_reached

no_carry_src:
    INC $AE
    BNE no_carry_dst
    INC $AF

no_carry_dst:
    JMP copy_loop

bank_boundary_reached:
    ; Increment dest pointer for the byte just copied
    INC $AE
    BNE bank_boundary_update
    INC $AF

bank_boundary_update:
    ; Update $90/$91 to actual end address ($A3/$A4)
    ; so the handler sees the stop at a bank boundary
    LDA $A3
    STA $90
    LDA $A4
    STA $91

copy_done:
    LDA #$37
    STA $01             ; I/O on
    LDA #${dir_bank:02X}
    STA $DE00            ; back to directory bank so handler page stays valid
    RTS

port_01_save:
    .byte $00
"#,
            trampoline = self.trampoline_address,
            temp = temp_addr,
            dir_bank = self.dir_bank,
            handler = HANDLER_ADDRESS,
        )
    }

    /// Generate trampoline binary code.
    pub fn generate_trampoline_binary(&mut self) -> Result<Vec<u8>, String> {
        if !self.has_files {
            return Ok(Vec::new());
        }

        // First pass: assemble with estimated temp address (size is independent
        // of the temp address value, only its bytes change).
        let first_pass_asm = self.generate_trampoline_asm(self.trampoline_address + 0xF0);
        let first_pass_bytes = assemble_to_bytes(&first_pass_asm)?;

        // Temp filename area goes right after the code.
        self.temp_filename_addr = self.trampoline_address + first_pass_bytes.len() as u16;

        // Second pass with correct temp address.
        let final_asm = self.generate_trampoline_asm(self.temp_filename_addr);
        let bytes = assemble_to_bytes(&final_asm)?;

        let final_bytes = if bytes.len() != first_pass_bytes.len() {
            self.temp_filename_addr = self.trampoline_address + bytes.len() as u16;
            let retry_asm = self.generate_trampoline_asm(self.temp_filename_addr);
            let retry_bytes = assemble_to_bytes(&retry_asm)?;
            if retry_bytes.len() != bytes.len() {
                return Err(format!(
                    "Magic Desk trampoline size unstable: {} vs {}",
                    bytes.len(),
                    retry_bytes.len()
                ));
            }
            retry_bytes
        } else {
            bytes
        };

        self.find_addresses(&final_bytes)?;
        self.trampoline_binary = final_bytes.clone();
        Ok(final_bytes)
    }

    /// Find routine addresses in the assembled trampoline.
    ///
    /// `copy_data` begins with the only `STX $DE00` (8E 00 DE) in the trampoline,
    /// and `save_trampoline` (CLC, RTS) is emitted immediately before it.
    fn find_addresses(&mut self, bytes: &[u8]) -> Result<(), String> {
        let mut copy_data_offset = None;
        for i in 0..bytes.len().saturating_sub(2) {
            if bytes[i] == 0x8E && bytes[i + 1] == 0x00 && bytes[i + 2] == 0xDE {
                copy_data_offset = Some(i);
                break;
            }
        }

        let copy_data_offset = copy_data_offset
            .ok_or_else(|| "Failed to find copy_data (STX $DE00) in trampoline".to_string())?;

        // save_trampoline (CLC RTS) is the two bytes immediately before copy_data.
        if copy_data_offset < 2
            || bytes[copy_data_offset - 2] != 0x18
            || bytes[copy_data_offset - 1] != 0x60
        {
            return Err("Failed to locate save_trampoline (CLC RTS) before copy_data".to_string());
        }

        self.copy_data_addr = self.trampoline_address + copy_data_offset as u16;
        self.save_trampoline_addr = self.trampoline_address + (copy_data_offset - 2) as u16;
        Ok(())
    }

    /// Hook LOAD and SAVE vectors in RAM (called before compression).
    pub fn hook_load_and_save(&mut self, ram: &mut [u8]) -> Result<(), String> {
        if !self.has_files {
            return Ok(());
        }

        let trampoline_code = self.generate_trampoline_binary()?;
        let addr = self.trampoline_address as usize;

        // Trampoline + temp filename (16 bytes reserved for the longest legal
        // filename) must stay inside its page-sized home when placed in the low
        // fixed areas: page 1 (below the restored stack) or the cassette buffer
        // (ends at $03FF). Elsewhere only the RAM bounds apply.
        let end = addr + trampoline_code.len() + 16;
        if addr < 0x0200 && end > 0x0200 {
            return Err(format!(
                "Magic Desk trampoline ({} bytes) does not fit in page 1 at ${:04X}",
                trampoline_code.len(),
                self.trampoline_address
            ));
        }
        if (0x0200..0x0400).contains(&addr) && end > 0x0400 {
            return Err(format!(
                "Magic Desk trampoline ({} bytes) does not fit in the cassette buffer at ${:04X}",
                trampoline_code.len(),
                self.trampoline_address
            ));
        }
        if end > ram.len() {
            return Err("Trampoline code exceeds RAM bounds".to_string());
        }

        ram[addr..addr + trampoline_code.len()].copy_from_slice(&trampoline_code);

        // Hook LOAD vector at $0330/$0331
        ram[LOAD_VECTOR] = (self.trampoline_address & 0xFF) as u8;
        ram[LOAD_VECTOR + 1] = ((self.trampoline_address >> 8) & 0xFF) as u8;

        // Hook SAVE vector at $0332/$0333
        ram[SAVE_VECTOR] = (self.save_trampoline_addr & 0xFF) as u8;
        ram[SAVE_VECTOR + 1] = ((self.save_trampoline_addr >> 8) & 0xFF) as u8;

        Ok(())
    }

    /// Generate the LOAD handler assembly for the directory bank @ $8400.
    ///
    /// This is the EasyFlash $A600 handler ported to Magic Desk: the metadata
    /// table is at $9000 (page $90, not $B0) and the filename table at $9800
    /// (so the search terminates at page $98, not $B8). It calls the RAM-resident
    /// `copy_data` routine to move each bank's bytes.
    fn generate_handler_asm(&self) -> String {
        let copy_data_addr = format!("{:04X}", self.copy_data_addr);
        let temp_filename = format!("{:04X}", self.temp_filename_addr);

        format!(
            r#"*=$8400

; Metadata format @ $9000 (16 bytes per entry):
;   +0: Filename pointer (2 bytes)
;   +2: Bank list (8 bytes, $00 = end)
;   +10: Start offset (2 bytes)
;   +12: File length (2 bytes)
;   +14: Load address (2 bytes)

load_handler:
    LDA $DD0D
    LDA $DC0D

    LDA #$00
    STA $A3
    STA $A4
    LDA #$90
    STA $A4

search_loop:
    LDY #$00
    LDA ($A3),Y
    STA $90
    INY
    LDA ($A3),Y
    STA $91
    ORA $90
    BNE metadata_not_empty
    JMP file_not_found
metadata_not_empty:

    LDX $B7
    BNE check_filename
    JMP filename_match
check_filename:
    ; Null-terminate the filename copy
    LDA #$00
    STA ${temp},X

    ; Wildcards: * = match all, ? = match one char
    ; Space matches space or end-of-filename (simulates disk padding)

    LDY #$00
compare_filename_loop:
    CPY $B7
    BEQ pattern_exhausted

    LDA ${temp},Y

    CMP #$2A
    BEQ filename_match

    CMP #$3F
    BEQ wildcard_question

    CMP #$20
    BEQ space_in_pattern

    ; Case-insensitive: convert PETSCII lowercase to uppercase
    CMP #$C1
    BCC check_ascii_lower
    CMP #$DB
    BCS check_ascii_lower
    SEC
    SBC #$80
    JMP compare_chars

check_ascii_lower:
    CMP #$61
    BCC compare_chars
    CMP #$7B
    BCS compare_chars
    SEC
    SBC #$20

compare_chars:
    CMP ($90),Y
    BEQ char_matches
    JMP next_entry

space_in_pattern:
    LDA ($90),Y
    BEQ space_matches_end
    CMP #$20
    BEQ char_matches
    JMP next_entry

space_matches_end:
check_remaining_spaces:
    INY
    CPY $B7
    BEQ filename_match
    LDA ${temp},Y
    CMP #$20
    BEQ check_remaining_spaces
    CMP #$2A
    BEQ filename_match
    CMP #$3F
    BEQ check_remaining_spaces
    JMP next_entry

wildcard_question:
    LDA ($90),Y
    BNE char_matches
    JMP next_entry

char_matches:
    INY
    JMP compare_filename_loop

pattern_exhausted:
    JMP filename_match

filename_match:
    LDA $A3
    STA $A7
    LDA $A4
    STA $A8

    ; SA=0: use file address, SA=1: use $C3/$C4
    LDA $93
    BEQ use_file_addr

    LDA $C3
    STA $AE
    LDA $C4
    STA $AF
    JMP got_dest_addr

use_file_addr:
    LDY #$0E
    LDA ($A7),Y
    STA $AE
    INY
    LDA ($A7),Y
    STA $AF

got_dest_addr:
    LDY #$0C
    LDA ($A7),Y
    STA $93
    INY
    LDA ($A7),Y
    STA $94

    LDY #$0A
    LDA ($A7),Y
    STA $A5
    INY
    LDA ($A7),Y
    STA $A6

    LDY #$02

load_bank_loop:
    LDA ($A7),Y
    BEQ load_complete

    TAX
    STY $92

    CPY #$02
    BNE not_first_bank

    LDA $A5
    CLC
    ADC #$00
    STA $A3
    LDA $A6
    ADC #$80
    STA $A4
    JMP calc_end_addr

not_first_bank:
    LDA #$00
    STA $A3
    LDA #$80
    STA $A4

calc_end_addr:
    ; end = src + remaining
    LDA $A3
    CLC
    ADC $93
    STA $90
    LDA $A4
    ADC $94
    STA $91

    ; clamp end to bank boundary
    LDA $91
    CMP #$A0
    BCC end_ok
    BNE clamp_end
    LDA $90
    BEQ end_ok
clamp_end:
    LDA #$00
    STA $90
    LDA #$A0
    STA $91
end_ok:

do_copy:
    LDA $A3
    STA $95
    LDA $A4
    STA $96

    LDY #$07
    JSR ${copy_data}

    LDA $90
    SEC
    SBC $95
    STA $A3
    LDA $91
    SBC $96
    STA $A4

    LDA $93
    SEC
    SBC $A3
    STA $93
    LDA $94
    SBC $A4
    STA $94

    LDA $93
    ORA $94
    BEQ load_complete

    LDY $92
    INY
    CPY #$0A
    BCS load_complete
    JMP load_bank_loop

load_complete:
    LDA #$00
    STA $90
    CLC
    LDX $AE
    LDY $AF
    RTS

next_entry:
    LDA $A3
    CLC
    ADC #$10
    STA $A3
    BCC no_carry
    INC $A4

no_carry:
    LDA $A4
    CMP #$98
    BCS file_not_found
    JMP search_loop

file_not_found:
    SEC
    LDX #$00
    LDY #$00
    RTS
"#,
            temp = temp_filename,
            copy_data = copy_data_addr,
        )
    }

    /// Generate the LOAD handler code (raw binary starting at $8400).
    pub fn generate_handler_rom_code(&mut self) -> Result<Vec<u8>, String> {
        if !self.has_files {
            return Ok(Vec::new());
        }

        // Ensure trampoline addresses are calculated first.
        if self.copy_data_addr == 0 || self.temp_filename_addr == 0 {
            self.generate_trampoline_binary()?;
        }

        let asm = self.generate_handler_asm();
        assemble_to_bytes(&asm)
    }

    /// Get the copy_data address (RAM).
    pub fn get_copy_data_addr(&self) -> u16 {
        self.copy_data_addr
    }

    /// Get the temp filename address (RAM).
    pub fn get_temp_filename_addr(&self) -> u16 {
        self.temp_filename_addr
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trampoline_assembles_and_addresses_resolve() {
        let mut hook = MagicDeskLoadSaveHook::new(true, None);
        let bytes = hook.generate_trampoline_binary().expect("trampoline assembles");
        assert!(!bytes.is_empty());

        // Trampoline + temp filename must fit in the cassette buffer.
        assert!(DEFAULT_TRAMPOLINE_ADDR as usize + bytes.len() + 16 <= 0x0400);

        // copy_data starts at the unique STX $DE00; save_trampoline (CLC RTS) sits
        // immediately before it.
        assert!(hook.get_copy_data_addr() > DEFAULT_TRAMPOLINE_ADDR);
        assert_eq!(hook.save_trampoline_addr, hook.get_copy_data_addr() - 2);
        let cd = (hook.get_copy_data_addr() - DEFAULT_TRAMPOLINE_ADDR) as usize;
        assert_eq!(&bytes[cd..cd + 3], &[0x8E, 0x00, 0xDE]); // STX $DE00
        assert_eq!(&bytes[cd - 2..cd], &[0x18, 0x60]); // CLC RTS
    }

    #[test]
    fn handler_assembles() {
        let mut hook = MagicDeskLoadSaveHook::new(true, None);
        let handler = hook.generate_handler_rom_code().expect("handler assembles");
        assert!(!handler.is_empty());
        // Must fit between $8400 and the metadata table at $9000.
        assert!(handler.len() <= (METADATA_ADDRESS - HANDLER_ADDRESS) as usize);
    }

    #[test]
    fn no_files_emits_nothing() {
        let mut hook = MagicDeskLoadSaveHook::new(false, None);
        assert!(hook.generate_trampoline_binary().unwrap().is_empty());
        assert!(hook.generate_handler_rom_code().unwrap().is_empty());
    }
}
