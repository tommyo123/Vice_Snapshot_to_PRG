; ===========================================================================
; LZAN "ZX" 6510 minimal decoder (rep0-only, mode 0x41, EOF-terminated), in
; asm6502 syntax. This is lzan's own 6510 decoder; there is no external upstream.
;
; Implementation notes:
;   * Caller-seeded: the caller writes src (bitstream) and dst (output) into ZP
;     before entry; the decoder reads both from ZP, never from assembly-time
;     constants. Each entry re-initializes only its own scratch (bitbuf, moff)
;     and leaves src/dst as seeded, so one code image is re-callable with no
;     self-modification.
;   * bitbuf init $80 makes the first refill arrive with carry already set, and
;     the guard bit supplies C=1 on every later refill, so the refill needs no
;     SEC.
;   * fetch reads via LDA (src,X) and requires X=0 (init ends with X=0;
;     copy_run exits with X=0; nothing else moves X).
;   * read_gamma exits with A = val lo and Z/C set (LDA val; C=1 on every
;     exit), so the offset-MSB EOF test is one BEQ (gamma 256 wraps the lo
;     byte to 0) and msb-1 is SBC #1.
;   * Literal run: "mptr = src; src += len" is done as one 16-bit copy+add.
;   * copy_run is one fused loop (page INCs on the Y wrap, X = count lo,
;     val+1 = count hi biased by one when lo != 0); clobbers val+1.
;
; It decodes the raw mode-0x41 blob produced by lzan::zx::compress_min_eof_e (not
; the LZAN container: the container's "LZAN"+mode+orig_len header is stripped;
; this blob starts directly at the bitstream).
;
; Entry = full_decomp; in-stream EOF marker -> finish -> RTS.
; ===========================================================================

zp_base = $F8

; ZP holds only the state that needs (zp),Y / (zp,X) indirect addressing plus
; the length counter, all inside the converter's preserved $F8-$FF window
; (8 bytes). bitbuf and moff are never dereferenced, so they live in the
; absolute scratch cell declared at the end of this file (lz_scratch).
src    = zp_base+0  ; 2 bytes: bitstream pointer (caller-seeded = comp_data); (src,X)
dst    = zp_base+2  ; 2 bytes: output pointer (caller-seeded = out_addr); (dst),Y
val    = zp_base+4  ; 2 bytes: gamma result / copy length (counter, not a pointer)
mptr   = zp_base+6  ; 2 bytes: copy source pointer; (mptr),Y

full_decomp:                      ; src/dst are caller-seeded in ZP; reset only
        LDX #0                    ; the decoder's scratch each entry (no SMC)
        STX moff                  ; rep0 offset = 0
        STX moff+1
        LDA #$80                  ; bitbuf = empty (guard-bit sentinel; C=1 on
        STA bitbuf                ; the first refill)
        ; fall into st_literals with X=0 (fetch/rg_entry X=0 invariant)

st_literals:
        JSR read_gamma            ; val = len
        LDA src                   ; mptr = src, src += len (in one pass;
        STA mptr                  ;  copy_run leaves the stream ptr alone)
        CLC
        ADC val
        STA src
        LDA src+1
        STA mptr+1
        ADC val+1
        STA src+1
        JSR copy_run              ; copy val bytes (mptr)->(dst)
        JSR gbit                  ; 1 = new offset, 0 = rep0
        BCC do_rep0
        ; fall into st_newoffset

st_newoffset:
        JSR read_gamma            ; val = msb; A = msb & $FF, Z set, C = 1
        BEQ eof_rts               ; msb == 256 (EOF): lo byte wrapped to 0
        SBC #1                    ; A = msb-1 (carry was set)
        LSR                       ; A = (msb-1)>>1; carry = (msb-1)&1
        STA moff+1
        JSR fetch                 ; A = lsb byte; src advanced (carry preserved)
        ROR                       ; A = (off-1)_lo; carry = lsb&1 (1st len ctrl bit)
        STA moff
        JSR rg_entry              ; gamma(len-1) with backtracked ctrl bit in carry
        INC val                   ; len = (len-1)+1
        BNE domatch
        INC val+1
        BNE domatch

do_rep0:
        JSR read_gamma            ; val = len
domatch:
        LDA dst                   ; mptr = dst - (moff+1)
        CLC
        SBC moff
        STA mptr
        LDA dst+1
        SBC moff+1
        STA mptr+1
        JSR copy_run
after_match:
        JSR gbit
        BCS st_newoffset
        BCC st_literals           ; carry clear -> always taken

; copy_run: copy val (16-bit) bytes (mptr)->(dst); advance dst by val.
; Exits with X=0, Y=val&255, mptr/dst highs page-adjusted; clobbers val+1.
copy_run:
        LDX val                   ; X = count lo
        BEQ cr_go
        INC val+1                 ; hi+1 when lo != 0 (X-loop borrows a page)
cr_go:
        LDY #0
cr_loop:
        LDA (mptr),Y
        STA (dst),Y
        INY
        BNE cr_nohi
        INC mptr+1
        INC dst+1
cr_nohi:
        DEX
        BNE cr_loop
        DEC val+1
        BNE cr_loop
        TYA                       ; dst += Y (page INCs already applied)
        CLC
        ADC dst
        STA dst
        BCC eof_rts
        INC dst+1
eof_rts:
        RTS

; BIT READER. gbit returns the next stream bit in carry (MSB first, guard
; sentinel). Preserves X. The refill ROL needs C=1: the init value $80 shifts
; out C=1 on the first refill, every later refill shifts out the guard bit.
gbit:
        ASL bitbuf                ; carry = next data bit
        BNE gb_have
        JSR fetch                 ; refill: A = next byte (carry = 1 here)
        ROL                       ; C = b7; bit0 = 1 (guard)
        STA bitbuf
gb_have:
        RTS

; fetch: A = (src), src += 1. Preserves carry, Y, X (requires X=0).
fetch:
        LDA (src,X)
        INC src
        BNE f_rts
        INC src+1
f_rts:
        RTS

; read_gamma: value=1; while ctrl==0 { value=(value<<1)|data }.
; Returns A = val & $FF with Z/N set, C = 1 (gbit refills clobber A, so the
; value accumulates in val, not A). Requires and preserves X=0: every caller
; comes from init, copy_run (exits X=0), or a path that kept X=0.
read_gamma:
        JSR gbit                  ; C = first control bit
rg_entry:
        STX val+1                 ; val = 1 (X=0)
        LDA #1
        STA val
        BCS rg_done
rg_data:
        JSR gbit                  ; data bit -> carry
        ROL val
        ROL val+1
        JSR gbit                  ; next control bit -> carry
        BCC rg_data
rg_done:
        LDA val
        RTS

; Absolute scratch for the bit buffer and rep-offset. bitbuf is read on every bit
; (ASL bitbuf), so it must live in memory that always reads back what was written.
; $01C0 satisfies that in both decode phases: page 1 is RAM under both $01=$34 and
; $01=$35, it sits above the relocated body ($0100-$01BC) and below the stack, and
; it is outside the $0200-$FFEF range filled by the RAM decode. It is re-initialised
; on entry, so the cell needs no seed and the decoder stays re-callable.
lz_scratch = $01C0
bitbuf = lz_scratch+0  ; current bit byte (MSB first, guard-bit sentinel); $80 = empty
moff   = lz_scratch+1  ; 2 bytes: current offset (rep0), stored as off-1
