; ===========================================================================
; ByteBoozer2 standard in-memory decruncher (forward), in asm6502 syntax.
; Upstream: ByteBoozer2 Decruncher.inc (c) 2018 Luigi Di Fraia (decruncher by
; HCL 2003, B2 by David Malmborg 2014), MIT.
;
; Decodes the ByteBoozer2 stream produced by lzan::bb2::compress.
;
; Caller contract:
;   * The caller writes the source pointer (src) and the destination pointer
;     (dst) into ZP before each JSR; the decoder reads both from ZP and
;     re-initialises its scratch on entry, so one code image can be re-entered
;     any number of times with different src/dst.
;   * The payload is the raw crunched bitstream (no dst prefix): there is no
;     embedded destination to read or skip, dst comes solely from the
;     caller-seeded ZP location.
;   * The source byte fetch is an (src,X) pointer read with X held at 0.
;   * The literal/match length compare and the match source pointer live in ZP
;     (len, mptr). No code byte is written at run time.
;
; GetBit = "ASL bits / BNE <rts>" falling into the refill (save A in Y, fetch,
; ROL with the sentinel carry, restore A). Callers consume only C. GetByte
; returns the next stream byte in A and preserves Y and C. `.byte $2C` (BIT abs)
; swallows MShort's "LDY #$FF". EOF (match length $FF) branches to the shared
; RTS at GbEnd.
;
; ZP window (zp_base=$F8): src $F8/$F9, dst $FA/$FB, mptr $FC/$FD, bits $FE,
; len $FF. Entry = full_decomp. Terminates with RTS.
; ===========================================================================

zp_base = $F8

src     = zp_base+0     ; 2: caller-seeded source pointer (= comp_data)
dst     = zp_base+2     ; 2: caller-seeded output pointer (= out_addr)
mptr    = zp_base+4     ; 2: match source pointer
bits    = zp_base+6     ; 1: bit accumulator
len     = zp_base+7     ; 1: literal/match length loop terminator
put     = dst           ; the decoder advances the output pointer in place

full_decomp:
        LDX #0          ; X stays 0 so GetByte's (src,X) reads *src
        LDA #$80
        STA bits
DLoop:
        JSR GetBit
        BCS Match
Literal:
        ; Literal run.. get length.
        JSR GetLen
        STA len

        LDY #0
LLoop:
        JSR GetByte
        STA (put),Y
        INY
        CPY len
        BNE LLoop

        CLC
        JSR AddPut
        INY
        BEQ DLoop

        ; Has to continue with a match..
Match:
        ; Match.. get length.
        JSR GetLen
        STA len

        ; Length 255 -> EOF
        CMP #$FF
        BEQ GbEnd

        ; Get num bits
        CMP #2
        LDA #0
        ROL
        JSR GetBit
        ROL
        JSR GetBit
        ROL
        TAY
        LDA Tab,Y
        BEQ M8

        ; Get bits < 8
M_1:
        JSR GetBit
        ROL
        BCS M_1
        BMI MShort
M8:
        ; Get byte
        EOR #$FF
        TAY
        JSR GetByte
        .byte $2C   ; BIT abs -> swallow the following "LDY #$FF" (skip trick)
MShort:
        LDY #$FF
Mdone:
        ; clc
        ADC put
        STA mptr
        TYA
        ADC put+1
        STA mptr+1

        LDY #$FF
MLoop:
        INY
        LDA (mptr),Y
        STA (put),Y
        CPY len
        BNE MLoop

        ; sec
        JSR AddPut
        JMP DLoop

GetLen:
        LDA #1
GlLoop:
        JSR GetBit
        BCC GlEnd
        JSR GetBit
        ROL
        BPL GlLoop
GlEnd:
        RTS

AddPut:
        TYA
        ADC put
        STA put
        BCC ApEnd
        INC put+1
ApEnd:
        RTS

GetBit:
        ASL bits
        BNE GbEnd       ; C = extracted bit
        ; fall into the refill (C=1: the shifted-out sentinel)
GetNewBits:
        TAY             ; save caller's A
        JSR GetByte
        ROL             ; A = (byte<<1)|1, C = byte bit7 (first data bit)
        STA bits
        TYA
        RTS

GetByte:
        LDA (src,X)     ; X=0: read *src (caller-seeded ZP pointer)
        INC src
        BNE GbEnd
        INC src+1
GbEnd:
        RTS

Tab:
        ; Short offsets
        .byte $DF                ; 3
        .byte $FB                ; 6
        .byte $00                ; 8
        .byte $80                ; 10
        ; Long offsets
        .byte $EF                ; 4
        .byte $FD                ; 7
        .byte $80                ; 10
        .byte $F0                ; 13
