//! Assembles a PRG loader, an EasyFlash CRT and a Magic Desk CRT for every pack format: the
//! format's decruncher is embedded, the seed sites resolve, and the relocated body fits page 1.
//! The generated 6502 code is not executed here.

use vice_snapshot_to_prg_converter::config::{Config, PackFormat};
use vice_snapshot_to_prg_converter::make_crt_asm::MakeCRTAsm;
use vice_snapshot_to_prg_converter::make_magic_desk_crt_asm::MakeMagicDeskCRTAsm;
use vice_snapshot_to_prg_converter::make_prg_asm::MakePRGAsm;
use vice_snapshot_to_prg_converter::pack_format;

fn write(path: &str, bytes: &[u8]) {
    std::fs::write(path, bytes).unwrap();
}

// A RAM image (65008 bytes) that compresses well enough to clear every format's in-place gap.
fn ram_image() -> Vec<u8> {
    let n = 0xFDF0;
    let mut v = vec![0u8; n];
    for a in 0x0000..0x0400 { v[a] = (a % 40) as u8; }
    for a in 0x0400..0x2C00 { v[a] = [0xA9u8, 0x8D, 0x20, 0x4C, 0x60, 0xA2, 0xE8, 0xD0, 0xCA, 0x85][a % 10]; }
    for a in 0x2C00..0x4C00 { v[a] = ((a * 37) & 0xff) as u8; }
    v
}

#[test]
fn every_format_assembles_a_prg() {
    let base = std::env::temp_dir().join(format!("vsf_fmt_test_{}", std::process::id()));
    std::fs::create_dir_all(&base).unwrap();
    let dir = base.to_str().unwrap();

    let color = vec![0x0Eu8; 1000];
    let vic = (0..47u8).collect::<Vec<_>>();
    let sid = vec![0x0Fu8; 29];
    let zp: Vec<u8> = (2..=0xF7u8).collect(); // $02..$F7
    let ram = ram_image();
    let cia = vec![0u8; 20];

    write(&format!("{dir}/cia1.bin"), &cia);
    write(&format!("{dir}/cia2.bin"), &cia);

    for fmt in PackFormat::all() {
        // Compress each block with this format (the emitter only .incbin's the bytes).
        write(&format!("{dir}/color.lzsa"), &pack_format::encode(fmt, &color));
        write(&format!("{dir}/vic.lzsa"), &pack_format::encode(fmt, &vic));
        write(&format!("{dir}/sid.lzsa"), &pack_format::encode(fmt, &sid));
        write(&format!("{dir}/zp.lzsa"), &pack_format::encode(fmt, &zp));
        write(&format!("{dir}/ram.lzsa"), &pack_format::encode(fmt, &ram));

        let cfg = Config::new(dir).with_pack_format(fmt);
        let maker = MakePRGAsm::new(
            &format!("{dir}/color.lzsa"),
            &format!("{dir}/vic.lzsa"),
            &format!("{dir}/sid.lzsa"),
            &format!("{dir}/cia1.bin"),
            &format!("{dir}/cia2.bin"),
            &format!("{dir}/zp.lzsa"),
            &format!("{dir}/ram.lzsa"),
            0xC000,
            [0u8; 8],
            &cfg,
        )
        .unwrap_or_else(|e| panic!("{}: MakePRGAsm::new failed: {e}", fmt.as_str()));

        let out = format!("{dir}/out_{}.prg", fmt.as_str());
        maker
            .generate_prg(&out)
            .unwrap_or_else(|e| panic!("{}: generate_prg failed: {e}", fmt.as_str()));

        let prg = std::fs::read(&out).unwrap();
        assert!(prg.len() > 2, "{}: PRG suspiciously small ({} bytes)", fmt.as_str(), prg.len());
        assert_eq!(&prg[0..2], &[0x01, 0x08], "{}: PRG load address should be $0801", fmt.as_str());

        let ram_packed_len = std::fs::read(format!("{dir}/ram.lzsa")).unwrap().len();

        // EasyFlash CRT: relocated decoder must assemble and fit page 1; main restore code must assemble.
        let crt = MakeCRTAsm::new(
            &format!("{dir}/color.lzsa"), &format!("{dir}/vic.lzsa"), &format!("{dir}/sid.lzsa"),
            &format!("{dir}/cia1.bin"), &format!("{dir}/cia2.bin"), &format!("{dir}/zp.lzsa"),
            0xC000, [0u8; 8], &cfg, 200, ram_packed_len, 100, 0,
        ).unwrap_or_else(|e| panic!("{}: MakeCRTAsm::new: {e}", fmt.as_str()));
        let reloc = crt.generate_relocated_decompressor()
            .unwrap_or_else(|e| panic!("{}: CRT relocated: {e}", fmt.as_str()));
        assert!(reloc.len() < 246, "{}: CRT relocated {} bytes >= 246", fmt.as_str(), reloc.len());
        let crt2 = MakeCRTAsm::new(
            &format!("{dir}/color.lzsa"), &format!("{dir}/vic.lzsa"), &format!("{dir}/sid.lzsa"),
            &format!("{dir}/cia1.bin"), &format!("{dir}/cia2.bin"), &format!("{dir}/zp.lzsa"),
            0xC000, [0u8; 8], &cfg, reloc.len(), ram_packed_len, 100, 0,
        ).unwrap();
        crt2.generate_restore_code_binary()
            .unwrap_or_else(|e| panic!("{}: CRT main code: {e}", fmt.as_str()));

        // Magic Desk CRT: same two checks.
        let md = MakeMagicDeskCRTAsm::new(
            &format!("{dir}/color.lzsa"), &format!("{dir}/vic.lzsa"), &format!("{dir}/sid.lzsa"),
            &format!("{dir}/cia1.bin"), &format!("{dir}/cia2.bin"), &format!("{dir}/zp.lzsa"),
            0xC000, [0u8; 8], &cfg, 200, ram_packed_len, 100, 100, 0,
        ).unwrap_or_else(|e| panic!("{}: MakeMagicDeskCRTAsm::new: {e}", fmt.as_str()));
        let md_reloc = md.generate_relocated_decompressor()
            .unwrap_or_else(|e| panic!("{}: MagicDesk relocated: {e}", fmt.as_str()));
        assert!(md_reloc.len() < 246, "{}: MagicDesk relocated {} bytes >= 246", fmt.as_str(), md_reloc.len());
        let md2 = MakeMagicDeskCRTAsm::new(
            &format!("{dir}/color.lzsa"), &format!("{dir}/vic.lzsa"), &format!("{dir}/sid.lzsa"),
            &format!("{dir}/cia1.bin"), &format!("{dir}/cia2.bin"), &format!("{dir}/zp.lzsa"),
            0xC000, [0u8; 8], &cfg, md_reloc.len(), ram_packed_len, 100, 100, 0,
        ).unwrap();
        md2.generate_restore_code_binary()
            .unwrap_or_else(|e| panic!("{}: MagicDesk main code: {e}", fmt.as_str()));
    }

    let _ = std::fs::remove_dir_all(&base);
}

/// The CRT emitters run their restore code at $0340 under $01=$35. The RAM scratch that the
/// non-LZSA1 formats decode the colour, VIC and SID blocks into must clear that code, stay below
/// the payload at the top of RAM, and stay out of the $D000-$DFFF I/O window, where reads return
/// hardware registers instead of stored bytes. The payload size is swept across the whole range
/// so the scratch is tested on both sides of the I/O window.
#[test]
fn crt_io_scratch_clears_code_payload_and_io() {
    const CODE_START: u16 = 0x0340;
    let bytes = pack_format::IO_SCRATCH_PAGES << 8;

    for fmt in PackFormat::all() {
        for code_size in [0x0200usize, 0x0800, 0x1000, 0x1800] {
            // Packed payload from 2 KB to 60 KB, covering the sizes that put the scratch near
            // the $D000-$DFFF I/O window.
            for packed in (2..=60).map(|kb| kb * 1024) {
                let end_data_start = 0x10000 - packed;
                let res = pack_format::check_io_scratch_fits(fmt, end_data_start, CODE_START, code_size);

                if fmt == PackFormat::Lzsa1 {
                    assert!(res.is_ok(), "LZSA1 decodes straight to I/O and needs no scratch");
                    continue;
                }

                // An accepted layout must give a scratch address clear of the code, the payload and I/O.
                if res.is_ok() {
                    let scratch = (pack_format::io_scratch_page_below(end_data_start) as usize) << 8;
                    let ctx = format!("{} code={code_size:#x} packed={packed}", fmt.as_str());
                    assert!(
                        scratch >= CODE_START as usize + code_size,
                        "{ctx}: scratch ${scratch:04X} overlaps the restore code"
                    );
                    assert!(
                        scratch + bytes <= end_data_start,
                        "{ctx}: scratch ${scratch:04X}+{bytes} overlaps the payload at ${end_data_start:04X}"
                    );
                    assert!(
                        scratch + bytes <= 0xD000 || scratch >= 0xE000,
                        "{ctx}: scratch ${scratch:04X}-${:04X} intersects the $D000-$DFFF I/O window",
                        scratch + bytes - 1
                    );
                }

                // A snapshot that leaves room for the scratch must be accepted.
                if packed <= 48 * 1024 && code_size <= 0x1000 {
                    res.unwrap_or_else(|e| {
                        panic!("{} code={code_size:#x} packed={packed}: rejected a valid layout: {e}", fmt.as_str())
                    });
                }
            }
        }
    }
}

/// Two decoders pin their scratch to a fixed page-1 address. The relocated body is assembled at
/// $0100, so it must stop short of that address or the decoder writes over its own code.
#[test]
fn page1_scratch_clears_the_relocated_body() {
    let base = std::env::temp_dir().join(format!("vsf_pg1_test_{}", std::process::id()));
    std::fs::create_dir_all(&base).unwrap();
    let dir = base.to_str().unwrap();
    let cia = vec![0u8; 20];
    write(&format!("{dir}/cia1.bin"), &cia);
    write(&format!("{dir}/cia2.bin"), &cia);
    for name in ["color", "vic", "sid", "zp"] {
        write(&format!("{dir}/{name}.lzsa"), &[0u8; 8]);
    }

    for fmt in PackFormat::all() {
        let Some(src) = pack_format::decoder_source(fmt) else { continue };

        // Every "<name> = $01xx" equate the decoder declares is scratch it writes at run time.
        let pinned: Vec<(String, usize)> = src
            .lines()
            .filter_map(|l| {
                let (name, rest) = l.split_once('=')?;
                let hex = rest.trim_start().strip_prefix("$01")?;
                let hex: String = hex.chars().take(2).collect();
                let lo = usize::from_str_radix(&hex, 16).ok()?;
                Some((name.trim().to_string(), 0x0100 + lo))
            })
            .collect();
        if pinned.is_empty() {
            continue;
        }

        let cfg = Config::new(dir).with_pack_format(fmt);
        let crt = MakeCRTAsm::new(
            &format!("{dir}/color.lzsa"), &format!("{dir}/vic.lzsa"), &format!("{dir}/sid.lzsa"),
            &format!("{dir}/cia1.bin"), &format!("{dir}/cia2.bin"), &format!("{dir}/zp.lzsa"),
            0xC000, [0u8; 8], &cfg, 200, 4096, 100, 0,
        ).unwrap();
        let body_end = 0x0100 + crt.generate_relocated_decompressor().unwrap().len();

        for (name, addr) in pinned {
            assert!(
                body_end <= addr,
                "{}: relocated body reaches ${body_end:04X} but {name} is pinned at ${addr:04X}",
                fmt.as_str()
            );
        }
    }

    let _ = std::fs::remove_dir_all(&base);
}

/// A payload that leaves no room for the scratch must be rejected.
#[test]
fn crt_io_scratch_rejects_an_impossible_layout() {
    for fmt in PackFormat::all() {
        // 63 KB packed: the payload starts at $0400, leaving nothing above the restore code.
        let res = pack_format::check_io_scratch_fits(fmt, 0x0400, 0x0340, 0x0800);
        if fmt == PackFormat::Lzsa1 {
            assert!(res.is_ok(), "LZSA1 decodes straight to I/O and needs no scratch");
        } else {
            assert!(res.is_err(), "{}: impossible layout accepted", fmt.as_str());
        }
    }
}
