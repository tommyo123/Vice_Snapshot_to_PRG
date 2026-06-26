//! Phase 1 smoke test: build a minimal EasyFlash cart that exercises libefs +
//! EAPI flash writing, independent of the snapshot-restore pipeline.
//!
//! Boot (ultimax) -> copy a stub to RAM -> 16K mode -> EFS_init / EFS_init_eapi
//! -> EFS_setnam("TEST") -> EFS_save 16 bytes. Border turns GREEN on success,
//! RED on error. The file lands in the rewritable area (banks 56-63).
//!
//! Run:  cargo run --example efs_smoke
//! Then attach the .crt in VICE with EasyFlashWriteCRT=1 and check $D020 / flash.

use vice_snapshot_to_prg_converter::asm_wrapper::assemble_to_bytes;
use vice_snapshot_to_prg_converter::crt_builder::{CRTBuilder, CartridgeType, BANK_SIZE_8K};
use vice_snapshot_to_prg_converter::ef_save;

fn main() -> Result<(), String> {
    // Ultimax boot entry @ $FC00 (bank 0 HIROM offset $1C00). Copies the main
    // routine from $FD00/$FE00 (cart) to $C000 RAM, then runs it from RAM (so it
    // can switch the cartridge mode without unmapping itself).
    let boot = r#"*=$FC00
    SEI
    CLD
    LDX #$FF
    TXS
    LDA #$37
    STA $01
    LDX #$00
copyloop:
    LDA $FD00,X
    STA $0C00,X
    INX
    BNE copyloop
    JMP $0C00
"#;

    // Main test @ $0C00 (low RAM is the only RAM mapped in ultimax mode; $C000 is
    // open bus there). Stored at bank 0 HIROM offset $1D00 = $FD00 ultimax.
    let main = r#"*=$0C00
    ; switch EasyFlash to 16K mode (running from RAM now, safe)
    LDA #$37
    STA $01
    LDA #$2F
    STA $00            ; set DDR so $01 takes effect
    LDA #$87
    STA $DE02
    LDA #$00
    STA $DE00

    ; Diagnostic: record each call's error code ($0F00..) and carry ($0F08..).
    JSR $8000          ; EFS_init
    LDA #$C2
    JSR $8003          ; EFS_init_eapi -> EAPI at $C200
    STA $0F00
    LDA #$00
    ROL
    STA $0F08

    JSR $800C          ; EFS_format
    STA $0F01
    LDA #$00
    ROL
    STA $0F09

    ; fill $3000-$300F with 0..15
    LDX #$00
fill:
    TXA
    STA $3000,X
    INX
    CPX #$10
    BNE fill

    ; EFS_setnam("TEST")
    LDA #$04
    LDX #<fname
    LDY #>fname
    JSR $DF06
    STA $0F02
    LDA #$00
    ROL
    STA $0F0A

    ; start address pointer $FB/$FC = $3000
    LDA #$00
    STA $FB
    LDA #$30
    STA $FC

    ; EFS_save: A=zp ptr ($FB), X=end lo, Y=end hi (end+1 = $3010)
    LDA #$FB
    LDX #$10
    LDY #$30
    JSR $DF24
    STA $0F03
    LDA #$00
    ROL
    STA $0F0B

    LDA #$05           ; border green regardless; read $0F00-$0F0B for results
    STA $D020
halt:
    JMP halt
fname:
    .byte $54,$45,$53,$54
"#;

    let boot_bytes = assemble_to_bytes(boot)?;
    let main_bytes = assemble_to_bytes(main)?;
    assert!(main_bytes.len() < 0x200, "main too big for the 512-byte copy");

    // Build bank 0 HIROM (8 KB):
    //   $0000-$17FF area0 (read-only) directory: $FF = empty
    //   $1800-$1AFF EAPI
    //   $1B00-$1B3F EF name + libefs config
    //   $1C00-      boot entry (ultimax $FC00)
    //   $1D00-      main test  (runs at $C000)
    //   $1FFA-$1FFF NMI/RESET/IRQ vectors
    let mut romh = [0xFFu8; BANK_SIZE_8K];
    let eapi = ef_save::eapi_code();
    romh[0x1800..0x1800 + eapi.len()].copy_from_slice(eapi);
    let cfg = ef_save::EfsConfig::with_top_rw_sector(56);
    let nc = ef_save::generate_efs_name_and_config("EFS SMOKE", &cfg);
    romh[0x1B00..0x1B00 + nc.len()].copy_from_slice(&nc);
    romh[0x1C00..0x1C00 + boot_bytes.len()].copy_from_slice(&boot_bytes);
    romh[0x1D00..0x1D00 + main_bytes.len()].copy_from_slice(&main_bytes);
    // vectors -> boot entry $FC00
    romh[0x1FFA] = 0x00; romh[0x1FFB] = 0xFC; // NMI
    romh[0x1FFC] = 0x00; romh[0x1FFD] = 0xFC; // RESET
    romh[0x1FFE] = 0x00; romh[0x1FFF] = 0xFC; // IRQ

    // Full 1 MB EasyFlash: every bank LOROM+ROMH erased ($FF), then bank 0.
    let mut crt = CRTBuilder::new(CartridgeType::EasyFlash, 64, "EFS SMOKE")?;
    let ff = [0xFFu8; BANK_SIZE_8K];
    for b in 0..64 {
        crt.clear_bank(b, 0xFF)?;        // LOROM
        crt.set_bank_romh(b, &ff)?;      // ROMH
    }
    // bank 0 LOROM = libefs library (loads at $8000)
    crt.fill_bank(0, ef_save::lib_efs_code(), 0)?;
    // bank 0 ROMH = our HIROM image
    crt.set_bank_romh(0, &romh)?;

    let out = "test_md/efs_smoke.crt";
    std::fs::create_dir_all("test_md").ok();
    let _ = std::fs::remove_file(out);
    crt.make_crt(out)?;
    println!("wrote {out}  (libefs {} B, eapi {} B, boot {} B, main {} B)",
        ef_save::lib_efs_code().len(), eapi.len(), boot_bytes.len(), main_bytes.len());
    Ok(())
}
