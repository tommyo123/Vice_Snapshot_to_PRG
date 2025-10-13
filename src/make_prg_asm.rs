//! PRG file generator using VASM assembler
//!
//! Generates a self-restoring C64 PRG from compressed snapshot components.
//!
//! This program is unlicensed and dedicated to the public domain.
//! Developed by Tommy Olsen.

#![allow(dead_code)]

use crate::config::Config;
use std::fs;

pub struct MakePRGAsm {
    color_lzsa: Vec<u8>,
    vic_lzsa: Vec<u8>,
    sid_lzsa: Vec<u8>,
    cia1_bin: Vec<u8>,
    cia2_bin: Vec<u8>,
    zp_lzsa: Vec<u8>,
    ram_lzsa: Vec<u8>,
    block9_addr: u16,
    f8_ff_data: [u8; 8],
    config: Config,
}

impl MakePRGAsm {
    pub fn new(
        color_lzsa_path: &str,
        vic_lzsa_path: &str,
        sid_lzsa_path: &str,
        cia1_bin_path: &str,
        cia2_bin_path: &str,
        zp_lzsa_path: &str,
        ram_lzsa_path: &str,
        block9_addr: u16,
        f8_ff_data: [u8; 8],
        config: &Config,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let cia1_bin = fs::read(cia1_bin_path)?;
        let cia2_bin = fs::read(cia2_bin_path)?;

        // Validate CIA file size
        if cia1_bin.len() != 20 {
            return Err(format!("CIA1 file must be 20 bytes, got {}", cia1_bin.len()).into());
        }
        if cia2_bin.len() != 20 {
            return Err(format!("CIA2 file must be 20 bytes, got {}", cia2_bin.len()).into());
        }

        Ok(Self {
            color_lzsa: fs::read(color_lzsa_path)?,
            vic_lzsa: fs::read(vic_lzsa_path)?,
            sid_lzsa: fs::read(sid_lzsa_path)?,
            cia1_bin,
            cia2_bin,
            zp_lzsa: fs::read(zp_lzsa_path)?,
            ram_lzsa: fs::read(ram_lzsa_path)?,
            block9_addr,
            f8_ff_data,
            config: config.clone(),
        })
    }

    pub fn generate_prg(&self, output_path: &str) -> Result<(), Box<dyn std::error::Error>> {
        // Assemble relocated decompressor
        let relocated_binary = self.assemble_relocated_code()?;

        if relocated_binary.len() > 256 {
            return Err(format!(
                "Relocated code too large: {} bytes (max 256)",
                relocated_binary.len()
            ).into());
        }

        // Write temporary data files for INCBIN
        self.write_data_files(&relocated_binary)?;

        // Assemble main code
        let main_asm = self.generate_main_code_vasm();
        let prg_binary = self.assemble_with_vasm(&main_asm)?;

        // Write final PRG
        fs::write(output_path, &prg_binary)?;

        Ok(())
    }

    fn write_data_files(&self, relocated_binary: &[u8]) -> Result<(), Box<dyn std::error::Error>> {
        let work = self.config.work_str();

        fs::write(format!("{}/color.lzsa", work), &self.color_lzsa)?;
        fs::write(format!("{}/vic.lzsa", work), &self.vic_lzsa)?;
        fs::write(format!("{}/sid.lzsa", work), &self.sid_lzsa)?;
        fs::write(format!("{}/cia1.bin", work), &self.cia1_bin)?;
        fs::write(format!("{}/cia2.bin", work), &self.cia2_bin)?;
        fs::write(format!("{}/zp.lzsa", work), &self.zp_lzsa)?;
        fs::write(format!("{}/relocated.bin", work), relocated_binary)?;
        fs::write(format!("{}/ram.lzsa", work), &self.ram_lzsa)?;

        Ok(())
    }

    fn assemble_with_vasm(&self, asm_source: &str) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        use crate::asm6502::Assembler6502;

        let mut assembler = Assembler6502::new(&self.config);
        let prg_binary = assembler.assemble_prg(asm_source)
            .map_err(|e| format!("VASM assembly failed: {:?}", e))?;

        Ok(prg_binary)
    }

    fn assemble_relocated_code(&self) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        use crate::asm6502::Assembler6502;

        let asm_source = self.generate_relocated_decompressor();

        let mut assembler = Assembler6502::new(&self.config);
        let binary = assembler.assemble_bytes(&asm_source)
            .map_err(|e| format!("Relocated code assembly failed: {:?}", e))?;

        Ok(binary)
    }

    fn generate_main_code_vasm(&self) -> String {
        let work = self.config.work_str();
        format!(r#"; C64 LZSA1 Snapshot Loader
.org 0x0801

; BASIC stub: SYS 2061
.byte 0x0B,0x08,0x0A,0x00,0x9E,0x32,0x30,0x36,0x31,0x00,0x00,0x00

; LZSA1 zero page variables
.equ LZSA_SRC_LO, 0xFC
.equ LZSA_SRC_HI, 0xFD
.equ LZSA_DST_LO, 0xFE
.equ LZSA_DST_HI, 0xFF
.equ LZSA_CMDBUF, 0xF9
.equ LZSA_WINPTR, 0xFA
.equ LZSA_OFFSET, 0xFA

START:
    sei
    cld

    ; Clear all pending interrupts
    lda 0xdc0d
    lda 0xdd0d
    lda #0x7f
    sta 0xdc0d
    sta 0xdd0d
    lda #0x00
    sta 0xd01a
    lda #0xff
    sta 0xd019

    ; Initialize memory map and stack
    lda #0x35
    sta 0x01
    ldx #0xff
    txs

    ; Decompress Color RAM
    lda #<COLOR_DATA
    sta LZSA_SRC_LO
    lda #>COLOR_DATA
    sta LZSA_SRC_HI
    lda #0x00
    sta LZSA_DST_LO
    lda #0xD8
    sta LZSA_DST_HI
    jsr DECOMPRESS_LZSA1

    ; Decompress VIC registers
    lda #<VIC_DATA
    sta LZSA_SRC_LO
    lda #>VIC_DATA
    sta LZSA_SRC_HI
    lda #0x00
    sta LZSA_DST_LO
    lda #0xD0
    sta LZSA_DST_HI
    jsr DECOMPRESS_LZSA1

    ; Disable VIC IRQs
    lda #0x00
    sta 0xd01a
    lda #0xff
    sta 0xd019

    ; Decompress SID registers
    lda #<SID_DATA
    sta LZSA_SRC_LO
    lda #>SID_DATA
    sta LZSA_SRC_HI
    lda #0x00
    sta LZSA_DST_LO
    lda #0xD4
    sta LZSA_DST_HI
    jsr DECOMPRESS_LZSA1

; =============================================================================
; CIA1 Complete Setup
; =============================================================================
    ; Disable all interrupts and stop timers
    lda #0x7f
    sta 0xdc0d
    lda #0x00
    sta 0xdc0e
    sta 0xdc0f

    ; Restore port registers
    lda CIA1_DATA+2
    sta 0xdc02
    lda CIA1_DATA+3
    sta 0xdc03
    lda CIA1_DATA+0
    sta 0xdc00
    lda CIA1_DATA+1
    sta 0xdc01

    ; Timer A: Write counter, force-load, write latch
    lda CIA1_DATA+16
    sta 0xdc04
    lda CIA1_DATA+17
    sta 0xdc05
    lda #0x10
    sta 0xdc0e
    lda #0x00
    sta 0xdc0e
    lda CIA1_DATA+4
    sta 0xdc04
    lda CIA1_DATA+5
    sta 0xdc05

    ; Timer B: Write counter, force-load, write latch
    lda CIA1_DATA+18
    sta 0xdc06
    lda CIA1_DATA+19
    sta 0xdc07
    lda #0x10
    sta 0xdc0f
    lda #0x00
    sta 0xdc0f
    lda CIA1_DATA+6
    sta 0xdc06
    lda CIA1_DATA+7
    sta 0xdc07

    ; TOD registers (hours->minutes->seconds->tenths)
    lda CIA1_DATA+11
    sta 0xdc0b
    lda CIA1_DATA+10
    sta 0xdc0a
    lda CIA1_DATA+9
    sta 0xdc09
    lda CIA1_DATA+8
    sta 0xdc08

    ; SDR and control registers
    lda CIA1_DATA+12
    sta 0xdc0c
    lda CIA1_DATA+14
    and #0xfe
    sta 0xdc0e
    lda CIA1_DATA+15
    and #0xfe
    sta 0xdc0f

; =============================================================================
; CIA2 Complete Setup
; =============================================================================
    lda #0x7f
    sta 0xdd0d
    lda #0x00
    sta 0xdd0e
    sta 0xdd0f

    lda CIA2_DATA+2
    sta 0xdd02
    lda CIA2_DATA+3
    sta 0xdd03
    lda CIA2_DATA+0
    sta 0xdd00
    lda CIA2_DATA+1
    sta 0xdd01

    lda CIA2_DATA+16
    sta 0xdd04
    lda CIA2_DATA+17
    sta 0xdd05
    lda #0x10
    sta 0xdd0e
    lda #0x00
    sta 0xdd0e
    lda CIA2_DATA+4
    sta 0xdd04
    lda CIA2_DATA+5
    sta 0xdd05

    lda CIA2_DATA+18
    sta 0xdd06
    lda CIA2_DATA+19
    sta 0xdd07
    lda #0x10
    sta 0xdd0f
    lda #0x00
    sta 0xdd0f
    lda CIA2_DATA+6
    sta 0xdd06
    lda CIA2_DATA+7
    sta 0xdd07

    lda CIA2_DATA+11
    sta 0xdd0b
    lda CIA2_DATA+10
    sta 0xdd0a
    lda CIA2_DATA+9
    sta 0xdd09
    lda CIA2_DATA+8
    sta 0xdd08

    lda CIA2_DATA+12
    sta 0xdd0c
    lda CIA2_DATA+14
    and #0xfe
    sta 0xdd0e
    lda CIA2_DATA+15
    and #0xfe
    sta 0xdd0f

; =============================================================================
; Decompress Zero Page
; =============================================================================
    lda #<ZP_DATA
    sta LZSA_SRC_LO
    lda #>ZP_DATA
    sta LZSA_SRC_HI
    lda #0x02
    sta LZSA_DST_LO
    lda #0x00
    sta LZSA_DST_HI
    jsr DECOMPRESS_LZSA1

    ; Switch to RAM-only mode
    lda #0x34
    sta 0x01

    ; Calculate RAM data block size
    lda #<RAM_DATA_SIZE
    sta 0xF8
    lda #>RAM_DATA_SIZE
    sta 0xF9

    ; Set source to end of RAM data
    lda #<(RAM_DATA_END-1)
    sta 0xFE
    lda #>(RAM_DATA_END-1)
    sta 0xFF

    ; Set destination to top of memory
    lda #0xFF
    sta 0xFC
    sta 0xFD

    ; Copy RAM data block to top of memory (backward)
    ldy #0x00
MVLP:
    lda (0xFE),y
    sta (0xFC),y
    lda 0xFE
    bne MV1
    dec 0xFF
MV1:
    dec 0xFE
    lda 0xFC
    bne MV2
    dec 0xFD
MV2:
    dec 0xFC
    lda 0xF8
    bne MV3
    dec 0xF9
MV3:
    dec 0xF8
    lda 0xF8
    ora 0xF9
    bne MVLP

    ; Copy relocated decompressor to $0100-$01FF
    ldx #<(0x10000 - RAM_DATA_SIZE)
    ldy #>(0x10000 - RAM_DATA_SIZE)
    stx 0xFE
    sty 0xFF
    ldy #0x00
CPLP:
    lda (0xFE),y
    sta 0x0100,y
    iny
    cpy #<RELOCATED_SIZE
    bne CPLP

    ; Setup source pointer for final RAM decompression
    lda #<(0x10000 - RAM_DATA_SIZE + RELOCATED_SIZE)
    sta LZSA_SRC_LO
    lda #>(0x10000 - RAM_DATA_SIZE + RELOCATED_SIZE)
    sta LZSA_SRC_HI

    ; Setup destination pointer
    lda #0x00
    sta LZSA_DST_LO
    lda #0x02
    sta LZSA_DST_HI

    ; Jump to relocated decompressor
    jmp 0x0100

; =============================================================================
; Data section
; =============================================================================
COLOR_DATA:
    .incbin "{}/color.lzsa"
VIC_DATA:
    .incbin "{}/vic.lzsa"
SID_DATA:
    .incbin "{}/sid.lzsa"
CIA1_DATA:
    .incbin "{}/cia1.bin"
CIA2_DATA:
    .incbin "{}/cia2.bin"
ZP_DATA:
    .incbin "{}/zp.lzsa"

RAM_DATA_START:
RELOCATED_CODE:
    .incbin "{}/relocated.bin"
RELOCATED_END:
.equ RELOCATED_SIZE, RELOCATED_END-RELOCATED_CODE

RAM_COMPRESSED:
    .incbin "{}/ram.lzsa"
RAM_DATA_END:
.equ RAM_DATA_SIZE, RAM_DATA_END-RAM_DATA_START

; =============================================================================
; LZSA1 Decompressor
; =============================================================================
DECOMPRESS_LZSA1:
    ldy #0
    ldx #0

cp_length:
    lda (LZSA_SRC_LO),y
    inc LZSA_SRC_LO
    bne cp_skip0
    inc LZSA_SRC_HI

cp_skip0:
    sta LZSA_CMDBUF
    and #0x70
    lsr a
    beq lz_offset
    lsr a
    lsr a
    lsr a
    cmp #0x07
    bcc cp_got_len
    jsr get_length
    stx cp_npages+1

cp_got_len:
    tax

cp_byte:
    lda (LZSA_SRC_LO),y
    sta (LZSA_DST_LO),y
    inc LZSA_SRC_LO
    bne cp_skip1
    inc LZSA_SRC_HI
cp_skip1:
    inc LZSA_DST_LO
    bne cp_skip2
    inc LZSA_DST_HI
cp_skip2:
    dex
    bne cp_byte
cp_npages:
    lda #0
    beq lz_offset
    dec cp_npages+1
    bcc cp_byte

lz_offset:
    lda (LZSA_SRC_LO),y
    inc LZSA_SRC_LO
    bne offset_lo
    inc LZSA_SRC_HI

offset_lo:
    sta LZSA_OFFSET+0

    lda #0xFF
    bit LZSA_CMDBUF
    bpl offset_hi

    lda (LZSA_SRC_LO),y
    inc LZSA_SRC_LO
    bne offset_hi
    inc LZSA_SRC_HI

offset_hi:
    sta LZSA_OFFSET+1

lz_length:
    lda LZSA_CMDBUF
    and #0x0F
    adc #0x03
    cmp #0x12
    bcc got_lz_len
    jsr get_length

got_lz_len:
    inx
    eor #0xFF
    tay
    eor #0xFF

get_lz_dst:
    adc LZSA_DST_LO
    sta LZSA_DST_LO
    iny
    bcs get_lz_win
    beq get_lz_win
    dec LZSA_DST_HI

get_lz_win:
    clc
    adc LZSA_OFFSET+0
    sta LZSA_WINPTR+0
    lda LZSA_DST_HI
    adc LZSA_OFFSET+1
    sta LZSA_WINPTR+1

lz_byte:
    lda (LZSA_WINPTR),y
    sta (LZSA_DST_LO),y
    iny
    bne lz_byte
    inc LZSA_DST_HI
    dex
    bne lz_more
    jmp cp_length

lz_more:
    inc LZSA_WINPTR+1
    bne lz_byte

get_length:
    clc
    adc (LZSA_SRC_LO),y
    inc LZSA_SRC_LO
    bne skip_inc
    inc LZSA_SRC_HI

skip_inc:
    bcc got_length
    clc
    tax

extra_byte:
    jsr get_byte
    pha
    txa
    beq extra_word

check_length:
    pla
    bne got_length
    dex
got_length:
    rts

extra_word:
    jsr get_byte
    tax
    bne check_length

finished:
    pla
    pla
    pla
    rts

get_byte:
    lda (LZSA_SRC_LO),y
    inc LZSA_SRC_LO
    bne got_byte
    inc LZSA_SRC_HI
got_byte:
    rts
"#, work, work, work, work, work, work, work, work)
    }

    fn generate_relocated_decompressor(&self) -> String {
        format!(r#"
.org 0x0100

.equ LZSA_SRC_LO, 0xFC
.equ LZSA_SRC_HI, 0xFD
.equ LZSA_DST_LO, 0xFE
.equ LZSA_DST_HI, 0xFF
.equ LZSA_CMDBUF, 0xF9
.equ LZSA_WINPTR, 0xFA
.equ LZSA_OFFSET, 0xFA

; Relocated LZSA1 decompressor in page 1
DECOMPRESS_LZSA1:
    ldy #0
    ldx #0

cp_length:
    lda (LZSA_SRC_LO),y
    inc LZSA_SRC_LO
    bne cp_skip0
    inc LZSA_SRC_HI

cp_skip0:
    sta LZSA_CMDBUF
    and #0x70
    lsr a
    beq lz_offset
    lsr a
    lsr a
    lsr a
    cmp #0x07
    bcc cp_got_len
    jsr get_length
    stx cp_npages+1

cp_got_len:
    tax

cp_byte:
    lda (LZSA_SRC_LO),y
    sta (LZSA_DST_LO),y
    inc LZSA_SRC_LO
    bne cp_skip1
    inc LZSA_SRC_HI
cp_skip1:
    inc LZSA_DST_LO
    bne cp_skip2
    inc LZSA_DST_HI
cp_skip2:
    dex
    bne cp_byte
cp_npages:
    lda #0
    beq lz_offset
    dec cp_npages+1
    bcc cp_byte

lz_offset:
    lda (LZSA_SRC_LO),y
    inc LZSA_SRC_LO
    bne offset_lo
    inc LZSA_SRC_HI

offset_lo:
    sta LZSA_OFFSET+0

    lda #0xFF
    bit LZSA_CMDBUF
    bpl offset_hi

    lda (LZSA_SRC_LO),y
    inc LZSA_SRC_LO
    bne offset_hi
    inc LZSA_SRC_HI

offset_hi:
    sta LZSA_OFFSET+1

lz_length:
    lda LZSA_CMDBUF
    and #0x0F
    adc #0x03
    cmp #0x12
    bcc got_lz_len
    jsr get_length

got_lz_len:
    inx
    eor #0xFF
    tay
    eor #0xFF

get_lz_dst:
    adc LZSA_DST_LO
    sta LZSA_DST_LO
    iny
    bcs get_lz_win
    beq get_lz_win
    dec LZSA_DST_HI

get_lz_win:
    clc
    adc LZSA_OFFSET+0
    sta LZSA_WINPTR+0
    lda LZSA_DST_HI
    adc LZSA_OFFSET+1
    sta LZSA_WINPTR+1

lz_byte:
    lda (LZSA_WINPTR),y
    sta (LZSA_DST_LO),y
    iny
    bne lz_byte
    inc LZSA_DST_HI
    dex
    bne lz_more
    jmp cp_length

lz_more:
    inc LZSA_WINPTR+1
    bne lz_byte

get_length:
    clc
    adc (LZSA_SRC_LO),y
    inc LZSA_SRC_LO
    bne skip_inc
    inc LZSA_SRC_HI

skip_inc:
    bcc got_length
    clc
    tax

extra_byte:
    jsr get_byte
    pha
    txa
    beq extra_word

check_length:
    pla
    bne got_length
    dex
got_length:
    rts

extra_word:
    jsr get_byte
    tax
    bne check_length

finished:
    ; Decompression complete - jump to block 9
    jmp 0x{:04X}

get_byte:
    lda (LZSA_SRC_LO),y
    inc LZSA_SRC_LO
    bne got_byte
    inc LZSA_SRC_HI
got_byte:
    rts
"#, self.block9_addr)
    }
}
