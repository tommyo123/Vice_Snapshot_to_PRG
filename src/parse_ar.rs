//! Action Replay (and compatible) freeze-file front-end.
//!
//! Instead of statically decoding each freezer's packed format, we **replay the
//! freeze file's own 6510 restore stub** in a tiny sandbox CPU and read the
//! resulting machine state. The restore stub banks ROM out and runs entirely in
//! RAM + I/O, so no C64 ROM images are needed: we model 64K RAM, the $00/$01 CPU
//! port banking, and capture every write to the VIC/SID/CIA/color I/O registers,
//! then stop at the handoff `RTI` that returns to the frozen program. At that
//! point the RAM array + captured I/O + the RTI frame == the full `C64Snapshot`.
//!
//! This is freezer-agnostic: any freeze saved as a self-restoring autoboot PRG
//! (Action Replay 4/5/6, and others built the same way) is handled by one engine.
//!
// Copyright (c) 2025-2026 Tommy Olsen
// Licensed under the MIT License.

#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};

use crate::config::{InputMode, FreezeMethod};
use crate::parse_vsf::{C64Snapshot, Cpu6510, C64Mem, VicII, Cia6526, Sid6581};

/// Maximum instructions to emulate before giving up (restore stubs finish in <1M).
const MAX_INSTRUCTIONS: u64 = 8_000_000;

/// Heuristic: does this look like a self-restoring freezer snapshot we can replay?
///
/// The Action Replay family (MK3 through V8.4), its clones, and Freeze Machine
/// v1/v2 save a freeze as a BASIC-autoboot PRG (`10 SYS<addr>`) whose restore
/// stub begins with the exact sequence
///
/// ```text
/// SEI                 ; 78
/// LDA #$7F / STA $DD0D ; A9 7F 8D 0D DD   (mask CIA2 NMI)
/// LDA #$34 / STA $01   ; A9 34 85 01      (all-RAM banking for the depack)
/// ```
///
/// We detect that signature at the BASIC stub's SYS target. It is precise so it
/// won't match ordinary PRGs, and it rejects formats we cannot replay without
/// ROM images:
///   - AR MK2 (its stub is a multi-file KERNAL disk loader: `JSR $0994 ...`),
///   - Freeze Frame / ISEPIC / Niki (not SYS-autoboot freeze PRGs).
pub fn is_ar_freeze(bytes: &[u8]) -> bool {
    if bytes.len() < 0x20 {
        return false;
    }
    // PRG load address must be $0801 (BASIC).
    if bytes[0] != 0x01 || bytes[1] != 0x08 {
        return false;
    }
    let body = &bytes[2..];
    // Resolve the BASIC stub's SYS target and map it to a file offset.
    let target = match sys_target(body) {
        Some(t) if t >= 0x0801 => t,
        _ => return false,
    };
    let off = (target - 0x0801) as usize + 2; // file offset of the restore stub
    let stub = match bytes.get(off..off + 16) {
        Some(s) => s,
        None => return false,
    };
    // SEI at the entry, then STA $DD0D and STA $01 close behind.
    stub[0] == 0x78
        && stub.windows(3).take(8).any(|w| w == [0x8D, 0x0D, 0xDD])
        && stub.windows(2).take(12).any(|w| w == [0x85, 0x01])
}

/// Replay an Action Replay freeze file and reconstruct the frozen `C64Snapshot`.
pub fn snapshot_from_ar(path: &str) -> Result<C64Snapshot, String> {
    let data = fs::read(path).map_err(|e| format!("Failed to read freeze file: {}", e))?;
    snapshot_from_ar_bytes(&data)
}

/// As [`snapshot_from_ar`] but from an in-memory PRG image.
pub fn snapshot_from_ar_bytes(data: &[u8]) -> Result<C64Snapshot, String> {
    if data.len() < 3 {
        return Err("Freeze file too short".to_string());
    }
    let load = (data[0] as u16) | ((data[1] as u16) << 8);
    let body = &data[2..];

    let mut cpu = Cpu::new();
    for (i, &b) in body.iter().enumerate() {
        cpu.m.ram[(load.wrapping_add(i as u16)) as usize] = b;
    }

    // Entry = SYS target in the BASIC stub, else load address.
    let start = sys_target(body).unwrap_or(load);
    cpu.pc = start;
    cpu.sp = 0xFF;
    cpu.p = 0x24;

    let mut n: u64 = 0;
    while !cpu.halted && n < MAX_INSTRUCTIONS {
        cpu.step();
        n += 1;
    }

    let frame = cpu
        .rti_frame
        .ok_or_else(|| {
            format!(
                "Freeze restore did not reach a handoff RTI within {} instructions \
                 (last PC ${:04X}). This freezer format may be unsupported.",
                MAX_INSTRUCTIONS, cpu.pc
            )
        })?;
    let (frozen_pc, frozen_p) = frame;

    cpu.m.build_snapshot(&cpu, frozen_pc, frozen_p)
}

/* ===================== format detection + dispatch ===================== */

/// Which freezer produced a given file (those we can reconstruct a snapshot from).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FreezerKind {
    /// Self-restoring autoboot PRG, replayed in the ROM-less sandbox.
    /// Covers Action Replay MK3-V8.4 + clones, Super Snapshot 5, Freeze Machine,
    /// and the Expert Cartridge (which banks RAM via $00/$01 instead of $DD0D).
    SelfRestoringPrg,
    /// ISEPIC 2-file freeze (this is the "-name" data file): KERNAL-stubbed replay.
    Isepic,
    /// Final Cartridge III 2-file freeze (this is the `fc` stub; the `-fc`
    /// companion holds the compressed bulk). Replayed with the turbo getbyte stubbed.
    Fc3,
}

/// Expert Cartridge (Trilogic) freeze: a self-restoring autoboot PRG whose stub
/// banks all RAM out via the CPU port ($00 + $01) rather than masking $DD0D.
/// Fingerprinted by the embedded "TRILOGIC" string plus the SEI/STA $00/STA $01 entry.
pub fn is_expert_freeze(bytes: &[u8]) -> bool {
    if bytes.len() < 0x20 || bytes[0] != 0x01 || bytes[1] != 0x08 {
        return false;
    }
    let target = match sys_target(&bytes[2..]) {
        Some(t) if t >= 0x0801 => t,
        _ => return false,
    };
    let off = (target - 0x0801) as usize + 2;
    let stub = match bytes.get(off..off + 12) {
        Some(s) => s,
        None => return false,
    };
    // SEI, then LDA #imm / STA $00 and LDA #imm / STA $01 (bank all RAM out).
    stub[0] == 0x78
        && stub.windows(2).take(10).any(|w| w == [0x85, 0x00])
        && stub.windows(2).take(12).any(|w| w == [0x85, 0x01])
}

/// ISEPIC data file ("-name"): not a normal PRG — it begins with the restore stub
/// `LDX #$FF / TXS / LDY #$13 / LDA $0680,Y / STA $0119,Y / ...` that runs at $039C.
pub fn is_isepic_freeze(bytes: &[u8]) -> bool {
    const SIG: [u8; 11] = [0xA2, 0xFF, 0x9A, 0xA0, 0x13, 0xB9, 0x80, 0x06, 0x99, 0x19, 0x01];
    bytes.len() > 0x400 && bytes.starts_with(&SIG)
}

/// Final Cartridge III freeze `fc` stub: PRG at $0801, BASIC `1986 SYS2061`, then a
/// stub at $080D beginning `SEI / LDY #$00 / LDA ($BB),Y / STA $0C33,Y` (copies the
/// load filename). The `-fc` companion (compressed bulk) is loaded separately.
pub fn is_fc3_freeze(bytes: &[u8]) -> bool {
    const SIG: [u8; 8] = [0x78, 0xA0, 0x00, 0xB1, 0xBB, 0x99, 0x33, 0x0C];
    bytes.len() > 0x100
        && bytes[0] == 0x01
        && bytes[1] == 0x08
        && bytes.get(0x0E..0x16) == Some(&SIG)
}

/// Identify a freeze file we can convert, or `None`.
pub fn detect_freezer(bytes: &[u8]) -> Option<FreezerKind> {
    if is_ar_freeze(bytes) || is_expert_freeze(bytes) {
        Some(FreezerKind::SelfRestoringPrg)
    } else if is_isepic_freeze(bytes) {
        Some(FreezerKind::Isepic)
    } else if is_fc3_freeze(bytes) {
        Some(FreezerKind::Fc3)
    } else {
        None
    }
}

/// Reconstruct a `C64Snapshot` from any single-file freezer image (auto-detected).
///
/// FC3 is a 2-file format whose bulk lives in a separate `-fc` companion, so it
/// cannot be reconstructed from this single buffer — use
/// [`snapshot_from_freeze_file`], which locates the companion on disk.
pub fn snapshot_from_freeze_bytes(data: &[u8]) -> Result<C64Snapshot, String> {
    match detect_freezer(data) {
        Some(FreezerKind::SelfRestoringPrg) => snapshot_from_ar_bytes(data),
        Some(FreezerKind::Isepic) => snapshot_from_isepic_bytes(data),
        Some(FreezerKind::Fc3) => Err(
            "Final Cartridge III is a 2-file freeze; load it via a path so the \
             '-fc' companion can be found"
                .to_string(),
        ),
        None => Err("Unrecognized freeze format".to_string()),
    }
}

/// Reconstruct a `C64Snapshot` from a freezer file on disk (auto-detected).
///
/// Single-file freezes defer to [`snapshot_from_freeze_bytes`]; Final Cartridge
/// III additionally loads its `-fc` companion from the same directory.
pub fn snapshot_from_freeze_file(path: &str) -> Result<C64Snapshot, String> {
    let data = fs::read(path).map_err(|e| format!("Failed to read freeze file: {}", e))?;
    match detect_freezer(&data) {
        Some(FreezerKind::Fc3) => snapshot_fc3_from_path(path, &data),
        Some(_) => snapshot_from_freeze_bytes(&data),
        None => Err("Unrecognized freeze format".to_string()),
    }
}

/// Reconstruct a `C64Snapshot` forcing a specific freezer family (no detection).
/// Used by the explicit "Cartridge freeze → force method" path. Fails clearly if
/// the file isn't actually that format (the forced replay won't reach a handoff).
pub fn snapshot_from_freeze_file_forced(path: &str, kind: FreezerKind) -> Result<C64Snapshot, String> {
    let data = fs::read(path).map_err(|e| format!("Failed to read freeze file: {}", e))?;
    match kind {
        FreezerKind::SelfRestoringPrg => snapshot_from_ar_bytes(&data),
        FreezerKind::Isepic => snapshot_from_isepic_bytes(&data),
        FreezerKind::Fc3 => snapshot_fc3_from_path(path, &data),
    }
}

/// Load an FC3 `fc` stub's `-fc` companion from the same directory and reconstruct.
fn snapshot_fc3_from_path(path: &str, data: &[u8]) -> Result<C64Snapshot, String> {
    let companion_path = fc3_companion_path(path).ok_or_else(|| {
        format!(
            "FC3 freeze '{}' needs its '-fc' companion (the compressed bulk) \
             in the same directory, but none was found",
            path
        )
    })?;
    let companion = fs::read(&companion_path).map_err(|e| {
        format!(
            "Failed to read FC3 companion '{}': {}",
            companion_path.display(),
            e
        )
    })?;
    snapshot_from_fc3_bytes(data, &companion)
}

/// Human-friendly name for a detected freezer family.
pub fn freezer_label(k: FreezerKind) -> &'static str {
    match k {
        FreezerKind::SelfRestoringPrg => "Action Replay / Super Snapshot / Freeze Machine / Expert",
        FreezerKind::Isepic => "ISEPIC",
        FreezerKind::Fc3 => "Final Cartridge III",
    }
}

/// Append a hint to a VSF parse error when the bytes actually look like a freeze —
/// the user likely needs to switch the input type to "Cartridge freeze".
pub fn vsf_hint(err: impl std::fmt::Display, bytes: &[u8]) -> String {
    match detect_freezer(bytes) {
        Some(kind) => format!(
            "{}\n\nThis file looks like a {} cartridge freeze, not a VSF snapshot. \
             Set the input type to \"Cartridge freeze\".",
            err,
            freezer_label(kind)
        ),
        None => err.to_string(),
    }
}

/// What the converters should do with the input after applying the [`InputMode`].
pub enum FreezeOutcome {
    /// Parse the input as a VICE VSF snapshot.
    Vsf,
    /// Use this reconstructed cartridge-freeze snapshot.
    Freeze(C64Snapshot),
}

/// Resolve how to handle the input file for a given [`InputMode`]. Centralises the
/// VSF-vs-freeze decision so all three converters behave identically.
pub fn resolve_input(path: &str, bytes: &[u8], mode: InputMode) -> Result<FreezeOutcome, String> {
    match mode {
        InputMode::Vsf => Ok(FreezeOutcome::Vsf),
        InputMode::Auto => {
            if detect_freezer(bytes).is_some() {
                Ok(FreezeOutcome::Freeze(snapshot_from_freeze_file(path)?))
            } else {
                Ok(FreezeOutcome::Vsf)
            }
        }
        InputMode::Freeze(method) => {
            let snap = match method {
                FreezeMethod::Auto => snapshot_from_freeze_file(path)?,
                FreezeMethod::SelfRestoring => {
                    snapshot_from_freeze_file_forced(path, FreezerKind::SelfRestoringPrg)?
                }
                FreezeMethod::Isepic => {
                    snapshot_from_freeze_file_forced(path, FreezerKind::Isepic)?
                }
                FreezeMethod::Fc3 => snapshot_from_freeze_file_forced(path, FreezerKind::Fc3)?,
            };
            Ok(FreezeOutcome::Freeze(snap))
        }
    }
}

/// Find the `-fc` companion for an FC3 `fc` stub. The freezer names the bulk file
/// `-` + the program name; on a PC dump that is typically `-<name>.prg`, `-<name>`,
/// or `-<stem>.prg`. Returns the first candidate that exists next to the stub.
pub fn fc3_companion_path(stub_path: &str) -> Option<PathBuf> {
    let p = Path::new(stub_path);
    let dir = p.parent().unwrap_or_else(|| Path::new("."));
    let file_name = p.file_name()?.to_string_lossy().to_string();
    let stem = p.file_stem()?.to_string_lossy().to_string();

    let candidates = [
        format!("-{}", file_name),  // -fc.prg
        format!("-{}", stem),       // -fc
        format!("-{}.prg", stem),   // -fc.prg (when stub had no .prg)
    ];
    for c in candidates {
        let cand = dir.join(&c);
        if cand.is_file() {
            return Some(cand);
        }
    }
    None
}

/* ============================ ISEPIC replay ============================ */

/// Bytes the ISEPIC boot loader reads to $039C..$06FF before `JMP $039C`.
const ISEPIC_HEADER: usize = 0x0700 - 0x039C; // 868

/// Reconstruct a `C64Snapshot` from an ISEPIC data file by replaying its restore
/// code at $039C with the KERNAL byte-readers stubbed to stream the file. State is
/// captured at the handoff RTI, after the second stage (entered via `JMP $003E`,
/// which restores the frozen zero page and part of $C000-$FFFF) has completed.
pub fn snapshot_from_isepic_bytes(data: &[u8]) -> Result<C64Snapshot, String> {
    if data.len() <= ISEPIC_HEADER {
        return Err("ISEPIC data file too short".to_string());
    }
    let mut cpu = Cpu::new();
    // Load the header chunk the boot loader would have read to $039C..$06FF.
    for i in 0..ISEPIC_HEADER {
        cpu.m.ram[0x039C + i] = data[i];
    }
    // Zero-page state the 256-byte boot loader leaves behind.
    cpu.m.ram[0xAE] = 0x00;
    cpu.m.ram[0xAF] = 0x07; // store pointer = $0700
    cpu.m.ram[0x90] = 0x00; // KERNAL status
    cpu.m.ram[0xFF] = 0x01; // select the CHRIN-continue restore path
    cpu.m.ram[0xBA] = 0x08; // device 8
    cpu.m.ram[0xB7] = 0x00; // filename length 0
    cpu.m.ram[0xBB] = 0x00;
    cpu.m.ram[0xBC] = 0x02;
    cpu.pc = 0x039C;
    cpu.sp = 0xFF;
    cpu.p = 0x24;
    cpu.isepic = Some(IsepicIo { data: data.to_vec(), cursor: ISEPIC_HEADER });

    let mut n: u64 = 0;
    while !cpu.halted && n < MAX_INSTRUCTIONS {
        cpu.step();
        n += 1;
    }

    let (frozen_pc, frozen_p) = cpu.rti_frame.ok_or_else(|| {
        format!(
            "ISEPIC restore did not reach a handoff RTI within {} instructions (last PC ${:04X})",
            MAX_INSTRUCTIONS, cpu.pc
        )
    })?;
    // Capture RAM + I/O + CPU port at the handoff RTI — the restore is now fully
    // applied (the earlier $0092 point was mid-restore: it predates the second-stage
    // zero-page restore, so resuming from it gave a wrong ZP and the game crashed).
    cpu.m.build_snapshot(&cpu, frozen_pc, frozen_p)
}

/* ====================== Final Cartridge III replay ====================== */

/// FC3's restore runs several internal trampoline RTIs (e.g. the $0D57 depacker
/// stage) before the FINAL handoff RTI to the frozen game PC. RTIs whose target
/// lands inside the restore code keep the replay going; the first RTI targeting
/// outside this range is the handoff.
const FC3_RESTORE_RANGE: std::ops::RangeInclusive<u16> = 0x0200..=0x1075;

/// FC3 decompresses a large companion (tens of KB); allow more head-room than the
/// self-restoring stubs while still bounding a runaway (the range check stops the
/// replay at the handoff long before this).
const FC3_MAX_INSTRUCTIONS: u64 = 30_000_000;

/// Reconstruct a `C64Snapshot` from a Final Cartridge III freeze: replay the `fc`
/// stub, streaming the `-fc` companion through the custom turbo getbyte at $0200
/// and no-opping the KERNAL serial (drive-code upload). The stub's own Phase-1
/// (contiguous high load) + Phase-2 (RLE depack to $0403+) rebuild memory; the
/// teardown then re-reads the freeze's page-2/3 image via ACPTR ($FFA5, served from
/// the companion) and runs the frozen game's overlay-processing code, before the
/// handoff RTI to the frozen program where we read the resulting state.
///
/// Game RAM, page 2/3 and the structural zero page are reconstructed exactly.
/// A few bytes are not: raster/frame-timing display counters, dead stack below
/// SP, and the $00/$01 CPU-port monitor quirk. These do not affect resume;
/// reproducing them exactly would need cycle-accurate VIC/CIA emulation.
pub fn snapshot_from_fc3_bytes(fc: &[u8], companion: &[u8]) -> Result<C64Snapshot, String> {
    if fc.len() < 0x20 {
        return Err("FC3 `fc` stub too short".to_string());
    }
    let load = (fc[0] as u16) | ((fc[1] as u16) << 8);
    let body = &fc[2..];

    let mut cpu = Cpu::new();
    for (i, &b) in body.iter().enumerate() {
        cpu.m.ram[(load.wrapping_add(i as u16)) as usize] = b;
    }
    // Entry = the BASIC SYS target (SYS2061 -> $080D).
    cpu.pc = sys_target(body).unwrap_or(0x080D);
    cpu.sp = 0xFF;
    cpu.p = 0x24;
    // Boot-left zero-page state the autostart leaves for the stub's filename copy.
    cpu.m.ram[0xB7] = 0x02; // filename length
    cpu.m.ram[0xBB] = 0x00; // filename pointer = $0200
    cpu.m.ram[0xBC] = 0x02;
    cpu.fc3 = Some(IsepicIo {
        data: companion.to_vec(),
        cursor: 0,
    });
    // FC3's overlay-processing code polls the joystick; an idle bus ($FF) matches
    // real hardware, keeping the resulting game-state counters correct.
    cpu.m.fc3_idle_cia = true;

    let mut n: u64 = 0;
    let mut frozen: Option<(u16, u8)> = None;
    while n < FC3_MAX_INSTRUCTIONS {
        cpu.step();
        n += 1;
        if cpu.halted {
            match cpu.rti_frame {
                // Intermediate trampoline within the restore code: keep going.
                Some(frame) if FC3_RESTORE_RANGE.contains(&frame.0) => {
                    cpu.halted = false;
                    cpu.rti_frame = None;
                }
                // Handoff RTI to the frozen game PC.
                Some(frame) => {
                    frozen = Some(frame);
                    break;
                }
                // BRK / illegal opcode: abort (no valid handoff frame).
                None => break,
            }
        }
    }

    let (frozen_pc, frozen_p) = frozen.ok_or_else(|| {
        format!(
            "FC3 restore did not reach a handoff RTI within {} instructions (last PC ${:04X})",
            FC3_MAX_INSTRUCTIONS, cpu.pc
        )
    })?;

    // The final RAM image + captured I/O + the frozen registers from the handoff
    // RTI form the snapshot. At the handoff the CPU port ($00/$01) already holds
    // the frozen banking the stub restored, so no override is needed (unlike ISEPIC).
    cpu.m.build_snapshot(&cpu, frozen_pc, frozen_p)
}

/// Parse the decimal operand after the BASIC SYS token ($9E) → entry address.
fn sys_target(body: &[u8]) -> Option<u16> {
    let idx = body.iter().position(|&b| b == 0x9E)?;
    let mut j = idx + 1;
    let mut num: u32 = 0;
    let mut any = false;
    while j < body.len() && (0x30..=0x39).contains(&body[j]) {
        num = num * 10 + (body[j] - 0x30) as u32;
        j += 1;
        any = true;
    }
    if any && num <= 0xFFFF {
        Some(num as u16)
    } else {
        None
    }
}

/* ============================ memory + I/O ============================ */

#[derive(Clone)]
struct Mem {
    ram: Box<[u8; 65536]>,
    port: u8, // $01 data
    ddr: u8,  // $00 direction
    vic: [u8; 64],
    sid: [u8; 32],
    color: [u8; 1024],
    cia1: [u8; 16],
    cia2: [u8; 16],
    vic_written: [bool; 64],
    sid_written: [bool; 32],
    cia1_ier: u8, // effective enabled-IRQ mask (set/clear semantics applied)
    cia2_ier: u8,
    tick: u64,
    /// FC3 only: read CIA1 PRA/PRB ($DC00/$DC01) as an idle bus ($FF, no key /
    /// joystick) instead of the stored value. The frozen game polls the joystick
    /// during its restore-time overlay processing; an idle bus matches real
    /// hardware, keeping the resulting game-state counters correct.
    fc3_idle_cia: bool,
}

impl Mem {
    fn new() -> Self {
        Mem {
            ram: Box::new([0u8; 65536]),
            port: 0x37,
            ddr: 0x2F,
            vic: [0; 64],
            sid: [0; 32],
            color: [0; 1024],
            cia1: [0; 16],
            cia2: [0; 16],
            vic_written: [false; 64],
            sid_written: [false; 32],
            cia1_ier: 0,
            cia2_ier: 0,
            tick: 0,
            fc3_idle_cia: false,
        }
    }

    /// I/O at $D000-$DFFF iff CHAREN=1 AND (LORAM=1 OR HIRAM=1).
    #[inline]
    fn io_visible(&self) -> bool {
        let l = self.port & 1;
        let h = (self.port >> 1) & 1;
        let c = (self.port >> 2) & 1;
        c == 1 && (l == 1 || h == 1)
    }

    fn read(&mut self, a: u16) -> u8 {
        match a {
            0x0000 => self.ddr,
            0x0001 => self.port,
            0xD000..=0xDFFF if self.io_visible() => self.io_read(a),
            _ => self.ram[a as usize],
        }
    }

    fn write(&mut self, a: u16, v: u8) {
        match a {
            0x0000 => self.ddr = v,
            0x0001 => self.port = v,
            0xD000..=0xDFFF if self.io_visible() => self.io_write(a, v),
            _ => self.ram[a as usize] = v, // RAM, incl. RAM hidden under I/O when banked out
        }
    }

    fn io_read(&mut self, a: u16) -> u8 {
        // Synthetic raster from instruction tick: consistent within a loop
        // iteration, sweeps all lines over time, so raster wait-loops terminate.
        if a == 0xD011 || a == 0xD012 {
            let line = ((self.tick / 16) % 312) as u16;
            if a == 0xD012 {
                return (line & 0xFF) as u8;
            }
            return (self.vic[0x11] & 0x7F) | if line >= 256 { 0x80 } else { 0 };
        }
        match a {
            0xD000..=0xD02E => self.vic[(a - 0xD000) as usize],
            0xD400..=0xD41F => self.sid[(a - 0xD400) as usize],
            0xD800..=0xDBFF => self.color[(a - 0xD800) as usize] | 0xF0,
            0xDC00..=0xDC0F => {
                let r = (a - 0xDC00) as usize;
                match r {
                    0x0D => 0x00,
                    0x04..=0x07 => 0xFF,
                    // PRA/PRB: idle bus ($FF) under FC3 — see `fc3_idle_cia`.
                    0x00 | 0x01 if self.fc3_idle_cia => 0xFF,
                    _ => self.cia1[r],
                }
            }
            0xDD00..=0xDD0F => {
                let r = (a - 0xDD00) as usize;
                match r {
                    0x0D => 0x00,
                    0x04..=0x07 => 0xFF,
                    _ => self.cia2[r],
                }
            }
            _ => 0xFF,
        }
    }

    fn io_write(&mut self, a: u16, v: u8) {
        match a {
            0xD000..=0xD03F => {
                let i = (a - 0xD000) as usize;
                self.vic[i] = v;
                self.vic_written[i] = true;
            }
            0xD400..=0xD41F => {
                let i = (a - 0xD400) as usize;
                self.sid[i] = v;
                self.sid_written[i] = true;
            }
            0xD800..=0xDBFF => {
                self.color[(a - 0xD800) as usize] = v & 0x0F;
            }
            0xDC00..=0xDC0F => {
                let r = (a - 0xDC00) as usize;
                self.cia1[r] = v;
                if r == 0x0D {
                    apply_icr(&mut self.cia1_ier, v);
                }
            }
            0xDD00..=0xDD0F => {
                let r = (a - 0xDD00) as usize;
                self.cia2[r] = v;
                if r == 0x0D {
                    apply_icr(&mut self.cia2_ier, v);
                }
            }
            _ => {} // mirrors / unhandled I/O ignored
        }
    }

    fn build_snapshot(&self, cpu: &Cpu, frozen_pc: u16, frozen_p: u8) -> Result<C64Snapshot, String> {
        let cpu_state = Cpu6510 {
            a: cpu.a,
            x: cpu.x,
            y: cpu.y,
            sp: cpu.sp,
            pc: frozen_pc,
            p: frozen_p,
        };

        let mem = C64Mem {
            cpu_port_data: self.port,
            cpu_port_dir: self.ddr,
            ram: self.ram.clone(),
        };

        let mut vic_regs = [0u8; 47];
        vic_regs.copy_from_slice(&self.vic[0..47]);
        let vic = VicII {
            registers: vic_regs,
            color_ram: Box::new(self.color),
        };

        let mut sid_regs = [0u8; 25];
        sid_regs.copy_from_slice(&self.sid[0..25]);
        let sid = Sid6581 { regs_25: sid_regs };

        let cia1 = self.cia_from(&self.cia1, self.cia1_ier);
        let cia2 = self.cia_from(&self.cia2, self.cia2_ier);

        Ok(C64Snapshot {
            cpu: cpu_state,
            mem,
            vic,
            cia1,
            cia2,
            sid,
        })
    }

    fn cia_from(&self, c: &[u8; 16], ier: u8) -> Cia6526 {
        let tal = (c[0x04] as u16) | ((c[0x05] as u16) << 8);
        let tbl = (c[0x06] as u16) | ((c[0x07] as u16) << 8);
        Cia6526 {
            ora: c[0x00],
            orb: if c[0x01] == 0x00 { 0xFF } else { c[0x01] },
            ddra: c[0x02],
            ddrb: c[0x03],
            tal,
            tbl,
            tac: tal, // counter starts from latch
            tbc: tbl,
            tod_10ths: c[0x08],
            tod_sec: c[0x09],
            tod_min: c[0x0A],
            tod_hr: c[0x0B],
            cra: c[0x0E],
            crb: c[0x0F],
            ier,
        }
    }
}

/// Apply a CIA ICR write ($DC0D/$DD0D): bit7 = set/clear, bits0-6 = mask.
#[inline]
fn apply_icr(mask: &mut u8, v: u8) {
    if v & 0x80 != 0 {
        *mask |= v & 0x7F;
    } else {
        *mask &= !(v & 0x7F);
    }
}

/* ============================ CPU ============================ */

const FN: u8 = 0x80;
const FV: u8 = 0x40;
const FB: u8 = 0x10;
const FD: u8 = 0x08;
const FI: u8 = 0x04;
const FZ: u8 = 0x02;
const FC: u8 = 0x01;

/// ISEPIC replay state: the data file fed through the stubbed KERNAL byte-readers.
struct IsepicIo {
    data: Vec<u8>,
    cursor: usize,
}

struct Cpu {
    a: u8,
    x: u8,
    y: u8,
    sp: u8,
    p: u8,
    pc: u16,
    m: Mem,
    halted: bool,
    rti_frame: Option<(u16, u8)>,
    /// When set, the CPU is replaying an ISEPIC freeze: KERNAL byte-readers
    /// (CHRIN/ACPTR) stream `data` and the disk-protocol calls are no-ops.
    isepic: Option<IsepicIo>,
    /// When set, the CPU is replaying a Final Cartridge III freeze: the custom
    /// turbo getbyte at $0200 streams the `-fc` companion and the KERNAL serial
    /// (drive-code upload) calls are no-ops.
    fc3: Option<IsepicIo>,
    /// FC3 ACPTR cursor: a SECOND read cursor over the `-fc` companion. After the
    /// turbo decompress, FC3's teardown re-reads the freeze's first 512 bytes via
    /// KERNAL ACPTR ($FFA5) to repopulate page 2/3 ($0200-$03FF); those bytes are
    /// the frozen page-2/3 image. Independent of the $0200 getbyte cursor.
    fc3_acptr: usize,
}

impl Cpu {
    fn new() -> Self {
        Cpu {
            a: 0,
            x: 0,
            y: 0,
            sp: 0xFF,
            p: 0x24,
            pc: 0,
            m: Mem::new(),
            halted: false,
            rti_frame: None,
            isepic: None,
            fc3: None,
            fc3_acptr: 0,
        }
    }

    /// Pop a return address and continue after it (used by KERNAL-routine stubs).
    fn rts(&mut self) {
        let lo = self.pull() as u16;
        let hi = self.pull() as u16;
        self.pc = (lo | (hi << 8)).wrapping_add(1);
    }

    /// ISEPIC CHRIN/ACPTR stub: return the next data-file byte; set KERNAL EOF
    /// (ST bit6) on the last real byte, matching the real KERNAL.
    fn isepic_readbyte(&mut self) {
        let (byte, eof) = {
            let io = self.isepic.as_mut().unwrap();
            if io.cursor < io.data.len() {
                let b = io.data[io.cursor];
                io.cursor += 1;
                (b, io.cursor >= io.data.len())
            } else {
                (0u8, true)
            }
        };
        self.a = byte;
        if eof {
            self.m.ram[0x90] |= 0x40;
        }
        self.rts();
    }

    /// FC3 turbo getbyte stub ($0200): return the next `-fc` companion byte in A,
    /// or 0 once exhausted (FC3's Phase-1 stops by its own counter, not on EOF).
    fn fc3_getbyte(&mut self) {
        let byte = {
            let io = self.fc3.as_mut().unwrap();
            if io.cursor < io.data.len() {
                let b = io.data[io.cursor];
                io.cursor += 1;
                b
            } else {
                0u8
            }
        };
        self.a = byte;
        self.rts();
    }

    /// FC3 ACPTR stub ($FFA5): the post-decompress teardown re-reads the freeze's
    /// page-2/3 image (the `-fc` companion's leading bytes) over the IEC bus. Serve
    /// it from the dedicated `fc3_acptr` cursor (256 bytes -> $0200-$02FF, then 256
    /// -> $0300-$03FF), clear carry (success), and RTS.
    fn fc3_acptr_byte(&mut self) {
        let cur = self.fc3_acptr;
        let (byte, len) = {
            let io = self.fc3.as_ref().unwrap();
            let b = if cur < io.data.len() { io.data[cur] } else { 0 };
            (b, io.data.len())
        };
        if cur < len {
            self.fc3_acptr = cur + 1;
        }
        self.a = byte;
        self.setf(FC, false);
        self.rts();
    }

    #[inline]
    fn set_zn(&mut self, v: u8) {
        self.p &= !(FZ | FN);
        if v == 0 {
            self.p |= FZ;
        }
        if v & 0x80 != 0 {
            self.p |= FN;
        }
    }
    #[inline]
    fn getf(&self, f: u8) -> u8 {
        if self.p & f != 0 { 1 } else { 0 }
    }
    #[inline]
    fn setf(&mut self, f: u8, on: bool) {
        if on { self.p |= f; } else { self.p &= !f; }
    }

    #[inline]
    fn push(&mut self, v: u8) {
        self.m.write(0x0100 + self.sp as u16, v);
        self.sp = self.sp.wrapping_sub(1);
    }
    #[inline]
    fn pull(&mut self) -> u8 {
        self.sp = self.sp.wrapping_add(1);
        self.m.read(0x0100 + self.sp as u16)
    }

    #[inline]
    fn fetch(&mut self) -> u8 {
        let v = self.m.read(self.pc);
        self.pc = self.pc.wrapping_add(1);
        v
    }
    #[inline]
    fn fetch16(&mut self) -> u16 {
        let lo = self.fetch() as u16;
        let hi = self.fetch() as u16;
        lo | (hi << 8)
    }

    // ---- addressing modes: return effective address ----
    #[inline]
    fn a_imm(&mut self) -> u16 {
        let a = self.pc;
        self.pc = self.pc.wrapping_add(1);
        a
    }
    #[inline]
    fn a_zp(&mut self) -> u16 {
        self.fetch() as u16
    }
    #[inline]
    fn a_zpx(&mut self) -> u16 {
        (self.fetch().wrapping_add(self.x)) as u16
    }
    #[inline]
    fn a_zpy(&mut self) -> u16 {
        (self.fetch().wrapping_add(self.y)) as u16
    }
    #[inline]
    fn a_abs(&mut self) -> u16 {
        self.fetch16()
    }
    #[inline]
    fn a_abx(&mut self) -> u16 {
        self.fetch16().wrapping_add(self.x as u16)
    }
    #[inline]
    fn a_aby(&mut self) -> u16 {
        self.fetch16().wrapping_add(self.y as u16)
    }
    #[inline]
    fn a_ind(&mut self) -> u16 {
        let p = self.fetch16();
        let lo = self.m.read(p) as u16;
        // 6502 indirect-JMP page-boundary bug
        let hi = self.m.read((p & 0xFF00) | ((p.wrapping_add(1)) & 0x00FF)) as u16;
        lo | (hi << 8)
    }
    #[inline]
    fn a_izx(&mut self) -> u16 {
        let z = self.fetch().wrapping_add(self.x);
        let lo = self.m.read(z as u16) as u16;
        let hi = self.m.read(z.wrapping_add(1) as u16) as u16;
        lo | (hi << 8)
    }
    #[inline]
    fn a_izy(&mut self) -> u16 {
        let z = self.fetch();
        let lo = self.m.read(z as u16) as u16;
        let hi = self.m.read(z.wrapping_add(1) as u16) as u16;
        (lo | (hi << 8)).wrapping_add(self.y as u16)
    }

    #[inline]
    fn branch(&mut self, cond: bool) {
        let off = self.fetch() as i8 as i16;
        if cond {
            self.pc = (self.pc as i16).wrapping_add(off) as u16;
        }
    }

    fn adc(&mut self, v: u8) {
        if self.p & FD != 0 {
            // BCD (restore stubs almost never use decimal mode)
            let mut lo = (self.a & 0x0F) as u16 + (v & 0x0F) as u16 + self.getf(FC) as u16;
            let mut hi = (self.a >> 4) as u16 + (v >> 4) as u16;
            if lo > 9 {
                lo += 6;
                hi += 1;
            }
            let mut res = (hi << 4) | (lo & 0x0F);
            self.setf(FC, hi > 9);
            if hi > 9 {
                res += 0x60;
            }
            self.a = (res & 0xFF) as u8;
            let a = self.a;
            self.set_zn(a);
            return;
        }
        let s = self.a as u16 + v as u16 + self.getf(FC) as u16;
        self.setf(FC, s > 0xFF);
        let r = (s & 0xFF) as u8;
        self.setf(FV, (!(self.a ^ v) & (self.a ^ r) & 0x80) != 0);
        self.a = r;
        self.set_zn(r);
    }
    #[inline]
    fn sbc(&mut self, v: u8) {
        self.adc(v ^ 0xFF);
    }
    #[inline]
    fn cmp_reg(&mut self, r: u8, v: u8) {
        self.setf(FC, r >= v);
        let t = r.wrapping_sub(v);
        self.set_zn(t);
    }

    // read-modify-write helpers
    fn asl_v(&mut self, v: u8) -> u8 {
        self.setf(FC, v & 0x80 != 0);
        let r = v << 1;
        self.set_zn(r);
        r
    }
    fn lsr_v(&mut self, v: u8) -> u8 {
        self.setf(FC, v & 1 != 0);
        let r = v >> 1;
        self.set_zn(r);
        r
    }
    fn rol_v(&mut self, v: u8) -> u8 {
        let nc = v & 0x80 != 0;
        let r = (v << 1) | self.getf(FC);
        self.setf(FC, nc);
        self.set_zn(r);
        r
    }
    fn ror_v(&mut self, v: u8) -> u8 {
        let nc = v & 1 != 0;
        let r = (v >> 1) | (self.getf(FC) << 7);
        self.setf(FC, nc);
        self.set_zn(r);
        r
    }

    #[inline]
    fn ld_a(&mut self, a: u16) { let v = self.m.read(a); self.a = v; self.set_zn(v); }
    #[inline]
    fn ld_x(&mut self, a: u16) { let v = self.m.read(a); self.x = v; self.set_zn(v); }
    #[inline]
    fn ld_y(&mut self, a: u16) { let v = self.m.read(a); self.y = v; self.set_zn(v); }

    fn step(&mut self) {
        // ISEPIC replay: capture the snapshot at $0092 (decompressor done) and
        // service the stubbed KERNAL routines.
        if self.isepic.is_some() {
            match self.pc {
                0xFFCF | 0xFFA5 => {
                    // CHRIN / ACPTR: stream the data file
                    self.m.tick += 1;
                    self.isepic_readbyte();
                    return;
                }
                // disk-protocol / setup calls: no-op (clc = success, then RTS)
                0xFFCC | 0xFFC3 | 0xFFC0 | 0xFFC6 | 0xFFC9 | 0xFFBA | 0xFFBD
                | 0xFFB1 | 0xFF93 | 0xFFA8 | 0xFFAE | 0xFFB4 | 0xFF96 | 0xFFAB
                | 0xFF8A => {
                    self.m.tick += 1;
                    self.setf(FC, false);
                    self.rts();
                    return;
                }
                _ => {}
            }
        }
        // FC3 replay: stream the `-fc` companion through the turbo getbyte at $0200
        // and no-op the KERNAL serial routines the drive-code upload calls.
        if self.fc3.is_some() {
            match self.pc {
                0x0200 => {
                    self.m.tick += 1;
                    self.fc3_getbyte();
                    return;
                }
                // ACPTR: serve the page-2/3 image from the `-fc` companion (NOT a
                // no-op — the teardown loads $0200-$03FF through it).
                0xFFA5 => {
                    self.m.tick += 1;
                    self.fc3_acptr_byte();
                    return;
                }
                0xFFB1 | 0xFF93 | 0xFFA8 | 0xFFAE | 0xFFAB | 0xFFB4 | 0xFF96
                | 0xFFBA | 0xFFBD | 0xFFC0 | 0xFFC6 | 0xFFC9 | 0xFFCC | 0xFFC3 | 0xFFCF
                | 0xFF8A | 0xFFE1 | 0xFFE4 | 0xFFB7 | 0xFFD2 => {
                    self.m.tick += 1;
                    self.setf(FC, false);
                    self.rts();
                    return;
                }
                _ => {}
            }
        }
        self.m.tick += 1;
        let op = self.fetch();
        match op {
            // ---- LDA ----
            0xA9 => { let a = self.a_imm(); self.ld_a(a); }
            0xA5 => { let a = self.a_zp(); self.ld_a(a); }
            0xB5 => { let a = self.a_zpx(); self.ld_a(a); }
            0xAD => { let a = self.a_abs(); self.ld_a(a); }
            0xBD => { let a = self.a_abx(); self.ld_a(a); }
            0xB9 => { let a = self.a_aby(); self.ld_a(a); }
            0xA1 => { let a = self.a_izx(); self.ld_a(a); }
            0xB1 => { let a = self.a_izy(); self.ld_a(a); }
            // ---- LDX ----
            0xA2 => { let a = self.a_imm(); self.ld_x(a); }
            0xA6 => { let a = self.a_zp(); self.ld_x(a); }
            0xB6 => { let a = self.a_zpy(); self.ld_x(a); }
            0xAE => { let a = self.a_abs(); self.ld_x(a); }
            0xBE => { let a = self.a_aby(); self.ld_x(a); }
            // ---- LDY ----
            0xA0 => { let a = self.a_imm(); self.ld_y(a); }
            0xA4 => { let a = self.a_zp(); self.ld_y(a); }
            0xB4 => { let a = self.a_zpx(); self.ld_y(a); }
            0xAC => { let a = self.a_abs(); self.ld_y(a); }
            0xBC => { let a = self.a_abx(); self.ld_y(a); }
            // ---- STA ----
            0x85 => { let a = self.a_zp(); self.m.write(a, self.a); }
            0x95 => { let a = self.a_zpx(); self.m.write(a, self.a); }
            0x8D => { let a = self.a_abs(); self.m.write(a, self.a); }
            0x9D => { let a = self.a_abx(); self.m.write(a, self.a); }
            0x99 => { let a = self.a_aby(); self.m.write(a, self.a); }
            0x81 => { let a = self.a_izx(); self.m.write(a, self.a); }
            0x91 => { let a = self.a_izy(); self.m.write(a, self.a); }
            // ---- STX / STY ----
            0x86 => { let a = self.a_zp(); self.m.write(a, self.x); }
            0x96 => { let a = self.a_zpy(); self.m.write(a, self.x); }
            0x8E => { let a = self.a_abs(); self.m.write(a, self.x); }
            0x84 => { let a = self.a_zp(); self.m.write(a, self.y); }
            0x94 => { let a = self.a_zpx(); self.m.write(a, self.y); }
            0x8C => { let a = self.a_abs(); self.m.write(a, self.y); }
            // ---- ADC ----
            0x69 => { let a = self.a_imm(); let v = self.m.read(a); self.adc(v); }
            0x65 => { let a = self.a_zp(); let v = self.m.read(a); self.adc(v); }
            0x75 => { let a = self.a_zpx(); let v = self.m.read(a); self.adc(v); }
            0x6D => { let a = self.a_abs(); let v = self.m.read(a); self.adc(v); }
            0x7D => { let a = self.a_abx(); let v = self.m.read(a); self.adc(v); }
            0x79 => { let a = self.a_aby(); let v = self.m.read(a); self.adc(v); }
            0x61 => { let a = self.a_izx(); let v = self.m.read(a); self.adc(v); }
            0x71 => { let a = self.a_izy(); let v = self.m.read(a); self.adc(v); }
            // ---- SBC ----
            0xE9 => { let a = self.a_imm(); let v = self.m.read(a); self.sbc(v); }
            0xE5 => { let a = self.a_zp(); let v = self.m.read(a); self.sbc(v); }
            0xF5 => { let a = self.a_zpx(); let v = self.m.read(a); self.sbc(v); }
            0xED => { let a = self.a_abs(); let v = self.m.read(a); self.sbc(v); }
            0xFD => { let a = self.a_abx(); let v = self.m.read(a); self.sbc(v); }
            0xF9 => { let a = self.a_aby(); let v = self.m.read(a); self.sbc(v); }
            0xE1 => { let a = self.a_izx(); let v = self.m.read(a); self.sbc(v); }
            0xF1 => { let a = self.a_izy(); let v = self.m.read(a); self.sbc(v); }
            // ---- AND ----
            0x29 => { let a = self.a_imm(); self.a &= self.m.read(a); let r=self.a; self.set_zn(r); }
            0x25 => { let a = self.a_zp(); self.a &= self.m.read(a); let r=self.a; self.set_zn(r); }
            0x35 => { let a = self.a_zpx(); self.a &= self.m.read(a); let r=self.a; self.set_zn(r); }
            0x2D => { let a = self.a_abs(); self.a &= self.m.read(a); let r=self.a; self.set_zn(r); }
            0x3D => { let a = self.a_abx(); self.a &= self.m.read(a); let r=self.a; self.set_zn(r); }
            0x39 => { let a = self.a_aby(); self.a &= self.m.read(a); let r=self.a; self.set_zn(r); }
            0x21 => { let a = self.a_izx(); self.a &= self.m.read(a); let r=self.a; self.set_zn(r); }
            0x31 => { let a = self.a_izy(); self.a &= self.m.read(a); let r=self.a; self.set_zn(r); }
            // ---- ORA ----
            0x09 => { let a = self.a_imm(); self.a |= self.m.read(a); let r=self.a; self.set_zn(r); }
            0x05 => { let a = self.a_zp(); self.a |= self.m.read(a); let r=self.a; self.set_zn(r); }
            0x15 => { let a = self.a_zpx(); self.a |= self.m.read(a); let r=self.a; self.set_zn(r); }
            0x0D => { let a = self.a_abs(); self.a |= self.m.read(a); let r=self.a; self.set_zn(r); }
            0x1D => { let a = self.a_abx(); self.a |= self.m.read(a); let r=self.a; self.set_zn(r); }
            0x19 => { let a = self.a_aby(); self.a |= self.m.read(a); let r=self.a; self.set_zn(r); }
            0x01 => { let a = self.a_izx(); self.a |= self.m.read(a); let r=self.a; self.set_zn(r); }
            0x11 => { let a = self.a_izy(); self.a |= self.m.read(a); let r=self.a; self.set_zn(r); }
            // ---- EOR ----
            0x49 => { let a = self.a_imm(); self.a ^= self.m.read(a); let r=self.a; self.set_zn(r); }
            0x45 => { let a = self.a_zp(); self.a ^= self.m.read(a); let r=self.a; self.set_zn(r); }
            0x55 => { let a = self.a_zpx(); self.a ^= self.m.read(a); let r=self.a; self.set_zn(r); }
            0x4D => { let a = self.a_abs(); self.a ^= self.m.read(a); let r=self.a; self.set_zn(r); }
            0x5D => { let a = self.a_abx(); self.a ^= self.m.read(a); let r=self.a; self.set_zn(r); }
            0x59 => { let a = self.a_aby(); self.a ^= self.m.read(a); let r=self.a; self.set_zn(r); }
            0x41 => { let a = self.a_izx(); self.a ^= self.m.read(a); let r=self.a; self.set_zn(r); }
            0x51 => { let a = self.a_izy(); self.a ^= self.m.read(a); let r=self.a; self.set_zn(r); }
            // ---- CMP ----
            0xC9 => { let a = self.a_imm(); let v=self.m.read(a); self.cmp_reg(self.a, v); }
            0xC5 => { let a = self.a_zp(); let v=self.m.read(a); self.cmp_reg(self.a, v); }
            0xD5 => { let a = self.a_zpx(); let v=self.m.read(a); self.cmp_reg(self.a, v); }
            0xCD => { let a = self.a_abs(); let v=self.m.read(a); self.cmp_reg(self.a, v); }
            0xDD => { let a = self.a_abx(); let v=self.m.read(a); self.cmp_reg(self.a, v); }
            0xD9 => { let a = self.a_aby(); let v=self.m.read(a); self.cmp_reg(self.a, v); }
            0xC1 => { let a = self.a_izx(); let v=self.m.read(a); self.cmp_reg(self.a, v); }
            0xD1 => { let a = self.a_izy(); let v=self.m.read(a); self.cmp_reg(self.a, v); }
            // ---- CPX / CPY ----
            0xE0 => { let a = self.a_imm(); let v=self.m.read(a); self.cmp_reg(self.x, v); }
            0xE4 => { let a = self.a_zp(); let v=self.m.read(a); self.cmp_reg(self.x, v); }
            0xEC => { let a = self.a_abs(); let v=self.m.read(a); self.cmp_reg(self.x, v); }
            0xC0 => { let a = self.a_imm(); let v=self.m.read(a); self.cmp_reg(self.y, v); }
            0xC4 => { let a = self.a_zp(); let v=self.m.read(a); self.cmp_reg(self.y, v); }
            0xCC => { let a = self.a_abs(); let v=self.m.read(a); self.cmp_reg(self.y, v); }
            // ---- BIT ----
            0x24 => { let a = self.a_zp(); self.do_bit(a); }
            0x2C => { let a = self.a_abs(); self.do_bit(a); }
            // ---- INC / DEC ----
            0xE6 => { let a = self.a_zp(); self.rmw(a, |c,v| { let r=v.wrapping_add(1); c.set_zn(r); r }); }
            0xF6 => { let a = self.a_zpx(); self.rmw(a, |c,v| { let r=v.wrapping_add(1); c.set_zn(r); r }); }
            0xEE => { let a = self.a_abs(); self.rmw(a, |c,v| { let r=v.wrapping_add(1); c.set_zn(r); r }); }
            0xFE => { let a = self.a_abx(); self.rmw(a, |c,v| { let r=v.wrapping_add(1); c.set_zn(r); r }); }
            0xC6 => { let a = self.a_zp(); self.rmw(a, |c,v| { let r=v.wrapping_sub(1); c.set_zn(r); r }); }
            0xD6 => { let a = self.a_zpx(); self.rmw(a, |c,v| { let r=v.wrapping_sub(1); c.set_zn(r); r }); }
            0xCE => { let a = self.a_abs(); self.rmw(a, |c,v| { let r=v.wrapping_sub(1); c.set_zn(r); r }); }
            0xDE => { let a = self.a_abx(); self.rmw(a, |c,v| { let r=v.wrapping_sub(1); c.set_zn(r); r }); }
            // ---- ASL / LSR / ROL / ROR (memory) ----
            0x06 => { let a = self.a_zp(); self.rmw(a, Cpu::asl_v); }
            0x16 => { let a = self.a_zpx(); self.rmw(a, Cpu::asl_v); }
            0x0E => { let a = self.a_abs(); self.rmw(a, Cpu::asl_v); }
            0x1E => { let a = self.a_abx(); self.rmw(a, Cpu::asl_v); }
            0x46 => { let a = self.a_zp(); self.rmw(a, Cpu::lsr_v); }
            0x56 => { let a = self.a_zpx(); self.rmw(a, Cpu::lsr_v); }
            0x4E => { let a = self.a_abs(); self.rmw(a, Cpu::lsr_v); }
            0x5E => { let a = self.a_abx(); self.rmw(a, Cpu::lsr_v); }
            0x26 => { let a = self.a_zp(); self.rmw(a, Cpu::rol_v); }
            0x36 => { let a = self.a_zpx(); self.rmw(a, Cpu::rol_v); }
            0x2E => { let a = self.a_abs(); self.rmw(a, Cpu::rol_v); }
            0x3E => { let a = self.a_abx(); self.rmw(a, Cpu::rol_v); }
            0x66 => { let a = self.a_zp(); self.rmw(a, Cpu::ror_v); }
            0x76 => { let a = self.a_zpx(); self.rmw(a, Cpu::ror_v); }
            0x6E => { let a = self.a_abs(); self.rmw(a, Cpu::ror_v); }
            0x7E => { let a = self.a_abx(); self.rmw(a, Cpu::ror_v); }
            // ---- accumulator shifts ----
            0x0A => { let v = self.a; self.a = self.asl_v(v); }
            0x4A => { let v = self.a; self.a = self.lsr_v(v); }
            0x2A => { let v = self.a; self.a = self.rol_v(v); }
            0x6A => { let v = self.a; self.a = self.ror_v(v); }
            // ---- transfers ----
            0xAA => { self.x = self.a; let r=self.x; self.set_zn(r); }
            0xA8 => { self.y = self.a; let r=self.y; self.set_zn(r); }
            0x8A => { self.a = self.x; let r=self.a; self.set_zn(r); }
            0x98 => { self.a = self.y; let r=self.a; self.set_zn(r); }
            0xBA => { self.x = self.sp; let r=self.x; self.set_zn(r); }
            0x9A => { self.sp = self.x; }
            // ---- stack ----
            0x48 => { self.push(self.a); }
            0x68 => { let v = self.pull(); self.a = v; self.set_zn(v); }
            0x08 => { self.push(self.p | FB | 0x20); }
            0x28 => { let v = self.pull(); self.p = (v & !FB) | 0x20; }
            // ---- inc/dec regs ----
            0xE8 => { self.x = self.x.wrapping_add(1); let r=self.x; self.set_zn(r); }
            0xC8 => { self.y = self.y.wrapping_add(1); let r=self.y; self.set_zn(r); }
            0xCA => { self.x = self.x.wrapping_sub(1); let r=self.x; self.set_zn(r); }
            0x88 => { self.y = self.y.wrapping_sub(1); let r=self.y; self.set_zn(r); }
            // ---- flags / nop ----
            0xEA => {}
            0x18 => self.setf(FC, false),
            0x38 => self.setf(FC, true),
            0x58 => self.setf(FI, false),
            0x78 => self.setf(FI, true),
            0xD8 => self.setf(FD, false),
            0xF8 => self.setf(FD, true),
            0xB8 => self.setf(FV, false),
            // ---- jumps / branches / subroutines ----
            0x4C => { self.pc = self.a_abs(); }
            0x6C => { self.pc = self.a_ind(); }
            0x20 => {
                let target = self.a_abs();
                let ret = self.pc.wrapping_sub(1);
                self.push((ret >> 8) as u8);
                self.push((ret & 0xFF) as u8);
                self.pc = target;
            }
            0x60 => {
                let lo = self.pull() as u16;
                let hi = self.pull() as u16;
                self.pc = (lo | (hi << 8)).wrapping_add(1);
            }
            0x40 => {
                let p = self.pull();
                let lo = self.pull() as u16;
                let hi = self.pull() as u16;
                self.p = (p & !FB) | 0x20;
                self.pc = lo | (hi << 8);
                self.rti_frame = Some((self.pc, p));
                self.halted = true; // stop at the freeze handoff
            }
            0x00 => { self.halted = true; } // BRK
            0x10 => { let c = self.p & FN == 0; self.branch(c); }
            0x30 => { let c = self.p & FN != 0; self.branch(c); }
            0x50 => { let c = self.p & FV == 0; self.branch(c); }
            0x70 => { let c = self.p & FV != 0; self.branch(c); }
            0x90 => { let c = self.p & FC == 0; self.branch(c); }
            0xB0 => { let c = self.p & FC != 0; self.branch(c); }
            0xD0 => { let c = self.p & FZ == 0; self.branch(c); }
            0xF0 => { let c = self.p & FZ != 0; self.branch(c); }

            // ---- common undocumented opcodes ----
            0x1A | 0x3A | 0x5A | 0x7A | 0xDA | 0xFA => {} // NOP
            0x80 | 0x82 | 0x89 | 0xC2 | 0xE2 => { let _ = self.a_imm(); } // NOP #imm
            0x04 | 0x44 | 0x64 => { let _ = self.a_zp(); }
            0x14 | 0x34 | 0x54 | 0x74 | 0xD4 | 0xF4 => { let _ = self.a_zpx(); }
            0x0C => { let _ = self.a_abs(); }
            0x1C | 0x3C | 0x5C | 0x7C | 0xDC | 0xFC => { let _ = self.a_abx(); }
            // LAX
            0xA7 => { let a=self.a_zp(); let v=self.m.read(a); self.a=v; self.x=v; self.set_zn(v); }
            0xB7 => { let a=self.a_zpy(); let v=self.m.read(a); self.a=v; self.x=v; self.set_zn(v); }
            0xAF => { let a=self.a_abs(); let v=self.m.read(a); self.a=v; self.x=v; self.set_zn(v); }
            0xBF => { let a=self.a_aby(); let v=self.m.read(a); self.a=v; self.x=v; self.set_zn(v); }
            0xA3 => { let a=self.a_izx(); let v=self.m.read(a); self.a=v; self.x=v; self.set_zn(v); }
            0xB3 => { let a=self.a_izy(); let v=self.m.read(a); self.a=v; self.x=v; self.set_zn(v); }
            // SAX
            0x87 => { let a=self.a_zp(); self.m.write(a, self.a & self.x); }
            0x97 => { let a=self.a_zpy(); self.m.write(a, self.a & self.x); }
            0x8F => { let a=self.a_abs(); self.m.write(a, self.a & self.x); }
            0x83 => { let a=self.a_izx(); self.m.write(a, self.a & self.x); }
            // DCP
            0xC7 => { let a=self.a_zp(); self.dcp(a); }
            0xD7 => { let a=self.a_zpx(); self.dcp(a); }
            0xCF => { let a=self.a_abs(); self.dcp(a); }
            0xDF => { let a=self.a_abx(); self.dcp(a); }
            0xDB => { let a=self.a_aby(); self.dcp(a); }
            0xC3 => { let a=self.a_izx(); self.dcp(a); }
            0xD3 => { let a=self.a_izy(); self.dcp(a); }
            // ISC
            0xE7 => { let a=self.a_zp(); self.isc(a); }
            0xF7 => { let a=self.a_zpx(); self.isc(a); }
            0xEF => { let a=self.a_abs(); self.isc(a); }
            0xFF => { let a=self.a_abx(); self.isc(a); }
            0xFB => { let a=self.a_aby(); self.isc(a); }
            0xE3 => { let a=self.a_izx(); self.isc(a); }
            0xF3 => { let a=self.a_izy(); self.isc(a); }
            // SLO
            0x07 => { let a=self.a_zp(); self.slo(a); }
            0x17 => { let a=self.a_zpx(); self.slo(a); }
            0x0F => { let a=self.a_abs(); self.slo(a); }
            0x1F => { let a=self.a_abx(); self.slo(a); }
            0x1B => { let a=self.a_aby(); self.slo(a); }
            0x03 => { let a=self.a_izx(); self.slo(a); }
            0x13 => { let a=self.a_izy(); self.slo(a); }
            // RLA
            0x27 => { let a=self.a_zp(); self.rla(a); }
            0x37 => { let a=self.a_zpx(); self.rla(a); }
            0x2F => { let a=self.a_abs(); self.rla(a); }
            0x3F => { let a=self.a_abx(); self.rla(a); }
            0x3B => { let a=self.a_aby(); self.rla(a); }
            0x23 => { let a=self.a_izx(); self.rla(a); }
            0x33 => { let a=self.a_izy(); self.rla(a); }
            // SRE
            0x47 => { let a=self.a_zp(); self.sre(a); }
            0x57 => { let a=self.a_zpx(); self.sre(a); }
            0x4F => { let a=self.a_abs(); self.sre(a); }
            0x5F => { let a=self.a_abx(); self.sre(a); }
            0x5B => { let a=self.a_aby(); self.sre(a); }
            0x43 => { let a=self.a_izx(); self.sre(a); }
            0x53 => { let a=self.a_izy(); self.sre(a); }
            // RRA
            0x67 => { let a=self.a_zp(); self.rra(a); }
            0x77 => { let a=self.a_zpx(); self.rra(a); }
            0x6F => { let a=self.a_abs(); self.rra(a); }
            0x7F => { let a=self.a_abx(); self.rra(a); }
            0x7B => { let a=self.a_aby(); self.rra(a); }
            0x63 => { let a=self.a_izx(); self.rra(a); }
            0x73 => { let a=self.a_izy(); self.rra(a); }
            // ANC / ALR / SBX
            0x0B | 0x2B => { let a=self.a_imm(); self.a &= self.m.read(a); let r=self.a; self.set_zn(r); self.setf(FC, r & 0x80 != 0); }
            0x4B => { let a=self.a_imm(); self.a &= self.m.read(a); self.setf(FC, self.a & 1 != 0); self.a >>= 1; let r=self.a; self.set_zn(r); }
            0xCB => { let a=self.a_imm(); let v=self.m.read(a); let t=self.a & self.x; self.setf(FC, t>=v); self.x=t.wrapping_sub(v); let r=self.x; self.set_zn(r); }

            // Anything else: stop (unknown / KILL opcode reached after handoff).
            _ => { self.halted = true; }
        }
    }

    fn do_bit(&mut self, a: u16) {
        let v = self.m.read(a);
        self.setf(FZ, (self.a & v) == 0);
        self.setf(FN, v & 0x80 != 0);
        self.setf(FV, v & 0x40 != 0);
    }

    fn rmw<F: Fn(&mut Cpu, u8) -> u8>(&mut self, a: u16, f: F) {
        let v = self.m.read(a);
        let r = f(self, v);
        self.m.write(a, r);
    }

    fn dcp(&mut self, a: u16) {
        let v = self.m.read(a).wrapping_sub(1);
        self.m.write(a, v);
        self.cmp_reg(self.a, v);
    }
    fn isc(&mut self, a: u16) {
        let v = self.m.read(a).wrapping_add(1);
        self.m.write(a, v);
        self.sbc(v);
    }
    fn slo(&mut self, a: u16) {
        let mut v = self.m.read(a);
        self.setf(FC, v & 0x80 != 0);
        v <<= 1;
        self.m.write(a, v);
        self.a |= v;
        let r = self.a;
        self.set_zn(r);
    }
    fn rla(&mut self, a: u16) {
        let v0 = self.m.read(a);
        let nc = v0 & 0x80 != 0;
        let v = (v0 << 1) | self.getf(FC);
        self.setf(FC, nc);
        self.m.write(a, v);
        self.a &= v;
        let r = self.a;
        self.set_zn(r);
    }
    fn sre(&mut self, a: u16) {
        let mut v = self.m.read(a);
        self.setf(FC, v & 1 != 0);
        v >>= 1;
        self.m.write(a, v);
        self.a ^= v;
        let r = self.a;
        self.set_zn(r);
    }
    fn rra(&mut self, a: u16) {
        let v0 = self.m.read(a);
        let nc = v0 & 1 != 0;
        let v = (v0 >> 1) | (self.getf(FC) << 7);
        self.setf(FC, nc);
        self.m.write(a, v);
        self.adc(v);
    }
}
