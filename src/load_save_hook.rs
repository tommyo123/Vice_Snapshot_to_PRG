//! LOAD/SAVE vector hooking for EasyFlash file system
//!
//! Implements KERNAL LOAD/SAVE hooks that intercept file operations and
//! serve files from EasyFlash ROM banks with metadata at $B000+
//!
// Copyright (c) 2025 Tommy Olsen
// Licensed under the MIT License.

use crate::asm_wrapper::assemble_to_bytes;

// KERNAL vectors on page 3
pub const LOAD_VECTOR: usize = 0x0330;
pub const SAVE_VECTOR: usize = 0x0332;

// ROMH addresses for LOAD/SAVE code (in bank 0 ROMH @ $A000-$BFFF in 16K mode)
pub const ROMH_LOAD_SAVE_CODE: u16 = 0xA600;

// Metadata and filenames in ROMH (in 16K mode)
pub const METADATA_ADDRESS: u16 = 0xB000;
pub const FILENAMES_ADDRESS: u16 = 0xB800;

// Trampoline addresses
pub const TRAMPOLINE_PAGE1: u16 = 0x0100;
pub const TRAMPOLINE_PAGE3: u16 = 0x0334;

/// Default trampoline address
pub const DEFAULT_TRAMPOLINE_ADDR: u16 = 0x0100;

/// Manages LOAD/SAVE vector hooking for EasyFlash cartridge file system
pub struct LoadSaveHook {
    #[allow(dead_code)]
    stack_pointer: u8,
    has_files: bool,
    trampoline_address: u16,
    set_bank_addr: u16,
    copy_data_addr: u16,
    save_trampoline_addr: u16,
    temp_filename_addr: u16,
    trampoline_binary: Vec<u8>,
}

impl LoadSaveHook {
    /// Create a new LOAD/SAVE hook manager
    ///
    /// trampoline_address: Where to place the trampoline code
    /// - $0100: Safe when SP >= 242 (stack won't collide)
    /// - $0334: Safe when SP < 242 (avoids stack area)
    ///
    /// The caller (convert_snapshot_crt) determines the address based on SP.
    pub fn new(stack_pointer: u8, has_files: bool, trampoline_address: Option<u16>) -> Self {
        // Use provided address, or default to $0334 if not specified
        let addr = trampoline_address.unwrap_or(TRAMPOLINE_PAGE3);

        Self {
            stack_pointer,
            has_files,
            trampoline_address: addr,
            set_bank_addr: 0,
            copy_data_addr: 0,
            save_trampoline_addr: 0,
            temp_filename_addr: 0,
            trampoline_binary: Vec::new(),
        }
    }

    /// Get the trampoline address
    pub fn get_trampoline_address(&self) -> u16 {
        self.trampoline_address
    }

    /// Generate trampoline assembly code
    fn generate_trampoline_asm(&self, temp_addr: u16) -> String {
        format!(
            r#"*=${:04X}

load_trampoline:
    STA $93
    SEI
    LDA $01
    STA restore_memmap+1

    ; Copy filename to temp area
    LDY $B7
    BEQ no_filename
    DEY
copy_filename_loop:
    LDA ($BB),Y
    STA ${:04X},Y
    DEY
    BPL copy_filename_loop
no_filename:

    LDA #$37
    STA $01
    LDX #$00
    LDY #$07
    JSR set_bank
    JSR $A600

    STX $AE
    STY $AF
    PHA
    PHP
    LDA #$04
    STA $DE02
    LDA #$37
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

set_bank:
    STX $DE00
    STY $DE02
    RTS

copy_data:
    JSR set_bank
    LDA #$33
    STA $01

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

    ; Check if we've reached $A000 (bank boundary)
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
    ; Increment dest pointer for the last byte we just copied
    INC $AE
    BNE bank_boundary_update
    INC $AF

bank_boundary_update:
    ; Update $90/$91 to actual end address ($A3/$A4)
    ; so ROMH code knows we stopped at bank boundary
    LDA $A3
    STA $90
    LDA $A4
    STA $91

copy_done:
    LDA #$37
    STA $01
    LDX #$00
    LDY #$07
    JSR set_bank

restore_memmap:
    RTS
"#,
            self.trampoline_address, temp_addr
        )
    }

    /// Generate trampoline binary code
    pub fn generate_trampoline_binary(&mut self) -> Result<Vec<u8>, String> {
        if !self.has_files {
            return Ok(Vec::new());
        }

        // First pass: assemble with estimated temp address
        let first_pass_asm = self.generate_trampoline_asm(self.trampoline_address + 0xF0);
        let first_pass_bytes = assemble_to_bytes(&first_pass_asm)?;

        // Calculate actual temp filename address
        let code_end_addr = self.trampoline_address + first_pass_bytes.len() as u16;
        self.temp_filename_addr = code_end_addr;

        // Second pass with correct temp address
        let final_asm = self.generate_trampoline_asm(self.temp_filename_addr);
        let bytes = assemble_to_bytes(&final_asm)?;

        // If size changed, do another pass
        let final_bytes = if bytes.len() != first_pass_bytes.len() {
            self.temp_filename_addr = self.trampoline_address + bytes.len() as u16;
            let retry_asm = self.generate_trampoline_asm(self.temp_filename_addr);
            let retry_bytes = assemble_to_bytes(&retry_asm)?;
            if retry_bytes.len() != bytes.len() {
                return Err(format!(
                    "Code size unstable: {} vs {}",
                    bytes.len(),
                    retry_bytes.len()
                ));
            }
            retry_bytes
        } else {
            bytes
        };

        // Find routine addresses in assembled code
        self.find_addresses(&final_bytes)?;

        // Store the binary for later use
        self.trampoline_binary = final_bytes.clone();

        Ok(final_bytes)
    }

    /// Find routine addresses in assembled code
    fn find_addresses(&mut self, bytes: &[u8]) -> Result<(), String> {
        // Find set_bank: STX $DE00 (8E 00 DE) STY $DE02 (8C 02 DE)
        for i in 0..bytes.len().saturating_sub(5) {
            if bytes[i] == 0x8E
                && bytes[i + 1] == 0x00
                && bytes[i + 2] == 0xDE
                && bytes[i + 3] == 0x8C
                && bytes[i + 4] == 0x02
                && bytes[i + 5] == 0xDE
            {
                self.set_bank_addr = self.trampoline_address + i as u16;
                break;
            }
        }

        self.copy_data_addr = self.set_bank_addr + 7;

        // Find save_trampoline: CLC (18) RTS (60)
        let set_bank_offset = (self.set_bank_addr - self.trampoline_address) as usize;
        for i in (0..set_bank_offset).rev() {
            if bytes[i] == 0x18 && i + 1 < bytes.len() && bytes[i + 1] == 0x60 {
                self.save_trampoline_addr = self.trampoline_address + i as u16;
                break;
            }
        }

        if self.set_bank_addr == 0 || self.copy_data_addr == 0 || self.save_trampoline_addr == 0 {
            return Err("Failed to find routine addresses in assembled code".to_string());
        }

        Ok(())
    }

    /// Hook LOAD and SAVE vectors in RAM
    pub fn hook_load_and_save(&mut self, ram: &mut [u8]) -> Result<(), String> {
        if !self.has_files {
            return Ok(());
        }

        let trampoline_code = self.generate_trampoline_binary()?;
        let addr = self.trampoline_address as usize;

        if addr + trampoline_code.len() > ram.len() {
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

    /// Generate ROMH handler assembly code
    fn generate_romh_handler_asm(&self) -> String {
        let copy_data_addr = format!("{:04X}", self.copy_data_addr);
        let temp_filename = format!("{:04X}", self.temp_filename_addr);

        format!(
            r#"*=$A600

; Metadata format @ $B000 (16 bytes per entry):
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
    LDA #$B0
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
    STA ${},X

    ; Wildcards: * = match all, ? = match one char
    ; Space matches space or end-of-filename (simulates disk padding)

    LDY #$00
compare_filename_loop:
    CPY $B7
    BEQ pattern_exhausted

    LDA ${},Y

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
    LDA ${},Y
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
    JSR ${}

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
    CMP #$B8
    BCS file_not_found
    JMP search_loop

file_not_found:
    SEC
    LDX #$00
    LDY #$00
    RTS
"#,
            temp_filename, temp_filename, temp_filename, copy_data_addr
        )
    }

    /// Generate LOAD/SAVE handler code for ROMH @ $A600
    pub fn generate_load_save_rom_code(&mut self) -> Result<Vec<u8>, String> {
        if !self.has_files {
            return Ok(Vec::new());
        }

        // Ensure addresses are calculated
        if self.copy_data_addr == 0 || self.temp_filename_addr == 0 {
            self.generate_trampoline_binary()?;
        }

        let asm = self.generate_romh_handler_asm();
        assemble_to_bytes(&asm)
    }

    /// Get copy_data address (needed for ROMH handler)
    pub fn get_copy_data_addr(&self) -> u16 {
        self.copy_data_addr
    }

    /// Get temp filename address
    pub fn get_temp_filename_addr(&self) -> u16 {
        self.temp_filename_addr
    }

    /// Get the generated trampoline binary code
    pub fn get_trampoline_binary(&self) -> &[u8] {
        &self.trampoline_binary
    }
}
