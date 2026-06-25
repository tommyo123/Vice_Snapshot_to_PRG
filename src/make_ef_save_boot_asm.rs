//! EasyFlash SAVE-variant boot ROM (bank 0 HIROM).
//!
//! Lays out bank 0 HIROM for a libefs SAVE cartridge:
//!   $0000-$17FF  read-only EFS directory (or $FF = empty)
//!   $1800-$1AFF  EAPI (768 bytes)
//!   $1B00-$1B3F  EF name + libefs config
//!   $1C00-       boot code (runs at $FC00 in ultimax)
//!   $1FFA-$1FFF  NMI / RESET / IRQ vectors -> boot
//!
//! Because bank 0 LOROM is occupied by the libefs library, the snapshot restore
//! payload lives in bank 1+. On reset (ultimax) the boot copies a small stub to
//! low RAM ($0200, the only RAM mapped in ultimax), the stub switches to 16K
//! mode and copies the restore code from bank 1 LOROM to $0340, then runs it.
//! The restore code (make_crt_asm, built with restore_start_bank=1) does the
//! rest and RTIs into the game.
//
// Copyright (c) 2025-2026 Tommy Olsen
// Licensed under the MIT License.

use crate::asm_wrapper::assemble_to_bytes;
use crate::crt_builder::BANK_SIZE_8K;

pub struct MakeEfSaveBootAsm {
    restore_code_size: usize,
    restore_start_bank: usize,
}

impl MakeEfSaveBootAsm {
    pub fn new(restore_code_size: usize, restore_start_bank: usize) -> Self {
        Self { restore_code_size, restore_start_bank }
    }

    /// Build the full 8 KB bank 0 HIROM image.
    ///
    /// `eapi` (768 bytes), `name_config` (64 bytes), and `efs_dir` (the read-only
    /// directory, up to $1800 bytes; empty -> filled with $FF) are placed at their
    /// fixed offsets around the boot code.
    pub fn generate_romh(
        &self,
        eapi: &[u8],
        name_config: &[u8],
        efs_dir: Option<&[u8]>,
    ) -> Result<[u8; BANK_SIZE_8K], String> {
        let mut romh = [0xFFu8; BANK_SIZE_8K];

        // read-only EFS directory at $0000-$17FF ($FF = empty by default)
        if let Some(dir) = efs_dir {
            let n = dir.len().min(0x1800);
            romh[0..n].copy_from_slice(&dir[..n]);
        }

        // EAPI at $1800
        if eapi.len() > 0x300 {
            return Err(format!("EAPI too large: {} bytes", eapi.len()));
        }
        romh[0x1800..0x1800 + eapi.len()].copy_from_slice(eapi);

        // name + config at $1B00
        if name_config.len() > 0x100 {
            return Err("name+config block too large".to_string());
        }
        romh[0x1B00..0x1B00 + name_config.len()].copy_from_slice(name_config);

        // boot ($FC00) + stub (stored at $1D00, runs at $0200)
        let boot = assemble_to_bytes(&self.boot_asm())?;
        let stub = assemble_to_bytes(&self.stub_asm())?;
        if boot.len() > 0x100 {
            return Err(format!("EF-save boot too large: {} bytes", boot.len()));
        }
        if stub.len() > 0x100 {
            return Err(format!("EF-save boot stub too large: {} bytes", stub.len()));
        }
        romh[0x1C00..0x1C00 + boot.len()].copy_from_slice(&boot);
        romh[0x1D00..0x1D00 + stub.len()].copy_from_slice(&stub);

        // vectors -> boot entry $FC00
        romh[0x1FFA] = 0x00; romh[0x1FFB] = 0xFC; // NMI
        romh[0x1FFC] = 0x00; romh[0x1FFD] = 0xFC; // RESET
        romh[0x1FFE] = 0x00; romh[0x1FFF] = 0xFC; // IRQ

        Ok(romh)
    }

    /// Boot entry at $FC00 (ultimax): init, copy the stub to $0200, run it.
    fn boot_asm(&self) -> String {
        r#"*=$FC00
    SEI
    CLD
    LDX #$FF
    TXS
    LDA #$37
    STA $01
    LDA #$2F
    STA $00
    LDA $DC0D
    LDA $DD0D
    LDA #$7F
    STA $DC0D
    STA $DD0D
    LDA #$00
    STA $D01A
    LDA #$FF
    STA $D019
    LDX #$00
copystub:
    LDA $FD00,X
    STA $0200,X
    INX
    BNE copystub
    JMP $0200
"#
        .to_string()
    }

    /// Stub at $0200 (low RAM, mapped in ultimax): switch to 16K, copy the
    /// restore code from the restore bank to $0340, then jump to it.
    fn stub_asm(&self) -> String {
        let pages = self.restore_code_size.div_ceil(256);
        format!(
            r#"*=$0200
    LDA #$37
    STA $01
    LDA #$87
    STA $DE02
    LDA #${bank:02X}
    STA $DE00
    LDA #$80
    STA $FC
    LDA #$00
    STA $FB
    LDA #$03
    STA $FE
    LDA #$40
    STA $FD
    LDA #${pages:02X}
    STA $F8
cp:
    LDA $F8
    BEQ done
    LDY #$00
cpb:
    LDA ($FB),Y
    STA ($FD),Y
    INY
    BNE cpb
    INC $FC
    INC $FE
    DEC $F8
    JMP cp
done:
    JMP $0340
"#,
            bank = self.restore_start_bank,
            pages = pages,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn romh_layout_is_correct() {
        let eapi = vec![0xAAu8; 768];
        let cfg = vec![0x5Au8; 64];
        let b = MakeEfSaveBootAsm::new(2000, 1);
        let romh = b.generate_romh(&eapi, &cfg, None).unwrap();
        // read-only dir defaults to $FF
        assert_eq!(romh[0x0000], 0xFF);
        // EAPI / config placed
        assert_eq!(romh[0x1800], 0xAA);
        assert_eq!(romh[0x1B00], 0x5A);
        // boot present at $1C00 (SEI), reset vector -> $FC00
        assert_eq!(romh[0x1C00], 0x78);
        assert_eq!(romh[0x1FFC], 0x00);
        assert_eq!(romh[0x1FFD], 0xFC);
    }
}
