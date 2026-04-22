//! C64 VSF parser and component extractor
//!
//! Parses VICE snapshot files and extracts CPU, memory, VIC, CIA, and SID state.
//!
// Copyright (c) 2025 Tommy Olsen
// Licensed under the MIT License.

#![allow(dead_code)]

use std::fs;
use std::io::{Cursor, Read, Write};
use std::path::Path;
use lzsa_sys::{compress_with_options, Options, Version, Mode, Quality};
use crate::config::Config;

/* ======================= Snapshot structures ======================= */

#[derive(Debug, Clone)]
pub struct C64Snapshot {
    pub cpu: Cpu6510,
    pub mem: C64Mem,
    pub vic: VicII,
    pub cia1: Cia6526,
    pub cia2: Cia6526,
    pub sid: Sid6581,
}

#[derive(Debug, Clone)]
pub struct Cpu6510 {
    pub a: u8,
    pub x: u8,
    pub y: u8,
    pub sp: u8,
    pub pc: u16,
    pub p: u8,
}

#[derive(Debug, Clone)]
pub struct C64Mem {
    pub cpu_port_data: u8,
    pub cpu_port_dir: u8,
    pub ram: Box<[u8; 65536]>,
}

#[derive(Debug, Clone)]
pub struct VicII {
    pub registers: [u8; 47],
    pub color_ram: Box<[u8; 1024]>,
}

#[derive(Debug, Clone)]
pub struct Cia6526 {
    pub ddra: u8,
    pub ddrb: u8,
    pub ora: u8,
    pub orb: u8,
    pub tac: u16,      // Timer A Counter
    pub tbc: u16,      // Timer B Counter
    pub tal: u16,      // Timer A Latch
    pub tbl: u16,      // Timer B Latch
    pub tod_10ths: u8,
    pub tod_sec: u8,
    pub tod_min: u8,
    pub tod_hr: u8,
    pub cra: u8,
    pub crb: u8,
    pub ier: u8,
}

#[derive(Debug, Clone)]
pub struct Sid6581 {
    pub regs_25: [u8; 25],
}

/* ======================= Parser configuration ======================= */

#[derive(Debug, Clone, Default)]
pub struct ParserConfig {
    pub vic_regs_off: Option<usize>,
    pub vic_color_off: Option<usize>,
    pub sid_regs_off: Option<usize>,
}

impl ParserConfig {
    pub fn default_vice_like() -> Self {
        Self::default()
    }
}

/// Module header version (major, minor). Used to dispatch per-module layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ModuleVersion {
    major: u8,
    minor: u8,
}

/// Emulator variant, selected by the machine name in the VSF header.
/// Only the VIC-II module layout differs between the two.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Machine {
    C64,
    C64Sc,
}

impl Machine {
    fn from_name(name: &str) -> Option<Self> {
        match name {
            "C64" => Some(Self::C64),
            "C64SC" => Some(Self::C64Sc),
            _ => None,
        }
    }
}

/// Accept VSF file versions 1.1 and 2.0. Per-module layout dispatches further below.
fn check_file_version(major: u8, minor: u8) -> Result<(), String> {
    match (major, minor) {
        (1, 1) | (2, 0) => Ok(()),
        _ => Err(format!(
            "Unsupported snapshot format version {}.{}",
            major, minor
        )),
    }
}

/* ======================= VSF reader ======================= */

pub struct ParseVSF {
    raw: Vec<u8>,
    file_path: String,
    config: Config,
}

impl ParseVSF {
    pub fn import(file_path: &str, config: &Config) -> Result<Self, Box<dyn std::error::Error>> {
        let raw = fs::read(file_path)?;
        Ok(Self {
            raw,
            file_path: file_path.to_string(),
            config: config.clone(),
        })
    }

    pub fn parse_import(&self) -> Result<C64Snapshot, String> {
        self.parse_import_with(&ParserConfig::default_vice_like())
    }

    pub fn parse_import_with(&self, cfg: &ParserConfig) -> Result<C64Snapshot, String> {
        let mut cur = Cursor::new(self.raw.as_slice());

        // Read and validate VSF magic header (19 bytes: "VICE Snapshot File\x1A")
        let magic = read_fixed(&mut cur, 19)?;
        if !vsf_magic_ok(&magic) {
            let hint = sniff_compression_prefix(&magic)
                .map(|c| format!(" (looks like {}-compressed; decompress first)", c))
                .unwrap_or_default();
            return Err(format!("Not a VSF file{}", hint));
        }

        let vmaj = read_u8(&mut cur)?;
        let vmin = read_u8(&mut cur)?;

        check_file_version(vmaj, vmin)?;

        let mach = trim_nul(&read_fixed(&mut cur, 16)?).to_string();

        let machine = Machine::from_name(&mach)
            .ok_or_else(|| format!("Unsupported machine type '{}'", mach))?;

        // Newer snapshots insert a 21-byte "VICE Version" block here; older ones don't.
        let pos = cur.position() as usize;
        if self.raw.get(pos..pos + 12) == Some(b"VICE Version") {
            let _ = read_fixed(&mut cur, 21)?;
        }

        let mut cpu: Option<Cpu6510> = None;
        let mut mem: Option<C64Mem> = None;
        let mut vic: Option<VicII> = None;
        let mut cia1: Option<Cia6526> = None;
        let mut cia2: Option<Cia6526> = None;
        let mut sid: Option<Sid6581> = None;

        // Parse all modules (each has: name(16), major(1), minor(1), size(4), payload(size-22))
        while (cur.position() as usize) < self.raw.len() {
            let name_raw = match read_fixed_opt(&mut cur, 16) {
                Some(n) => n,
                None => break,
            };

            let name = trim_nul(&name_raw).to_string();
            let mmaj = read_u8(&mut cur)?;
            let mmin = read_u8(&mut cur)?;
            let size = read_u32(&mut cur)? as usize;

            // Calculate payload size (total size minus 22-byte module header)
            let payload_len = size.checked_sub(22)
                .ok_or_else(|| "Module size corrupt".to_string())?;
            let start = cur.position() as usize;
            let end = start + payload_len;

            if end > self.raw.len() {
                return Err(format!("Module '{}' beyond EOF", name));
            }

            let payload = &self.raw[start..end];
            cur.set_position(end as u64);

            let mver = ModuleVersion { major: mmaj, minor: mmin };
            match name.as_str() {
                "MAINCPU" => cpu = Some(parse_cpu(payload, mver)?),
                "C64MEM" => mem = Some(parse_memory(payload, mver)?),
                "VIC-II" => vic = Some(parse_vic(payload, cfg, machine, mver)?),
                "CIA1" => cia1 = Some(parse_cia(payload)?),
                "CIA2" => cia2 = Some(parse_cia(payload)?),
                "SID" => sid = Some(parse_sid(payload, cfg, mver)?),
                _ => {}  // Ignore unknown modules (e.g. DRIVE, PRINTER)
            }
        }

        let cpu = cpu.ok_or_else(|| "MAINCPU missing".to_string())?;
        validate_cpu(&cpu)?;

        let mem = mem.ok_or_else(|| "C64MEM missing".to_string())?;
        let mut vic = vic.ok_or_else(|| "VIC-II missing".to_string())?;
        let cia1 = cia1.ok_or_else(|| "CIA1 missing".to_string())?;
        let cia2 = cia2.ok_or_else(|| "CIA2 missing".to_string())?;
        let sid = sid.ok_or_else(|| "SID missing".to_string())?;

        // Extract Color RAM from main memory ($D800-$DBFF) instead of VIC module
        // The VIC module's color RAM is often unreliable, but main RAM $D800-$DBFF
        // contains the actual color RAM values that were active during snapshot
        let color_slice = &mem.ram[0xD800..=0xDBFF];

        // Validate color RAM data quality (should be 4-bit values in low nibble)
        let all_low_nibble = color_slice.iter().all(|&b| (b & 0xF0) == 0);
        let count_0 = color_slice.iter().filter(|&&b| b == 0x00).count();

        // Only use main memory color RAM if it looks valid (mostly non-zero, low nibble only)
        if all_low_nibble && count_0 < 900 {
            vic.color_ram = Box::new(
                color_slice.try_into()
                    .map_err(|_| "Color RAM slice conversion error".to_string())?
            );
        }

        Ok(C64Snapshot {
            cpu,
            mem,
            vic,
            cia1,
            cia2,
            sid,
        })
    }
    
    /// Extract components to separate files for compression and assembly
    /// Returns paths: (ram, color, zp, vic, sid, cia1, cia2)
    pub fn extract_ram(&self, snap: &C64Snapshot) -> Result<(String, String, String, String, String, String, String), Box<dyn std::error::Error>> {
        let path = Path::new(&self.file_path);
        let base_name = path.file_stem()
            .and_then(|s| s.to_str())
            .ok_or("Invalid filename")?;

        let work = self.config.work_str();
        let ram_hi_path = format!("{}/{}-ram.hi", work, base_name);
        let color_path = format!("{}/{}-color", work, base_name);
        let zp_path = format!("{}/{}-zp", work, base_name);
        let vic_path = format!("{}/{}-vic", work, base_name);
        let sid_path = format!("{}/{}-sid", work, base_name);
        let cia1_path = format!("{}/{}-cia1", work, base_name);
        let cia2_path = format!("{}/{}-cia2", work, base_name);

        let mut ram_file = fs::File::create(&ram_hi_path)?;
        ram_file.write_all(&snap.mem.ram[0x0200..=0xFFEF])?;

        let mut color_file = fs::File::create(&color_path)?;
        color_file.write_all(&snap.vic.color_ram[..])?;

        let mut zp_file = fs::File::create(&zp_path)?;
        zp_file.write_all(&snap.mem.ram[0x02..=0xF7])?;

        let mut vic_file = fs::File::create(&vic_path)?;
        vic_file.write_all(&snap.vic.registers)?;

        let mut sid_file = fs::File::create(&sid_path)?;
        sid_file.write_all(&snap.sid.regs_25)?;

        let mut cia1_file = fs::File::create(&cia1_path)?;
        cia1_file.write_all(&[
            snap.cia1.ora,
            snap.cia1.orb,
            snap.cia1.ddra,
            snap.cia1.ddrb,
            (snap.cia1.tal & 0xFF) as u8,
            (snap.cia1.tal >> 8) as u8,
            (snap.cia1.tbl & 0xFF) as u8,
            (snap.cia1.tbl >> 8) as u8,
            snap.cia1.tod_10ths,
            snap.cia1.tod_sec,
            snap.cia1.tod_min,
            snap.cia1.tod_hr,
            0x00,  // SDR
            snap.cia1.ier,
            snap.cia1.cra,
            snap.cia1.crb,
            (snap.cia1.tac & 0xFF) as u8,
            (snap.cia1.tac >> 8) as u8,
            (snap.cia1.tbc & 0xFF) as u8,
            (snap.cia1.tbc >> 8) as u8,
        ])?;

        let mut cia2_file = fs::File::create(&cia2_path)?;
        cia2_file.write_all(&[
            snap.cia2.ora,
            snap.cia2.orb,
            snap.cia2.ddra,
            snap.cia2.ddrb,
            (snap.cia2.tal & 0xFF) as u8,
            (snap.cia2.tal >> 8) as u8,
            (snap.cia2.tbl & 0xFF) as u8,
            (snap.cia2.tbl >> 8) as u8,
            snap.cia2.tod_10ths,
            snap.cia2.tod_sec,
            snap.cia2.tod_min,
            snap.cia2.tod_hr,
            0x00,  // SDR
            snap.cia2.ier,
            snap.cia2.cra,
            snap.cia2.crb,
            (snap.cia2.tac & 0xFF) as u8,
            (snap.cia2.tac >> 8) as u8,
            (snap.cia2.tbc & 0xFF) as u8,
            (snap.cia2.tbc >> 8) as u8,
        ])?;

        Ok((ram_hi_path, color_path, zp_path, vic_path, sid_path, cia1_path, cia2_path))
    }

    pub fn compress_lzsa(&self, in_path: &str, out_path: &str) -> Result<(), Box<dyn std::error::Error>> {
        let input_data = fs::read(in_path)?;

        // Configure LZSA1 with raw mode (no frame header)
        let options = Options {
            version: Version::V1,
            mode: Mode::RawForward,
            quality: Quality::Ratio,
            min_match_size: 3,
        };

        let compressed = compress_with_options(&input_data, &options)
            .map_err(|e| format!("LZSA compression failed: {}", e))?;

        fs::write(out_path, &compressed)?;

        Ok(())
    }
}

/* ======================= Module parsers ======================= */

fn parse_cpu(payload: &[u8], mver: ModuleVersion) -> Result<Cpu6510, String> {
    // MAINCPU CLOCK field: 4 bytes up to module 1.1, 8 bytes from 1.3 onward.
    let clock_size: usize = if mver.minor >= 3 { 8 } else { 4 };

    let mut c = Cursor::new(payload);
    let _ = read_fixed(&mut c, clock_size)
        .map_err(|_| format!("MAINCPU {}.{} too short for clock field", mver.major, mver.minor))?;

    let a = read_u8(&mut c)?;
    let x = read_u8(&mut c)?;
    let y = read_u8(&mut c)?;
    let sp = read_u8(&mut c)?;
    let pc = read_u16(&mut c)?;
    let p = read_u8(&mut c)?;

    Ok(Cpu6510 { a, x, y, sp, pc, p })
}

fn parse_memory(payload: &[u8], _mver: ModuleVersion) -> Result<C64Mem, String> {
    // C64MEM prefix (shared across 0.0 and 0.1):
    //   cpu_port_data(1), cpu_port_dir(1), exrom(1), game(1), ram(65536)
    // The 0.1 tail (pport bit6/bit7 decay) is skipped.
    if payload.len() < 4 + 65536 {
        return Err("C64MEM too short".to_string());
    }

    let mut c = Cursor::new(payload);
    let cpu_port_data = read_u8(&mut c)?;
    let cpu_port_dir = read_u8(&mut c)?;
    let _exrom = read_u8(&mut c)?;
    let _game = read_u8(&mut c)?;

    let ram_vec = read_fixed(&mut c, 65536)?;
    let ram_array: [u8; 65536] = ram_vec.try_into()
        .map_err(|_| "RAM size mismatch".to_string())?;
    let ram = Box::new(ram_array);

    Ok(C64Mem { cpu_port_data, cpu_port_dir, ram })
}

fn parse_vic(payload: &[u8], _cfg: &ParserConfig, machine: Machine, mver: ModuleVersion) -> Result<VicII, String> {
    let (regs_off, color_off) = match machine {
        // Cycle-accurate VIC-II: model(1), regs(0x40), ..., cram(0x400).
        // cram offset depends on trigger_cycle width (32-bit vs CLOCK).
        Machine::C64Sc => (1usize, if mver.minor >= 3 { 761 } else { 757 }),

        // Non-cycle-accurate VIC-II: flags+cbuf first, then cram, then regs.
        Machine::C64 => (1119usize, 43usize),
    };

    if payload.len() < regs_off + 47 {
        return Err(format!(
            "VIC-II {}.{} too small for registers ({} bytes)",
            mver.major, mver.minor, payload.len()
        ));
    }
    if payload.len() < color_off + 1024 {
        return Err(format!(
            "VIC-II {}.{} too small for color RAM (need {}, got {})",
            mver.major, mver.minor, color_off + 1024, payload.len()
        ));
    }

    let color_ram_array: [u8; 1024] = payload[color_off..color_off + 1024]
        .try_into()
        .map_err(|_| "Color RAM slice error".to_string())?;

    let registers: [u8; 47] = payload[regs_off..regs_off + 47]
        .try_into()
        .map_err(|_| "VIC regs slice error".to_string())?;

    Ok(VicII {
        registers,
        color_ram: Box::new(color_ram_array),
    })
}

fn parse_cia(payload: &[u8]) -> Result<Cia6526, String> {
    let mut c = Cursor::new(payload);

    let ora = read_u8(&mut c)?;
    let orb = read_u8(&mut c)?;
    let ddra = read_u8(&mut c)?;
    let ddrb = read_u8(&mut c)?;
    let tac = read_u16(&mut c)?;
    let tbc = read_u16(&mut c)?;
    let tod_10ths = read_u8(&mut c)?;
    let tod_sec = read_u8(&mut c)?;
    let tod_min = read_u8(&mut c)?;
    let tod_hr = read_u8(&mut c)?;
    let _sdr = read_u8(&mut c)?;
    let ier = read_u8(&mut c)?;
    let cra = read_u8(&mut c)?;
    let crb = read_u8(&mut c)?;
    let tal = read_u16(&mut c)?;
    let tbl = read_u16(&mut c)?;

    // Fix PRB if zero (key pressed during snapshot)
    let orb_fixed = if orb == 0x00 { 0xFF } else { orb };

    Ok(Cia6526 {
        ddra,
        ddrb,
        ora,
        orb: orb_fixed,
        tac,
        tbc,
        tal,
        tbl,
        tod_10ths,
        tod_sec,
        tod_min,
        tod_hr,
        cra,
        crb,
        ier
    })
}

fn parse_sid(payload: &[u8], _cfg: &ParserConfig, mver: ModuleVersion) -> Result<Sid6581, String> {
    // Primary SID module layout by minor version:
    //   1.0:    sound(1), engine(1), siddata(32)                       regs@2
    //           (or payload_len==1 for "sound off", or bare 32)
    //   1.1-3:  sids(1), sound(1), engine(1), siddata(32)              regs@3
    //   1.4+:   sids(1), sound(1), engine(1), model(1), siddata(32)    regs@4

    let slice_regs = |offset: usize| -> Result<[u8; 25], String> {
        if payload.len() < offset + 25 {
            return Err(format!(
                "SID {}.{} payload too short for registers at offset {} (len {})",
                mver.major, mver.minor, offset, payload.len()
            ));
        }
        payload[offset..offset + 25]
            .try_into()
            .map_err(|_| "SID regs slice error".to_string())
    };

    let regs_25: [u8; 25] = match (mver.major, mver.minor) {
        (1, 0) => match payload.len() {
            1 => [0u8; 25],                 // sound off, no register data
            n if n >= 34 => slice_regs(2)?, // sound, engine, siddata
            n if n >= 32 => slice_regs(0)?, // bare siddata fallback
            _ => return Err(format!("SID 1.0 payload too short (len {})", payload.len())),
        },
        (1, 1) | (1, 2) | (1, 3) => slice_regs(3)?,
        _ => slice_regs(4)?,
    };

    Ok(Sid6581 { regs_25 })
}

/* ======================= Validation ======================= */

fn validate_cpu(_c: &Cpu6510) -> Result<(), String> {
    Ok(())
}

/* ======================= Restore toolkit (unused but kept for reference) ======================= */

pub trait Bus {
    fn write8(&mut self, addr: u16, val: u8);
    fn read8(&mut self, addr: u16) -> u8 {
        let _ = addr;
        0
    }
}

pub trait CpuControl {
    fn set_cpu(&mut self, a: u8, x: u8, y: u8, sp: u8, p: u8, pc: u16);
}

const CIA1_BASE: u16 = 0xDC00;
const CIA2_BASE: u16 = 0xDD00;

pub fn restore_cia(b: &mut impl Bus, base: u16, s: &Cia6526) {
    b.write8(base + 0x0E, 0x00);
    b.write8(base + 0x0F, 0x00);
    b.write8(base + 0x02, s.ddra);
    b.write8(base + 0x03, s.ddrb);
    b.write8(base + 0x00, s.ora);
    b.write8(base + 0x01, s.orb);
    b.write8(base + 0x04, (s.tal & 0x00FF) as u8);
    b.write8(base + 0x05, (s.tal >> 8) as u8);
    b.write8(base + 0x06, (s.tbl & 0x00FF) as u8);
    b.write8(base + 0x07, (s.tbl >> 8) as u8);
    b.write8(base + 0x0E, 0x10);
    b.write8(base + 0x0E, 0x00);
    b.write8(base + 0x0F, 0x10);
    b.write8(base + 0x0F, 0x00);
    b.write8(base + 0x0D, 0x7F);
    if (s.ier & 0x7F) != 0 {
        b.write8(base + 0x0D, 0x80 | (s.ier & 0x7F));
    }
    b.write8(base + 0x0E, s.cra & !0x10);
    b.write8(base + 0x0F, s.crb & !0x10);
}

pub fn restore_vic(b: &mut impl Bus, v: &VicII) {
    let base = 0xD000u16;
    for (i, &val) in v.registers.iter().enumerate() {
        b.write8(base + (i as u16), val);
    }
    let mut addr = 0xD800u16;
    for &c in v.color_ram.iter() {
        b.write8(addr, c & 0x0F);
        addr += 1;
    }
}

pub fn restore_sid(b: &mut impl Bus, sid: &Sid6581) {
    let base = 0xD400u16;
    for (i, &v) in sid.regs_25.iter().enumerate() {
        b.write8(base + (i as u16), v);
    }
}

pub fn restore_ram(b: &mut impl Bus, m: &C64Mem) {
    b.write8(0x0001, 0x07);
    b.write8(0x0001, 0x00);

    for (addr, &val) in m.ram.iter().enumerate() {
        let a = addr as u16;
        if (0xD800..=0xDBFF).contains(&a) {
            continue;
        }
        b.write8(a, val);
    }

    b.write8(0x0001, m.cpu_port_dir);
    b.write8(0x0001, m.cpu_port_data);
}

pub fn restore_cpu(ctrl: &mut impl CpuControl, c: &Cpu6510) {
    ctrl.set_cpu(c.a, c.x, c.y, c.sp, c.p, c.pc);
}

pub fn restore_all<B: Bus, C: CpuControl>(bus: &mut B, cpu: &mut C, snap: &C64Snapshot) {
    restore_cia(bus, CIA1_BASE, &snap.cia1);
    restore_cia(bus, CIA2_BASE, &snap.cia2);
    restore_vic(bus, &snap.vic);
    restore_sid(bus, &snap.sid);
    restore_ram(bus, &snap.mem);
    restore_cpu(cpu, &snap.cpu);
}

/* ======================= Helper functions ======================= */

fn ensure(cond: bool, msg: &str) -> Result<(), String> {
    if cond {
        Ok(())
    } else {
        Err(msg.to_string())
    }
}

fn ensure_eq(actual: &[u8], expected: &[u8], msg: &str) -> Result<(), String> {
    if actual == expected {
        Ok(())
    } else {
        Err(msg.to_string())
    }
}

fn read_fixed(cur: &mut Cursor<&[u8]>, n: usize) -> Result<Vec<u8>, String> {
    let mut buf = vec![0u8; n];
    cur.read_exact(&mut buf)
        .map_err(|_| "Unexpected EOF".to_string())?;
    Ok(buf)
}

fn read_fixed_opt(cur: &mut Cursor<&[u8]>, n: usize) -> Option<Vec<u8>> {
    let mut buf = vec![0u8; n];
    cur.read_exact(&mut buf).ok().map(|_| buf)
}

fn read_u8(cur: &mut Cursor<&[u8]>) -> Result<u8, String> {
    let mut b = [0u8; 1];
    cur.read_exact(&mut b)
        .map_err(|_| "Unexpected EOF".to_string())?;
    Ok(b[0])
}

fn read_u16(cur: &mut Cursor<&[u8]>) -> Result<u16, String> {
    let lo = read_u8(cur)? as u16;
    let hi = read_u8(cur)? as u16;
    Ok(lo | (hi << 8))
}

fn read_u32(cur: &mut Cursor<&[u8]>) -> Result<u32, String> {
    let w1 = read_u16(cur)? as u32;
    let w2 = read_u16(cur)? as u32;
    Ok(w1 | (w2 << 16))
}

fn trim_nul(bytes: &[u8]) -> &str {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    std::str::from_utf8(&bytes[..end]).unwrap_or("")
}

fn vsf_magic_ok(magic19: &[u8]) -> bool {
    if magic19.len() != 19 {
        return false;
    }
    let prefix = b"VICE Snapshot File";
    if !magic19.starts_with(prefix) {
        return false;
    }
    let sep = magic19[18];
    matches!(sep, b' ' | 0x00 | 0x1A)
}

fn sniff_compression_prefix(head: &[u8]) -> Option<&'static str> {
    if head.len() >= 2 && head[0] == 0x1F && head[1] == 0x8B {
        return Some("gzip");
    }
    if head.len() >= 3 && &head[0..3] == b"BZh" {
        return Some("bzip2");
    }
    if head.len() >= 4 && &head[0..4] == [0x50, 0x4B, 0x03, 0x04] {
        return Some("zip");
    }
    None
}
