//! CRT ROM code generator
//!
//! Generates restore code that starts at $0340 (called from ROMH @ $E000).
//! RAM lzsa is already copied to end of memory by ROMH, so we don't include it here.
//!
// Copyright (c) 2025 Tommy Olsen
// Licensed under the MIT License.

use std::fs;
use crate::asm_wrapper::assemble_to_bytes;
use crate::config::Config;

/// CRT restore code generator
pub struct MakeCRTAsm {
    color_lzsa: Vec<u8>,
    vic_lzsa: Vec<u8>,
    sid_lzsa: Vec<u8>,
    cia1_bin: Vec<u8>,
    cia2_bin: Vec<u8>,
    zp_lzsa: Vec<u8>,
    block9_addr: u16,
    f8_ff_data: [u8; 8],
    #[allow(dead_code)]
    config: Config,
    relocated_size: usize,
    ram_lzsa_size: usize,
    restore_code_size: usize,
    load_save_code_size: usize,
}

impl MakeCRTAsm {
    pub fn new(
        color_lzsa_path: &str,
        vic_lzsa_path: &str,
        sid_lzsa_path: &str,
        cia1_bin_path: &str,
        cia2_bin_path: &str,
        zp_lzsa_path: &str,
        block9_addr: u16,
        f8_ff_data: [u8; 8],
        config: &Config,
        relocated_size: usize,
        ram_lzsa_size: usize,
        restore_code_size: usize,
        load_save_code_size: usize,
    ) -> Result<Self, String> {
        let cia1_bin = fs::read(cia1_bin_path)
            .map_err(|e| format!("Failed to read CIA1 file: {}", e))?;
        let cia2_bin = fs::read(cia2_bin_path)
            .map_err(|e| format!("Failed to read CIA2 file: {}", e))?;

        if cia1_bin.len() != 20 {
            return Err(format!("CIA1 file must be 20 bytes, got {}", cia1_bin.len()));
        }
        if cia2_bin.len() != 20 {
            return Err(format!("CIA2 file must be 20 bytes, got {}", cia2_bin.len()));
        }

        Ok(Self {
            color_lzsa: fs::read(color_lzsa_path)
                .map_err(|e| format!("Failed to read color LZSA: {}", e))?,
            vic_lzsa: fs::read(vic_lzsa_path)
                .map_err(|e| format!("Failed to read VIC LZSA: {}", e))?,
            sid_lzsa: fs::read(sid_lzsa_path)
                .map_err(|e| format!("Failed to read SID LZSA: {}", e))?,
            cia1_bin,
            cia2_bin,
            zp_lzsa: fs::read(zp_lzsa_path)
                .map_err(|e| format!("Failed to read ZP LZSA: {}", e))?,
            block9_addr,
            f8_ff_data,
            config: config.clone(),
            relocated_size,
            ram_lzsa_size,
            restore_code_size,
            load_save_code_size,
        })
    }

    /// Generate CRT restore code binary (to be placed at $0340 in RAM)
    pub fn generate_restore_code_binary(&self) -> Result<Vec<u8>, String> {
        let main_asm = self.generate_main_code_asm6502();
        assemble_to_bytes(&main_asm)
    }

    /// Generate data copying code
    fn generate_data_copy_code(&self, ram_end_data_start: usize, end_data_size: usize) -> String {
        let roml_bank_start = 0x8000usize;
        let roml_bank_size = 8192usize;
        let roml_end_data_start = roml_bank_start + self.restore_code_size + self.load_save_code_size;

        let source_bank = (roml_end_data_start - roml_bank_start) / roml_bank_size;
        let source_hi = (roml_end_data_start >> 8) & 0xFF;
        let source_lo = roml_end_data_start & 0xFF;
        let ram_dest_hi = (ram_end_data_start >> 8) & 0xFF;
        let ram_dest_lo = ram_end_data_start & 0xFF;

        let disable_mode = if self.load_save_code_size > 0 {
            "; Use $03 (allow re-enable later for LOAD/SAVE)\n    LDA #$03"
        } else {
            "; Use $04 (full disable - original behavior)\n    LDA #$04"
        };

        format!(
            r#"    ; =============================================================================
    ; DIRECT copy from ROML to final position (NO temp buffer)
    ; =============================================================================

    LDA #$37
    STA $01

    LDA #${:02X}
    STA $F7
    STA EASYFLASH_ROML

    LDA #$33
    STA $01

    LDA #${:02X}
    STA $FC
    LDA #${:02X}
    STA $FB

    LDA #${:02X}
    STA $FE
    LDA #${:02X}
    STA $FD

    LDA #${:02X}
    STA $F8
    LDA #${:02X}
    STA $F9

copy_loop:
    LDA $F8
    BNE copy_byte
    LDA $F9
    BEQ copy_done

copy_byte:
    LDY #$00
    LDA ($FB),Y
    STA ($FD),Y

    INC $FB
    BNE skip_src_hi
    INC $FC
    LDA $FC
    CMP #$A0
    BNE skip_src_hi
    LDA #$37
    STA $01
    INC $F7
    LDA $F7
    STA EASYFLASH_ROML
    LDA #$33
    STA $01
    LDA #$80
    STA $FC
    LDA #$00
    STA $FB
skip_src_hi:

    INC $FD
    BNE skip_dst_hi
    INC $FE
skip_dst_hi:

    LDA $F9
    BNE dec_lo
    DEC $F8
dec_lo:
    DEC $F9

    JMP copy_loop

copy_done:
    LDA #$37
    STA $01

    {}
    STA $DE02

    LDA $DC0D
    LDA $DD0D
    LDA #$FF
    STA $D019

    LDA #$34
    STA $01
"#,
            source_bank,
            source_hi,
            source_lo,
            ram_dest_hi,
            ram_dest_lo,
            (end_data_size >> 8) & 0xFF,
            end_data_size & 0xFF,
            disable_mode
        )
    }

    fn generate_main_code_asm6502(&self) -> String {
        let ram_data_size = self.relocated_size + self.ram_lzsa_size;
        let end_data_start = 0x10000 - ram_data_size;
        let ram_lzsa_start = end_data_start + self.relocated_size;

        let data_copy_code = self.generate_data_copy_code(end_data_start, ram_data_size);

        // Generate inline data bytes
        let color_data = self.format_bytes(&self.color_lzsa);
        let vic_data = self.format_bytes(&self.vic_lzsa);
        let sid_data = self.format_bytes(&self.sid_lzsa);
        let cia1_data = self.format_bytes(&self.cia1_bin);
        let cia2_data = self.format_bytes(&self.cia2_bin);
        let zp_data = self.format_bytes(&self.zp_lzsa);
        let f8_ff_bytes = self.format_bytes(&self.f8_ff_data);

        format!(
            r#"; C64 EasyFlash CRT Snapshot Restore Code
; Entry point: $0340 (called from minimal trampoline @ $0100)
*=$0340

EASYFLASH_ROML = $DE00
EASYFLASH_CONTROL = $DE02

RELOCATED_SIZE = {}
RAM_DATA_SIZE = {}
END_DATA_START = ${:04X}
RAM_LZSA_START = ${:04X}

LZSA_SRC_LO = $FC
LZSA_SRC_HI = $FD
LZSA_DST_LO = $FE
LZSA_DST_HI = $FF
LZSA_CMDBUF = $F9
LZSA_WINPTR = $FA
LZSA_OFFSET = $FA

start:
    SEI
    CLD

{}

    LDA #$35
    STA $01

    LDA #$2F
    STA $00

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

    LDA #<color_data
    STA LZSA_SRC_LO
    LDA #>color_data
    STA LZSA_SRC_HI
    LDA #$00
    STA LZSA_DST_LO
    LDA #$D8
    STA LZSA_DST_HI
    JSR decompress_lzsa1

    LDA #<vic_data
    STA LZSA_SRC_LO
    LDA #>vic_data
    STA LZSA_SRC_HI
    LDA #$00
    STA LZSA_DST_LO
    LDA #$D0
    STA LZSA_DST_HI
    JSR decompress_lzsa1

    LDA $D011
    STA $D011
    LDA $D012
    STA $D012

    LDA #$00
    STA $D01A

    LDA #$FF
    STA $D019

    LDA #<sid_data
    STA LZSA_SRC_LO
    LDA #>sid_data
    STA LZSA_SRC_HI
    LDA #$00
    STA LZSA_DST_LO
    LDA #$D4
    STA LZSA_DST_HI
    JSR decompress_lzsa1

; CIA1 Setup
    LDA #$7F
    STA $DC0D
    LDA #$00
    STA $DC0E
    STA $DC0F

    LDA cia1_data+2
    STA $DC02
    LDA cia1_data+3
    STA $DC03
    LDA cia1_data+0
    STA $DC00
    LDA cia1_data+1
    STA $DC01

    LDA cia1_data+16
    STA $DC04
    LDA cia1_data+17
    STA $DC05
    LDA #$10
    STA $DC0E
    LDA #$00
    STA $DC0E
    LDA cia1_data+4
    STA $DC04
    LDA cia1_data+5
    STA $DC05

    LDA cia1_data+18
    STA $DC06
    LDA cia1_data+19
    STA $DC07
    LDA #$10
    STA $DC0F
    LDA #$00
    STA $DC0F
    LDA cia1_data+6
    STA $DC06
    LDA cia1_data+7
    STA $DC07

    LDA cia1_data+11
    STA $DC0B
    LDA cia1_data+10
    STA $DC0A
    LDA cia1_data+9
    STA $DC09
    LDA cia1_data+8
    STA $DC08

    LDA cia1_data+12
    STA $DC0C
    LDA cia1_data+14
    AND #$FE
    STA $DC0E
    LDA cia1_data+15
    AND #$FE
    STA $DC0F

; CIA2 Setup
    LDA #$7F
    STA $DD0D
    LDA #$00
    STA $DD0E
    STA $DD0F

    LDA cia2_data+2
    STA $DD02
    LDA cia2_data+3
    STA $DD03
    LDA cia2_data+0
    STA $DD00
    LDA cia2_data+1
    STA $DD01

    LDA cia2_data+16
    STA $DD04
    LDA cia2_data+17
    STA $DD05
    LDA #$10
    STA $DD0E
    LDA #$00
    STA $DD0E
    LDA cia2_data+4
    STA $DD04
    LDA cia2_data+5
    STA $DD05

    LDA cia2_data+18
    STA $DD06
    LDA cia2_data+19
    STA $DD07
    LDA #$10
    STA $DD0F
    LDA #$00
    STA $DD0F
    LDA cia2_data+6
    STA $DD06
    LDA cia2_data+7
    STA $DD07

    LDA cia2_data+11
    STA $DD0B
    LDA cia2_data+10
    STA $DD0A
    LDA cia2_data+9
    STA $DD09
    LDA cia2_data+8
    STA $DD08

    LDA cia2_data+12
    STA $DD0C
    LDA cia2_data+14
    AND #$FE
    STA $DD0E
    LDA cia2_data+15
    AND #$FE
    STA $DD0F

; Decompress Zero Page
    LDA #<zp_data
    STA LZSA_SRC_LO
    LDA #>zp_data
    STA LZSA_SRC_HI
    LDA #$02
    STA LZSA_DST_LO
    LDA #$00
    STA LZSA_DST_HI
    JSR decompress_lzsa1

    LDA #$00
    STA $F8
    STA $F9
    STA $FA
    STA $FB

    LDX #<END_DATA_START
    LDY #>END_DATA_START
    STX $FE
    STY $FF
    LDY #$00
CPLP:
    LDA ($FE),Y
    STA $0100,Y
    INY
    CPY #<RELOCATED_SIZE
    BNE CPLP

    LDA #<RAM_LZSA_START
    STA LZSA_SRC_LO
    LDA #>RAM_LZSA_START
    STA LZSA_SRC_HI

    LDA #$00
    STA LZSA_DST_LO
    LDA #$02
    STA LZSA_DST_HI

    LDA #$34
    STA $01

    JMP $0100

; Data section
color_data:
{}
vic_data:
{}
sid_data:
{}
cia1_data:
{}
cia2_data:
{}
zp_data:
{}
f8_ff_data:
{}

; LZSA1 Decompressor
decompress_lzsa1:
    LDY #0
    LDX #0

cp_length:
    LDA (LZSA_SRC_LO),Y
    INC LZSA_SRC_LO
    BNE cp_skip0
    INC LZSA_SRC_HI

cp_skip0:
    STA LZSA_CMDBUF
    AND #$70
    LSR
    BEQ lz_offset
    LSR
    LSR
    LSR
    CMP #$07
    BCC cp_got_len
    JSR get_length
    STX cp_npages+1

cp_got_len:
    TAX

cp_byte:
    LDA (LZSA_SRC_LO),Y
    STA (LZSA_DST_LO),Y
    INC LZSA_SRC_LO
    BNE cp_skip1
    INC LZSA_SRC_HI
cp_skip1:
    INC LZSA_DST_LO
    BNE cp_skip2
    INC LZSA_DST_HI
cp_skip2:
    DEX
    BNE cp_byte
cp_npages:
    LDA #0
    BEQ lz_offset
    DEC cp_npages+1
    BCC cp_byte

lz_offset:
    LDA (LZSA_SRC_LO),Y
    INC LZSA_SRC_LO
    BNE offset_lo
    INC LZSA_SRC_HI

offset_lo:
    STA LZSA_OFFSET+0

    LDA #$FF
    BIT LZSA_CMDBUF
    BPL offset_hi

    LDA (LZSA_SRC_LO),Y
    INC LZSA_SRC_LO
    BNE offset_hi
    INC LZSA_SRC_HI

offset_hi:
    STA LZSA_OFFSET+1

lz_length:
    LDA LZSA_CMDBUF
    AND #$0F
    ADC #$03
    CMP #$12
    BCC got_lz_len
    JSR get_length

got_lz_len:
    INX
    EOR #$FF
    TAY
    EOR #$FF

get_lz_dst:
    ADC LZSA_DST_LO
    STA LZSA_DST_LO
    INY
    BCS get_lz_win
    BEQ get_lz_win
    DEC LZSA_DST_HI

get_lz_win:
    CLC
    ADC LZSA_OFFSET+0
    STA LZSA_WINPTR+0
    LDA LZSA_DST_HI
    ADC LZSA_OFFSET+1
    STA LZSA_WINPTR+1

lz_byte:
    LDA (LZSA_WINPTR),Y
    STA (LZSA_DST_LO),Y
    INY
    BNE lz_byte
    INC LZSA_DST_HI
    DEX
    BNE lz_more
    JMP cp_length

lz_more:
    INC LZSA_WINPTR+1
    LDY #$00
    BEQ lz_byte

get_length:
    CLC
    ADC (LZSA_SRC_LO),Y
    INC LZSA_SRC_LO
    BNE skip_inc
    INC LZSA_SRC_HI

skip_inc:
    BCC got_length
    CLC
    TAX

extra_byte:
    JSR get_byte
    PHA
    TXA
    BEQ extra_word

check_length:
    PLA
    BNE got_length
    DEX
got_length:
    RTS

extra_word:
    JSR get_byte
    TAX
    BNE check_length

finished:
    PLA
    PLA
    PLA
    RTS

get_byte:
    LDA (LZSA_SRC_LO),Y
    INC LZSA_SRC_LO
    BNE got_byte
    INC LZSA_SRC_HI
got_byte:
    RTS
"#,
            self.relocated_size,
            ram_data_size,
            end_data_start,
            ram_lzsa_start,
            data_copy_code,
            color_data,
            vic_data,
            sid_data,
            cia1_data,
            cia2_data,
            zp_data,
            f8_ff_bytes
        )
    }

    /// Generate relocated decompressor binary
    pub fn generate_relocated_decompressor(&self) -> Result<Vec<u8>, String> {
        let asm_source = format!(
            r#"*=$0100

LZSA_SRC_LO = $FC
LZSA_SRC_HI = $FD
LZSA_DST_LO = $FE
LZSA_DST_HI = $FF
LZSA_CMDBUF = $F9
LZSA_WINPTR = $FA
LZSA_OFFSET = $FA

DECOMPRESS_LZSA1:
    LDY #0
    LDX #0

cp_length:
    LDA (LZSA_SRC_LO),Y
    INC LZSA_SRC_LO
    BNE cp_skip0
    INC LZSA_SRC_HI

cp_skip0:
    STA LZSA_CMDBUF
    AND #$70
    LSR
    BEQ lz_offset
    LSR
    LSR
    LSR
    CMP #$07
    BCC cp_got_len
    JSR get_length
    STX cp_npages+1

cp_got_len:
    TAX

cp_byte:
    LDA (LZSA_SRC_LO),Y
    STA (LZSA_DST_LO),Y
    INC LZSA_SRC_LO
    BNE cp_skip1
    INC LZSA_SRC_HI
cp_skip1:
    INC LZSA_DST_LO
    BNE cp_skip2
    INC LZSA_DST_HI
cp_skip2:
    DEX
    BNE cp_byte
cp_npages:
    LDA #0
    BEQ lz_offset
    DEC cp_npages+1
    BCC cp_byte

lz_offset:
    LDA (LZSA_SRC_LO),Y
    INC LZSA_SRC_LO
    BNE offset_lo
    INC LZSA_SRC_HI

offset_lo:
    STA LZSA_OFFSET+0

    LDA #$FF
    BIT LZSA_CMDBUF
    BPL offset_hi

    LDA (LZSA_SRC_LO),Y
    INC LZSA_SRC_LO
    BNE offset_hi
    INC LZSA_SRC_HI

offset_hi:
    STA LZSA_OFFSET+1

lz_length:
    LDA LZSA_CMDBUF
    AND #$0F
    ADC #$03
    CMP #$12
    BCC got_lz_len
    JSR get_length

got_lz_len:
    INX
    EOR #$FF
    TAY
    EOR #$FF

get_lz_dst:
    ADC LZSA_DST_LO
    STA LZSA_DST_LO
    INY
    BCS get_lz_win
    BEQ get_lz_win
    DEC LZSA_DST_HI

get_lz_win:
    CLC
    ADC LZSA_OFFSET+0
    STA LZSA_WINPTR+0
    LDA LZSA_DST_HI
    ADC LZSA_OFFSET+1
    STA LZSA_WINPTR+1

lz_byte:
    LDA (LZSA_WINPTR),Y
    STA (LZSA_DST_LO),Y
    INY
    BNE lz_byte
    INC LZSA_DST_HI
    DEX
    BNE lz_more
    JMP cp_length

lz_more:
    INC LZSA_WINPTR+1
    LDY #$00
    BEQ lz_byte

get_length:
    CLC
    ADC (LZSA_SRC_LO),Y
    INC LZSA_SRC_LO
    BNE skip_inc
    INC LZSA_SRC_HI

skip_inc:
    BCC got_length
    CLC
    TAX

extra_byte:
    JSR get_byte
    PHA
    TXA
    BEQ extra_word

check_length:
    PLA
    BNE got_length
    DEX
got_length:
    RTS

extra_word:
    JSR get_byte
    TAX
    BNE check_length

finished:
    LDA #$30
    STA $01
    JMP ${:04X}

get_byte:
    LDA (LZSA_SRC_LO),Y
    INC LZSA_SRC_LO
    BNE got_byte
    INC LZSA_SRC_HI
got_byte:
    RTS
"#,
            self.block9_addr
        );

        assemble_to_bytes(&asm_source)
    }

    fn format_bytes(&self, data: &[u8]) -> String {
        if data.is_empty() {
            return "    .byte $00".to_string();
        }

        let mut lines = Vec::new();
        for chunk in data.chunks(16) {
            let bytes: Vec<String> = chunk.iter().map(|b| format!("${:02X}", b)).collect();
            lines.push(format!("    .byte {}", bytes.join(",")));
        }
        lines.join("\n")
    }
}
