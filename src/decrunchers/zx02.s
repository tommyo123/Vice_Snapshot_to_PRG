; ===========================================================================
; ZX02 6502 decruncher, in asm6502 syntax.
; Upstream: zx02 6502 decoder (c) 2022 DMSC, MIT.
; Decodes a raw forward ZX02 stream as produced by lzan::zx02::compress.
;
; Caller-seeded: before entry the caller writes the compressed-source pointer
; (= comp_data) into ZX0_src and the output pointer (= out_addr) into ZX0_dst.
; Each entry re-inits only its own scratch (offset, bit reservoir), so one
; loaded code image can be called repeatedly with different src/dst.
; Entry = full_decomp; terminates with RTS.
;
; Zero page is held to $F8-$FF (8 bytes) so it never collides with the
; converter's $02-$F7 ZP restore block: only the pointers that need indirect
; addressing stay in ZP: ZX0_dst / ZX0_src / pntr for the (ptr,X) reads and
; writes, plus the bit reservoir and the saved X. The rep/last-offset word is
; arithmetic-only and lives in absolute scratch (offset); the copy-source
; subtraction is unrolled to address it directly. full_decomp clears that
; scratch before the final RTS, so the loaded image is byte-for-byte identical
; between calls and no operand is self-modified.
; ===========================================================================

zp_base = $F8

bitr     = zp_base+0   ; 1 byte : bit reservoir  (scratch, re-init on entry)
ZX0_dst  = zp_base+1   ; 2 bytes: output pointer  (caller-seeded = out_addr)
ZX0_src  = zp_base+3   ; 2 bytes: source pointer  (caller-seeded = comp_data)
pntr     = zp_base+5   ; 2 bytes: match source ptr (aliases ZX0_src+2)
setx     = zp_base+7   ; 1 byte : saved X

;--------------------------------------------------
; Decompress ZX0 data (6502 optimized format)
full_decomp:
        ; Re-init scratch only; the caller has already seeded ZX0_src/ZX0_dst.
        LDA #0
        STA offset          ; offset-1 = 0  -> initial offset 1 (ZX02 default)
        STA offset+1
        LDA #$80
        STA bitr            ; bit reservoir; $80 is the required init value

        ; Init: X = -2
        LDX #$FE

; Decode literal: copy next N bytes from compressed file
decode_literal:
        LDY #1
        JSR get_elias
        JSR put_byte
        BCS dzx0s_new_offset

        ; Copy from last offset (repeat N bytes from last offset)
        INY
        JSR get_elias
dzx0s_copy:
        ; C=0 from get_elias. pntr = ZX0_dst - offset - 1 (16-bit); X enters
        ; $FE and leaves $00 so the following put_byte reads through (pntr).
        ; offset is absolute, so the two SBC steps are unrolled (no ZP wrap).
        LDA ZX0_dst+2,X
        SBC offset
        STA pntr+2,X
        INX
        LDA ZX0_dst+2,X
        SBC offset+1
        STA pntr+2,X
        INX

        JSR put_byte
        BCC decode_literal

; Copy from new offset (repeat N bytes from new offset)
dzx0s_new_offset:
        ; Read elias code for high part of offset
        INY
        JSR get_elias
        BEQ decode_done   ; Read a 0, signals the end
        ; Decrease and divide by 2
        DEY
        TYA
        LSR
        STA offset+1

        ; Get low part of offset, a literal 7 bits
        JSR get_byte

        ; Divide by 2
        ROR
        STA offset

        ; And get the copy length.
        ; Start elias reading with the bit already in carry:
        LDY #1
        JSR elias_skip1

        INY
        BCC dzx0s_copy

; Read an elias-gamma interlaced code.
; ------------------------------------
elias_loop:
        ; Read next data bit to result
        ASL bitr
        ROL
        TAY

get_elias:
        ; Get one bit
        ASL bitr
        BNE elias_skip1

        ; Read new bit from stream
        JSR get_byte
        ROL
        STA bitr

elias_skip1:
        TYA
        BCS elias_loop
        ; Got ending bit, stop reading
        RTS

;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;
get_byte:
        LDA (ZX0_src+2,X)
        INC ZX0_src+2,X
        BNE get_byte_done
        INC ZX0_src+3,X
get_byte_done:
        RTS

;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;;
put_byte:
        STX setx
ploop:
        LDX setx
        JSR get_byte
        LDX #$FE
        STA (ZX0_dst+2,X)
        INC ZX0_dst
        BNE put_byte_skip
        INC ZX0_dst+1
put_byte_skip:
        DEY
        BNE ploop
        ASL bitr
        RTS

; End of stream: clear the absolute offset scratch so the loaded image is
; byte-identical for the next call, then return to the caller.
decode_done:
        LDA #0
        STA offset
        STA offset+1
        RTS

; ---- absolute scratch (arithmetic-only; needs no zero page) ----
; offset is read on every match (SBC offset), so it must sit in memory that reads
; back what was written under both $01=$34 and $01=$35. Page 1 is RAM under both
; settings and lies outside the RAM decode range ($0200-$FFEF). $01C0 sits above
; the relocated body and below the stack; full_decomp re-inits it on entry.
offset = $01C0             ; 2 bytes: last offset - 1 (lo/hi)
