//! LZSA1 zero-page equates and decoder source for the PRG emitter.

pub const LZSA1_ZP_EQUATES: &str = r#"; LZSA1 zero page variables
LZSA_SRC_LO = $FC
LZSA_SRC_HI = $FD
LZSA_DST_LO = $FE
LZSA_DST_HI = $FF
LZSA_CMDBUF = $F9
LZSA_WINPTR = $FA
LZSA_OFFSET = $FA"#;

pub const LZSA1_MAIN_DECODER: &str = r#"; =============================================================================
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
"#;
