//! PRG file generator using inline asm6502 assembler
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

        // Write temporary data files for .incbin
        self.write_data_files(&relocated_binary)?;

        // Assemble main code
        let main_asm = self.generate_main_code_asm6502();
        let prg_binary = self.assemble_with_asm6502(&main_asm)?;

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

    fn assemble_with_asm6502(&self, asm_source: &str) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        use crate::asm_wrapper::Assembler6502Wrapper;

        let mut assembler = Assembler6502Wrapper::new();
        let prg_binary = assembler.assemble_prg(asm_source)
            .map_err(|e| format!("Assembly failed: {:?}", e))?;

        Ok(prg_binary)
    }

    fn assemble_relocated_code(&self) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        use crate::asm_wrapper::Assembler6502Wrapper;

        let asm_source = self.generate_relocated_decompressor();

        let mut assembler = Assembler6502Wrapper::new();
        let binary = assembler.assemble_bytes(&asm_source)
            .map_err(|e| format!("Relocated code assembly failed: {:?}", e))?;

        Ok(binary)
    }

    fn generate_main_code_asm6502(&self) -> String {
        let work = self.config.work_str();

        // Convert Windows backslashes to forward slashes for cross-platform compatibility
        let work_path = work.replace('\\', "/");

        format!(r#"; C64 LZSA1 Snapshot Loader - Conservative Optimization
*=$0801

; BASIC stub: SYS 2061
.byte $0B,$08,$0A,$00,$9E,$32,$30,$36,$31,$00,$00,$00

; LZSA1 zero page variables
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

    ; Initialize memory map and stack
    LDA #$35
    STA $01
    LDX #$FF
    TXS

    ; Decompress Color RAM
    LDA #<color_data
    STA LZSA_SRC_LO
    LDA #>color_data
    STA LZSA_SRC_HI
    LDA #$00
    STA LZSA_DST_LO
    LDA #$D8
    STA LZSA_DST_HI
    JSR decompress_lzsa1

    ; Decompress VIC registers
    LDA #<vic_data
    STA LZSA_SRC_LO
    LDA #>vic_data
    STA LZSA_SRC_HI
    LDA #$00
    STA LZSA_DST_LO
    LDA #$D0
    STA LZSA_DST_HI
    JSR decompress_lzsa1

    ; OPTIMIZATION: Setup VIC raster position early (moved from $01xx)
    ; This is 100% safe - no interrupts enabled yet
    LDA $D011
    STA $D011
    LDA $D012
    STA $D012

    ; Disable VIC IRQs (extra safety)
    LDA #$00
    STA $D01A

    ; Clear VIC IRQ flags
    LDA #$FF
    STA $D019

    ; Decompress SID registers
    LDA #<sid_data
    STA LZSA_SRC_LO
    LDA #>sid_data
    STA LZSA_SRC_HI
    LDA #$00
    STA LZSA_DST_LO
    LDA #$D4
    STA LZSA_DST_HI
    JSR decompress_lzsa1

; =============================================================================
; CIA1 Complete Setup (100% safe - no timers started yet)
; =============================================================================
    ; Disable all interrupts and stop timers
    LDA #$7F
    STA $DC0D
    LDA #$00
    STA $DC0E
    STA $DC0F

    ; Restore port registers
    LDA cia1_data+2
    STA $DC02
    LDA cia1_data+3
    STA $DC03
    LDA cia1_data+0
    STA $DC00
    LDA cia1_data+1
    STA $DC01

    ; Timer A: Write counter, force-load, write latch
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

    ; Timer B: Write counter, force-load, write latch
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

    ; TOD registers (hours->minutes->seconds->tenths)
    LDA cia1_data+11
    STA $DC0B
    LDA cia1_data+10
    STA $DC0A
    LDA cia1_data+9
    STA $DC09
    LDA cia1_data+8
    STA $DC08

    ; SDR and control registers (WITHOUT start bit - safe!)
    LDA cia1_data+12
    STA $DC0C
    LDA cia1_data+14
    AND #$FE
    STA $DC0E
    LDA cia1_data+15
    AND #$FE
    STA $DC0F

; =============================================================================
; CIA2 Complete Setup (100% safe - no timers started yet)
; =============================================================================
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

; =============================================================================
; Decompress Zero Page
; =============================================================================
    LDA #<zp_data
    STA LZSA_SRC_LO
    LDA #>zp_data
    STA LZSA_SRC_HI
    LDA #$02
    STA LZSA_DST_LO
    LDA #$00
    STA LZSA_DST_HI
    JSR decompress_lzsa1

    ; Switch to RAM-only mode
    LDA #$34
    STA $01

    ; Calculate RAM data block size
    LDA #<RAM_DATA_SIZE
    STA $F8
    LDA #>RAM_DATA_SIZE
    STA $F9

    ; Set source to end of RAM data
    LDA #<(RAM_DATA_END-1)
    STA $FE
    LDA #>(RAM_DATA_END-1)
    STA $FF

    ; Set destination to top of memory
    LDA #$FF
    STA $FC
    STA $FD

    ; Copy RAM data block to top of memory (backward)
    LDY #$00
MVLP:
    LDA ($FE),Y
    STA ($FC),Y
    LDA $FE
    BNE MV1
    DEC $FF
MV1:
    DEC $FE
    LDA $FC
    BNE MV2
    DEC $FD
MV2:
    DEC $FC
    LDA $F8
    BNE MV3
    DEC $F9
MV3:
    DEC $F8
    LDA $F8
    ORA $F9
    BNE MVLP

    ; Copy relocated decompressor to $0100-$01FF
    LDX #<($10000 - RAM_DATA_SIZE)
    LDY #>($10000 - RAM_DATA_SIZE)
    STX $FE
    STY $FF
    LDY #$00
CPLP:
    LDA ($FE),Y
    STA $0100,Y
    INY
    CPY #<RELOCATED_SIZE
    BNE CPLP

    ; Setup source pointer for final RAM decompression
    LDA #<($10000 - RAM_DATA_SIZE + RELOCATED_SIZE)
    STA LZSA_SRC_LO
    LDA #>($10000 - RAM_DATA_SIZE + RELOCATED_SIZE)
    STA LZSA_SRC_HI

    ; Setup destination pointer (start at $0200 - skip $0100-$01FF!)
    LDA #$00
    STA LZSA_DST_LO
    LDA #$02
    STA LZSA_DST_HI

    ; Jump to relocated decompressor
    JMP $0100

; =============================================================================
; Data section
; =============================================================================
color_data:
    .incbin "{}/color.lzsa"
vic_data:
    .incbin "{}/vic.lzsa"
sid_data:
    .incbin "{}/sid.lzsa"
cia1_data:
    .incbin "{}/cia1.bin"
cia2_data:
    .incbin "{}/cia2.bin"
zp_data:
    .incbin "{}/zp.lzsa"

ram_data_start:
relocated_code:
    .incbin "{}/relocated.bin"
relocated_end:
RELOCATED_SIZE = relocated_end-relocated_code

ram_compressed:
    .incbin "{}/ram.lzsa"
ram_data_end:
RAM_DATA_SIZE = ram_data_end-ram_data_start
RAM_DATA_END = ram_data_end

; =============================================================================
; LZSA1 Decompressor
; =============================================================================
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
"#, work_path, work_path, work_path, work_path, work_path, work_path, work_path, work_path)
    }

    fn generate_relocated_decompressor(&self) -> String {
        format!(r#"*=$0100

LZSA_SRC_LO = $FC
LZSA_SRC_HI = $FD
LZSA_DST_LO = $FE
LZSA_DST_HI = $FF
LZSA_CMDBUF = $F9
LZSA_WINPTR = $FA
LZSA_OFFSET = $FA

; Relocated LZSA1 decompressor in page 1
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
    BNE lz_byte

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
    ; Decompression complete - jump to block 9
    JMP ${:04X}

get_byte:
    LDA (LZSA_SRC_LO),Y
    INC LZSA_SRC_LO
    BNE got_byte
    INC LZSA_SRC_HI
got_byte:
    RTS
"#, self.block9_addr)
    }
}
