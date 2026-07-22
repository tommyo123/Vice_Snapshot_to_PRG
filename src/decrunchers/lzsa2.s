; ===========================================================================
; LZSA2 6502 decruncher, legal-opcode variant, caller-seeded and re-callable.
;
; Based on lzsa2-marty-small-legal.s (upstream: decompress_small_v2.asm,
; (c) 2019 Emmanuel Marty, zlib). Decodes a raw forward LZSA2 stream as
; produced by lzan::lzsa2::compress at MAX_LEVEL.
;
;   * Caller-seeded: the caller writes src (= comp_data) at zp_base+0 and dst
;     (= out_addr) at zp_base+2 before entry. GETSRC reads (src),Y and PUTDST
;     writes (dst),Y under a Y = 0 invariant.
;   * Re-callable: no operand is self-modified. Entry re-seeds the rep-offset
;     high byte to $FF, and the nibble-buffer flag (NIBCOUNT) is 0 both in the
;     load image and after the EOD scrub.
;   * The match back-reference reads through the mptr zero-page pointer; both
;     copy loops hold their high count in cnthi so Y stays 0 for every indirect
;     access.
;
; ZP is confined to $F8-$FF (8 bytes) so it never collides with the converter's
; $02-$F7 ZP restore block: only the three (ptr),Y pointers and the rep-offset
; stay in zero page. The copy-loop high count (cnthi) and the nibble buffer
; (NIBCOUNT/NIBBLES) live in an absolute scratch area after the body. cnthi is
; left at 0 by every copy loop; the two nibble bytes are scrubbed to zero on the
; EOD exit. The loaded image is therefore byte-identical across re-entries.
;
; ALR #$18 is expanded to AND #$18 / LSR (identical A, Z, C) so the body
; contains no illegal opcodes.
;
; Entry = full_decomp; RTS at DECOMPRESSION_DONE.
; ===========================================================================

zp_base = $F8

src      = zp_base+0    ; 2 bytes: compressed source pointer (caller-seeded = comp_data)
dst      = zp_base+2    ; 2 bytes: output pointer (caller-seeded = out_addr)
mptr     = zp_base+4    ; 2 bytes: match back-reference pointer
offs     = zp_base+6    ; 2 bytes: rep-match offset (high byte re-seeded $FF each entry)
; cnthi (1), NIBCOUNT (1) and NIBBLES (1) live in absolute scratch after the body.

full_decomp:
DECOMPRESS_LZSA2:
        LDY #$FF                        ; rep-offset seed = $FFFF (-1). An all-literals
        STY offs+1                      ; stream reaches EOD before any match sets the
        STY offs                        ; offset, and EOD is recognised only by the carry
        INY                             ; out of the offset add. With both bytes seeded
                                        ; that carry comes from dst_lo + $FF, which holds
                                        ; for every dst_lo >= 1 and does not depend on
                                        ; dst_hi. Y = 0 for the run.
        ; NIBCOUNT starts 0 from the image / the EOD scrub, so no re-init needed.

DECODE_TOKEN:
        JSR GETSRC                      ; read token byte: XYZ|LL|MMM
        PHA                             ; preserve token on stack

        AND #$18                        ; legal expansion of ALR #$18:
        LSR                             ; (token & $18) >> 1
        BEQ NO_LITERALS
        LSR
        LSR
        CMP #$03                        ; LITERALS_RUN_LEN_V2?
        BCC PREPARE_COPY_LITERALS

        JSR GETNIBBLE                   ; extra literals length nibble
        ADC #$02                        ; (LITERALS_RUN_LEN_V2) minus carry
        CMP #$12                        ; LITERALS_RUN_LEN_V2 + 15 ?
        BCC PREPARE_COPY_LITERALS

        JSR GETSRC                      ; extra byte of variable literals count
        SBC #$EE                        ; overflow?

PREPARE_COPY_LITERALS:
        JSR PREP_COUNT                  ; X = low count, cnthi = high count

COPY_LITERALS:
        JSR GETPUT                      ; copy one byte of literals
        DEX
        BNE COPY_LITERALS
        DEC cnthi
        BNE COPY_LITERALS

NO_LITERALS:
        PLA                             ; retrieve token from stack
        PHA
        ASL
        BCS REPMATCH_OR_LARGE_OFFSET    ; 1YZ: rep-match or 13/16 bit offset

        ASL                             ; 0YZ: 5 or 9 bit offset
        BCS OFFSET_9_BIT

        ; 00Z: 5 bit offset
        LDX #$FF                        ; set offset bits 15-8 to 1
        JSR GETCOMBINEDBITS             ; rotate Z bit into bit 0, read nibble for bits 4-1
        ORA #$E0                        ; set bits 7-5 to 1
        BNE GOT_OFFSET_LO               ; store low byte and prepare match

OFFSET_9_BIT:                           ; 01Z: 9 bit offset
        ROL                             ; carry: Z bit; A: xxxxxxx1
        ADC #$00                        ; if Z set, add 1
        ORA #$FE                        ; set offset bits 15-9 to 1
        BNE GOT_OFFSET_HI               ; (like JMP GOT_OFFSET_HI but shorter)

REPMATCH_OR_LARGE_OFFSET:
        ASL                             ; 13 bit offset?
        BCS REPMATCH_OR_16_BIT

        ; 10Z: 13 bit offset
        JSR GETCOMBINEDBITS             ; rotate Z bit into bit 8, read nibble for bits 12-9
        ADC #$DE                        ; set bits 15-13 to 1 and subtract 2 (512)
        BNE GOT_OFFSET_HI

REPMATCH_OR_16_BIT:                     ; rep-match or 16 bit offset
        BMI REP_MATCH                   ; reuse previous offset (rep-match)

        ; 110: handle 16 bit offset
        JSR GETSRC                      ; grab high 8 bits
GOT_OFFSET_HI:
        TAX
        JSR GETSRC                      ; grab low 8 bits
GOT_OFFSET_LO:
        STA offs                        ; store low byte of match offset
        STX offs+1                      ; store high byte of match offset

REP_MATCH:
        ; Forward decompression: mptr = dst + offset. The high add's carry is
        ; consumed by the match-length ADC #$01 below (MIN_MATCH_SIZE_V2): a real
        ; match offset is negative and its source lies inside the output already
        ; written, so the add always carries. The $FFFF seed gives the same carry
        ; when EOD is reached before any match has set an offset.
        CLC
        LDA dst
        ADC offs                        ; low 8 bits
        STA mptr
        LDA offs+1                      ; high 8 bits
        ADC dst+1
        STA mptr+1

        PLA                             ; retrieve token from stack again
        AND #$07                        ; isolate match len (MMM)
        ADC #$01                        ; add MIN_MATCH_SIZE_V2 and carry
        CMP #$09                        ; MIN_MATCH_SIZE_V2 + MATCH_RUN_LEN_V2?
        BCC PREPARE_COPY_MATCH

        JSR GETNIBBLE                   ; extra match length nibble
        ADC #$08                        ; (MIN_MATCH_SIZE_V2 + MATCH_RUN_LEN_V2) minus carry
        CMP #$18
        BCC PREPARE_COPY_MATCH

        JSR GETSRC                      ; extra byte of variable match length
        SBC #$E8                        ; overflow?
        BEQ DECOMPRESSION_DONE          ; length 0 here (C set) is the EOD code: bail
                                        ; (only the SBC path can be 0; short paths skip this)

PREPARE_COPY_MATCH:
        JSR PREP_COUNT                  ; X = low count, cnthi = high count

COPY_MATCH_LOOP:
        LDA (mptr),Y                    ; get one byte of backreference (Y = 0)
        JSR PUTDST                      ; copy to destination

        INC mptr
        BNE GETMATCH_DONE
        INC mptr+1
GETMATCH_DONE:

        DEX
        BNE COPY_MATCH_LOOP
        DEC cnthi
        BNE COPY_MATCH_LOOP
        JMP DECODE_TOKEN

; Shared copy-length setup. Enter with A = low count and C = 16-bit-count flag
; (C set => a full 16-bit count follows in the stream). Leaves X = low count and
; cnthi = high count, so the caller's copy loop runs A + 256*cnthi bytes.
PREP_COUNT:
        TAX
        LDA #$00                        ; high count = 0 for the < 256 path
        BCC PREP_COUNT_STORE
        JSR GETLARGESRC                 ; 16 bit count: low in X, high in A
PREP_COUNT_STORE:
        CPX #$01                        ; C = (low != 0): a partial page adds one
        ADC #$00                        ; high count += (low != 0)
        STA cnthi
        RTS

GETCOMBINEDBITS:
        EOR #$80
        ASL
        PHP

        JSR GETNIBBLE                   ; get nibble into bits 0-3 (offset bits 1-4)
        PLP                             ; merge Z bit as carry (offset bit 0)
        ROL                             ; nibble -> bits 1-4; carry -> bit 0
        RTS

DECOMPRESSION_DONE:                     ; EOD: A = 0 here, so scrub the nibble
        STA NIBCOUNT                    ; scratch back to its load-time zeros ;@reloc-drop
        STA NIBBLES                     ; (cnthi is already 0 after every copy). ;@reloc-drop
        RTS                             ;@exit

GETNIBBLE:
        LDA NIBBLES                     ; buffered nibble pair (absolute scratch)
        LSR NIBCOUNT
        BCS HAS_NIBBLES

        INC NIBCOUNT
        JSR GETSRC                      ; get 2 nibbles
        STA NIBBLES
        LSR
        LSR
        LSR
        LSR
        SEC

HAS_NIBBLES:
        AND #$0F                        ; isolate low 4 bits of nibble
        RTS

; --- Forward GETPUT / PUTDST / GETLARGESRC / GETSRC (all via zero page) ------
GETPUT:
        JSR GETSRC
PUTDST:
        STA (dst),Y                     ; Y = 0 invariant
        INC dst
        BNE PUTDST_DONE
        INC dst+1
PUTDST_DONE:
        RTS

GETLARGESRC:
        JSR GETSRC                      ; grab low 8 bits
        TAX                             ; move to X
                                        ; fall through grab high 8 bits
GETSRC:
        LDA (src),Y                     ; Y = 0 invariant
        INC src
        BNE GETSRC_DONE
        INC src+1
GETSRC_DONE:
        RTS

; ---- absolute scratch (page-1 image, invariant across re-entries) ----------
cnthi:
        .byte 0                         ; copy-loop high count (0 after each loop)
NIBCOUNT:
        .byte 0                         ; nibble-buffer flag (bit0); 0 = empty
NIBBLES:
        .byte 0                         ; buffered nibble pair
