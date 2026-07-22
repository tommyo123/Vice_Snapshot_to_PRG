; ===========================================================================
; Standard ZX0 (v2) 6502 decruncher, forward, in asm6502 syntax.
; Upstream: BeebAsm ZX0 decoder by NegativeCharge (port of Krzysztof "XXL"
; Dudek's standard ZX0 decoder); ZX0 format (c) Einar Saukas, BSD-3-Clause.
;
; Decodes a standard ZX0 v2 forward stream, exactly what
; lzan::zx0compat::compress emits (not a bitfire/Dali-modified variant).
;
; Caller-seeded, re-callable, no self-modifying code. The caller writes the
; source pointer (= comp_data) at src and the destination pointer (= out_addr)
; at dst before entry. Only the three pointers that need (ptr),Y indirection
; stay in zero page (src, output cursor, match source = 6 bytes, kept inside the
; $F8-$FF window). The non-pointer state (the running length lo/hi and the ZX0
; "last offset" lo/hi) lives in an absolute scratch area after the body.
; full_decomp re-initialises the last offset (to -1, i.e. ZX0's initial offset 1)
; and the length high byte on every entry, and the EOF path scrubs the scratch
; back to its load-time zeros, so the same assembled image can be called any
; number of times.
;
; Entry = full_decomp; EOF is the standard ZX0 end marker -> RTS.
; ===========================================================================

zp_base = $F8

src        = zp_base+0  ; 2 bytes: compressed source pointer (caller-seeded = comp_data)
ZX0_OUTPUT = zp_base+2  ; 2 bytes: output cursor (caller-seeded = out_addr)
COPY_SRC   = zp_base+4  ; 2 bytes: match-copy source pointer

full_decomp:
        LDA #$FF                ; last offset = -1 (ZX0 initial offset 1)
        STA offsetL
        STA offsetH
        LDA #$00                ; length high starts at 0 (elias seeds the low byte)
        STA lenH
        LDA #$80                ; empty bit buffer (marker bit only)
        ; src (stream) and ZX0_OUTPUT (dest) are seeded by the caller in ZP.
        ; falls through into dzx0s_literals.
dzx0s_literals:
        JSR dzx0s_elias
        PHA
cop0:
        JSR get_byte
        LDY #$00
        STA (ZX0_OUTPUT),Y
        INC ZX0_OUTPUT
        BNE l0
        INC ZX0_OUTPUT+1
l0:
        LDA lenL
        BNE l1
        DEC lenH
l1:
        DEC lenL
        BNE cop0
        LDA lenH
        BNE cop0
        PLA
        ASL
        BCS dzx0s_new_offset
        JSR dzx0s_elias         ; returns X = lenL with Z/N still valid
        BEQ dzx0s_copy
        INC lenH
dzx0s_copy:
        ; copy X + 256*(lenH-1) bytes (callers preload X and bias lenH):
        ; X counts the partial page (X=0 -> 256), DEC lenH counts full
        ; pages; Y wraps bump both pointer high bytes. A = bit buffer.
        PHA
        LDA ZX0_OUTPUT
        CLC
        ADC offsetL             ; COPY_SRC = ZX0_OUTPUT + offset (offset is negative)
        STA COPY_SRC
        LDA ZX0_OUTPUT+1
        ADC offsetH
        STA COPY_SRC+1
        LDY #$00
copyByte:
        LDA (COPY_SRC),Y
        STA (ZX0_OUTPUT),Y
        INY
        BNE nowrap
        INC COPY_SRC+1
        INC ZX0_OUTPUT+1
nowrap:
        DEX
        BNE copyByte
        DEC lenH
        BNE copyByte
        TYA                     ; Y = count & $FF; add it to the cursor
        CLC
        ADC ZX0_OUTPUT
        STA ZX0_OUTPUT
        BCC copyDone
        INC ZX0_OUTPUT+1
copyDone:
        PLA                     ; lenH is 0 here; lenL is reseeded by elias
        ASL
        BCC dzx0s_literals
dzx0s_new_offset:
        LDX #$FE
        JSR dzx0s_elias_seed    ; returns X = lenL
        INX
        BNE dzx0s_have_offset   ; elias result 1 (X: $FF->0) is the EOF marker
        LDA #$00                ; EOF: scrub the absolute scratch back to its
        STA lenL                ; load-time zeros so the code image is unchanged
        STA lenH                ; across re-entries (re-callable, no SMC)
        STA offsetL
        STA offsetH
        RTS
dzx0s_have_offset:
        PHA
        TXA
        ROR                     ; C=1 here (elias stop bit), so
        STA offsetH             ; offsetH = $80 | X>>1, C = X bit 0
        JSR get_byte            ; (get_byte keeps C)
        ROR                     ; offsetL = C<<7 | byte>>1, C = byte bit 0
        STA offsetL
        LDX #$00
        STX lenH
        INX
        STX lenL
        PLA
        JSR dzx0s_elias_skip    ; C=1: return at once; C=0: keep reading
        INX                     ; match length = elias + 1: X = lenL+1 and
        INC lenH                ; one extra lenH round (X=0 rolls the +1
                                ; into a full extra page)
        BNE dzx0s_copy          ; always: len <= output size < $FF00 so
                                ; lenH+1 never wraps to 0
dzx0s_elias:
        LDX #$01
dzx0s_elias_seed:
        STX lenL                ; seed the accumulator (1, or $FE for offsets)
        BNE dzx0s_elias_loop    ; always: Z=0 from the caller's LDX #imm
                                ; (STX and JSR both leave the flags alone)
dzx0s_elias_backtrack:
        ASL
        ROL lenL
        ROL lenH
dzx0s_elias_loop:
        ASL
        BNE dzx0s_elias_skip
        JSR get_byte
        ROL                     ; C set by the ASL of the $80 marker
dzx0s_elias_skip:
        BCC dzx0s_elias_backtrack
        LDX lenL                ; return low count in X, Z/N valid past RTS
done:
        RTS
get_byte:
        LDY #$00
        LDA (src),Y             ; read stream byte through the ZP cursor
        INC src                 ; advance the cursor (no SMC)
        BNE l5
        INC src+1
l5:
        RTS

; ---- absolute scratch (non-pointer state; counts toward the page-1 budget) ----
; Re-initialised on entry / scrubbed on EOF, so the loaded image is invariant.
lenL:                           ; current length low
        .byte 0
lenH:                           ; current length high
        .byte 0
offsetL:                        ; last offset low  (two's complement, init $FF)
        .byte 0
offsetH:                        ; last offset high (init $FF -> offset -1)
        .byte 0
