//! Compression-format registry.
//!
//! Maps each [`PackFormat`] to its lzan encoder, in-place safety-gap function, and, for the six
//! non-LZSA1 formats, the embedded caller-seeded 6502 decruncher. LZSA1 keeps its own inline
//! decoder in the emitters.
//!
//! The decrunchers in `decrunchers/*.s` are caller-seeded (the emitter writes the source and
//! destination pointers into zero page before entry), re-callable (each entry re-initialises its
//! own scratch), and keep all zero-page use within the $F8-$FF window the converter preserves
//! across decompression. Each decodes the exact stream its lzan encoder emits.

use crate::config::PackFormat;

/// Zero-page addresses the emitter seeds before calling a decruncher: the source pointer at
/// `src_lo`/`src_lo+1` and the destination pointer at `dst_lo`/`dst_lo+1`. Both lie in $F8-$FF.
pub struct ZpSeed {
    pub src_lo: u8,
    pub dst_lo: u8,
}

/// Pages the I/O scratch spans: enough for the largest block (color RAM, 1024 bytes).
pub const IO_SCRATCH_PAGES: usize = 4;

/// First page of the I/O window ($D000) and the page just past it ($E000). Under $01=$35 the small
/// blocks decode with this range banked to I/O, so the scratch must not intersect it.
const IO_WINDOW_LO: usize = 0xD0;
const IO_WINDOW_HI: usize = 0xE0;

/// Page-aligned base of the RAM scratch used by [`io_decode_block`]. The scratch normally sits
/// directly below the payload the loader parks at the top of RAM (`end_data_start`): that region is
/// free while the small blocks decode and is later overwritten by the RAM decode itself. The small
/// blocks decode under $01=$35, however, so if that position would fall in the $D000-$DFFF I/O
/// window the scratch is dropped to just below it ($CC00), which is RAM under $35 and, for any
/// payload high enough to trigger the case, still below `end_data_start`.
pub fn io_scratch_page_below(end_data_start: usize) -> u8 {
    let natural = (end_data_start >> 8).saturating_sub(IO_SCRATCH_PAGES);
    let intersects_io = natural < IO_WINDOW_HI && natural + IO_SCRATCH_PAGES > IO_WINDOW_LO;
    if intersects_io {
        (IO_WINDOW_LO - IO_SCRATCH_PAGES) as u8
    } else {
        natural as u8
    }
}

/// Verify the scratch returned by [`io_scratch_page_below`] is usable: it must clear the restore
/// code it sits above, stay below the payload it sits below, and be RAM (never the $D000-$DFFF I/O
/// window) under $01=$35. `code_start` is where the loader parks the code in RAM, `code_size` its
/// assembled length. LZSA1 decodes straight to I/O and needs no scratch.
pub fn check_io_scratch_fits(
    format: PackFormat,
    end_data_start: usize,
    code_start: u16,
    code_size: usize,
) -> Result<(), String> {
    if format == PackFormat::Lzsa1 {
        return Ok(());
    }
    let bytes = IO_SCRATCH_PAGES << 8;
    let page = io_scratch_page_below(end_data_start) as usize;
    let scratch = page << 8;
    let code_end = code_start as usize + code_size;
    let intersects_io = page < IO_WINDOW_HI && page + IO_SCRATCH_PAGES > IO_WINDOW_LO;
    if scratch < code_end || scratch + bytes > end_data_start || intersects_io {
        return Err(format!(
            "{} needs a {}-byte RAM scratch, but ${:04X}-${:04X} does not fit between the restore \
             code (${:04X}-${:04X}) and the payload at ${:04X} while staying clear of I/O",
            format.as_str(),
            bytes,
            scratch,
            scratch + bytes - 1,
            code_start,
            code_end - 1,
            end_data_start,
        ));
    }
    Ok(())
}

/// Emit the code that decompresses one small block to an I/O register range.
///
/// The color/VIC/SID blocks land in write-mostly I/O ($D000/$D400/$D800) where reads do not return
/// the byte just written. A match-based decoder that back-references its output would therefore read
/// hardware values and corrupt the result. LZSA1 decodes straight to I/O; every other format
/// decodes into a RAM scratch first, where reads are faithful, then copies the fixed-size block to
/// its I/O destination. `src_label` is the `.incbin` label, `io_page` the destination high byte,
/// `size` the decompressed length, `n` a unique loop-label suffix, and `scratch_page` the high byte
/// of the scratch, which must be free RAM for `IO_SCRATCH_PAGES` pages in the emitter's memory map.
pub fn io_decode_block(
    format: PackFormat,
    src_label: &str,
    io_page: u8,
    size: usize,
    n: &str,
    scratch_page: u8,
) -> String {
    let seed = |dst_page: u8| {
        format!(
            "    LDA #<{s}\n    STA LZSA_SRC_LO\n    LDA #>{s}\n    STA LZSA_SRC_HI\n\
             \x20   LDA #$00\n    STA LZSA_DST_LO\n    LDA #${p:02X}\n    STA LZSA_DST_HI\n    JSR {e}",
            s = src_label,
            p = dst_page,
            e = entry_label(format),
        )
    };

    if format == PackFormat::Lzsa1 {
        // LZSA1 decodes straight to the I/O range.
        return seed(io_page);
    }

    // Decode into the RAM scratch page(s), then copy to the I/O range.
    let mut out = seed(scratch_page);
    out.push('\n');
    let full_pages = size / 256;
    let rem = size % 256;
    // One indexed pass (X = 0..=255) moves one byte from each covered page; a final short pass
    // handles a partial trailing page.
    if full_pages > 0 {
        out.push_str("    LDX #$00\n");
        out.push_str(&format!("io_copy_{n}:\n"));
        for k in 0..full_pages {
            out.push_str(&format!(
                "    LDA ${:02X}00,X\n    STA ${:02X}00,X\n",
                scratch_page + k as u8,
                io_page + k as u8
            ));
        }
        out.push_str("    INX\n    BNE io_copy_");
        out.push_str(n);
        out.push('\n');
    }
    if rem > 0 {
        let src_page = scratch_page + full_pages as u8;
        let dst_page = io_page + full_pages as u8;
        out.push_str("    LDX #$00\n");
        out.push_str(&format!("io_tail_{n}:\n"));
        out.push_str(&format!(
            "    LDA ${:02X}00,X\n    STA ${:02X}00,X\n    INX\n    CPX #${:02X}\n    BNE io_tail_{n}\n",
            src_page, dst_page, rem as u8
        ));
    }
    out
}

/// Encode `data` for `format`: a raw forward block/stream the embedded decruncher decodes.
/// LZAN-min's stream carries a one-byte mode header that its 6502 decoder does not read, so it is
/// stripped here.
pub fn encode(format: PackFormat, data: &[u8]) -> Vec<u8> {
    match format {
        PackFormat::Lzsa1 => lzan::lzsa1::compress(data, lzan::lzsa1::MAX_LEVEL, false),
        PackFormat::Lzsa2 => lzan::lzsa2::compress(data, lzan::lzsa2::MAX_LEVEL, false),
        PackFormat::Zx0 => lzan::zx0compat::compress(data, lzan::zx0compat::MAX_LEVEL, false),
        PackFormat::Zx02 => lzan::zx02::compress(data, lzan::zx02::MAX_LEVEL, false),
        PackFormat::LzanMin => {
            let s = lzan::zx::compress_min_eof_e(data, 3);
            if s.is_empty() { s } else { s[1..].to_vec() }
        }
        PackFormat::Bolt => lzan::bolt::compress(data, lzan::bolt::MAX_LEVEL, false),
        PackFormat::Bb2 => lzan::bb2::compress(data, lzan::bb2::MAX_LEVEL, false),
    }
}

/// Forward in-place safety gap (bytes) the packed stream needs: the peak by which the write head
/// leads the read head when decoding over a source that ends at the output's top. The converter
/// checks this against the RAM block's available headroom.
pub fn max_gap_forward(format: PackFormat, packed: &[u8]) -> usize {
    match format {
        PackFormat::Lzsa1 => lzan::lzsa1::max_gap_forward(packed),
        PackFormat::Lzsa2 => lzan::lzsa2::max_gap_forward(packed),
        PackFormat::Zx0 => lzan::zx0compat::max_gap_forward(packed),
        PackFormat::Zx02 => lzan::zx02::max_gap_forward(packed),
        PackFormat::LzanMin => lzan::zx::max_gap_min_forward(packed),
        PackFormat::Bolt => lzan::bolt::max_gap_forward(packed),
        PackFormat::Bb2 => lzan::bb2::max_gap_forward(packed),
    }
}

/// The embedded decruncher source, or `None` for LZSA1 (whose decoder is inline in the emitters).
pub fn decoder_source(format: PackFormat) -> Option<&'static str> {
    match format {
        PackFormat::Lzsa1 => None,
        PackFormat::Lzsa2 => Some(include_str!("decrunchers/lzsa2.s")),
        PackFormat::Zx0 => Some(include_str!("decrunchers/zx0.s")),
        PackFormat::Zx02 => Some(include_str!("decrunchers/zx02.s")),
        PackFormat::LzanMin => Some(include_str!("decrunchers/lzan-min.s")),
        PackFormat::Bolt => Some(include_str!("decrunchers/bolt-opt-speed.s")),
        PackFormat::Bb2 => Some(include_str!("decrunchers/byteboozer2.s")),
    }
}

/// The decruncher entry label the emitter `JSR`s / `JMP`s to.
pub fn entry_label(format: PackFormat) -> &'static str {
    match format {
        PackFormat::Lzsa1 => "decompress_lzsa1",
        _ => "full_decomp",
    }
}

/// Zero-page seed addresses for `format`.
pub fn zp_seed(format: PackFormat) -> ZpSeed {
    match format {
        PackFormat::Lzsa1 => ZpSeed { src_lo: 0xFC, dst_lo: 0xFE },
        PackFormat::Lzsa2 => ZpSeed { src_lo: 0xF8, dst_lo: 0xFA },
        PackFormat::Zx0 => ZpSeed { src_lo: 0xF8, dst_lo: 0xFA },
        PackFormat::Zx02 => ZpSeed { src_lo: 0xFB, dst_lo: 0xF9 },
        PackFormat::LzanMin => ZpSeed { src_lo: 0xF8, dst_lo: 0xFA },
        PackFormat::Bolt => ZpSeed { src_lo: 0xFA, dst_lo: 0xFC },
        PackFormat::Bb2 => ZpSeed { src_lo: 0xF8, dst_lo: 0xFA },
    }
}

/// The `LZSA_SRC_LO/HI` + `LZSA_DST_LO/HI` zero-page equates the seed sites use, pointed at this
/// format's pointer addresses. For a non-LZSA1 format the decoder body defines its own scratch
/// symbols; only the caller-seeded src/dst pointers need naming here. Returns `None` for LZSA1
/// (its emitter supplies its own equate block).
pub fn seed_equates(format: PackFormat) -> Option<String> {
    if format == PackFormat::Lzsa1 {
        return None;
    }
    let s = zp_seed(format);
    Some(format!(
        "LZSA_SRC_LO = ${:02X}\nLZSA_SRC_HI = ${:02X}\nLZSA_DST_LO = ${:02X}\nLZSA_DST_HI = ${:02X}",
        s.src_lo,
        s.src_lo + 1,
        s.dst_lo,
        s.dst_lo + 1,
    ))
}

/// The main decoder body to inline where the emitter `JSR`s it (RTS-terminated). `None` for LZSA1.
pub fn main_body(format: PackFormat) -> Option<String> {
    decoder_source(format).map(|s| s.to_string())
}

/// The relocated ($0100) decoder body, transferring to `block9` on completion. `None` for LZSA1.
///
/// Five formats use a `JSR <entry> / JMP block9` wrapper: page 1 survives the RAM decode (which
/// starts at $0200), so the decoder returns to the wrapper, which jumps to `block9`. This also
/// covers decoders whose terminal RTS is shared with a helper (e.g. ByteBoozer2). LZSA2 is too
/// large for the wrapper's extra stack level, so it uses its `;@exit` marker: the single-use body
/// drops the `;@reloc-drop` re-callability scrub and turns the terminal RTS into `JMP block9`.
pub fn relocated_body(format: PackFormat, block9: u16) -> Option<String> {
    let src = decoder_source(format)?;
    Some(if format == PackFormat::Lzsa2 {
        let mut out = String::new();
        for line in src.lines() {
            if line.contains(";@reloc-drop") {
                continue;
            }
            if line.contains(";@exit") {
                let indent: String = line.chars().take_while(|c| c.is_whitespace()).collect();
                out.push_str(&format!("{}JMP ${:04X}\n", indent, block9));
            } else {
                out.push_str(line);
                out.push('\n');
            }
        }
        out
    } else {
        format!(
            "    JSR {}\n    JMP ${:04X}\n{}",
            entry_label(format),
            block9,
            src
        )
    })
}
