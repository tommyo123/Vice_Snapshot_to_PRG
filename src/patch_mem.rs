//! Memory patcher for C64 snapshot restoration - Conservative optimization
//!
//! Small, safe optimizations from working code:
//! - Move SP restore to block 9
//! - Use X/Y registers strategically (Action Replay style)
//! - Move 100% safe VIC setup to main code
//!
//! This program is unlicensed and dedicated to the public domain.
//! Developed by Tommy Olsen.

#![allow(dead_code)]

use crate::find_ram::FindRam;
use crate::parse_vsf::C64Snapshot;

#[derive(Debug)]
pub enum PatchError {
    AllocationFailed(String),
    StackTooLow(String),
    CodeTooLarge(String),
}

impl std::fmt::Display for PatchError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            PatchError::AllocationFailed(s) => write!(f, "Allocation failed: {}", s),
            PatchError::StackTooLow(s) => write!(f, "Stack too low: {}", s),
            PatchError::CodeTooLarge(s) => write!(f, "Code too large: {}", s),
        }
    }
}

impl std::error::Error for PatchError {}

struct BlockAllocation {
    address: u16,
    original_value: u8,
    size: u16,
}

pub struct PatchMem {
    blocks: Vec<BlockAllocation>,
    block9_addr: u16,
}

impl PatchMem {
    /// Patch RAM with restoration code and allocate blocks
    pub fn new(snap: &C64Snapshot, ram: &mut [u8; 65536], ram_finder: &mut FindRam) -> Result<Self, PatchError> {
        let sp = snap.cpu.sp;

        // Allocate blocks 1-8 for preserving stack area
        let mut blocks = Vec::new();
        let sizes = [48u16, 40, 32, 32, 32, 32, 32, 32];

        for (i, &size) in sizes.iter().enumerate() {
            match ram_finder.allocate(size) {
                Some((addr, value)) => {
                    blocks.push(BlockAllocation { address: addr, original_value: value, size });
                }
                None => {
                    return Err(PatchError::AllocationFailed(
                        format!("Failed to allocate block {} ({} bytes)", i + 1, size)
                    ));
                }
            }
        }

        // Generate block 9 core to calculate exact size
        let mut f8_ff = [0u8; 8];
        f8_ff.copy_from_slice(&snap.mem.ram[0xF8..=0xFF]);

        // Generate block 9 with placeholder JMP
        let mut block9_code = Self::generate_block9_final(&blocks, &f8_ff, snap)?;
        let exact_block9_size = block9_code.len() as u16;

        if exact_block9_size > 255 {
            return Err(PatchError::CodeTooLarge(
                format!("Block 9 is {} bytes (max 255)", exact_block9_size)
            ));
        }

        // Allocate block 9 with exact size
        let (block9_addr, block9_fill) = match ram_finder.allocate(exact_block9_size) {
            Some((addr, value)) => (addr, value),
            None => {
                return Err(PatchError::AllocationFailed(
                    format!("Failed to allocate block 9 ({} bytes). Try with a cleaner snapshot (run 'f 0000 ffff 00' in VICE monitor before taking snapshot)", exact_block9_size)
                ));
            }
        };

        // Generate restore code (now smaller!)
        let restore_code = Self::generate_restore_code(snap, block9_addr, exact_block9_size, block9_fill)?;
        let code_len = restore_code.len() as u16;

        // Calculate placement for restore code
        const SAFETY_MARGIN: u16 = 6;
        let ideal_end = 0x0100 + (sp as u16).saturating_sub(SAFETY_MARGIN);
        let ideal_start = ideal_end.saturating_sub(code_len);

        let code_start = if ideal_start < 0x0100 {
            // Not enough room with margin - place at end of $01xx
            let end = 0x0200;
            let start = end - code_len;

            if start < 0x0100 {
                return Err(PatchError::CodeTooLarge(
                    format!("Restore code {} bytes too large for $0100-$01FF", code_len)
                ));
            }

            start
        } else {
            ideal_start
        };

        // CRITICAL: Patch the JMP address in block 9!
        let jmp_offset = block9_code.len() - 3; // Last 3 bytes are JMP $0000
        block9_code[jmp_offset + 1] = (code_start & 0xFF) as u8;
        block9_code[jmp_offset + 2] = (code_start >> 8) as u8;

        // Patch restore code into RAM
        let code_start_usize = code_start as usize;
        let code_end_usize = code_start_usize + restore_code.len();
        ram[code_start_usize..code_end_usize].copy_from_slice(&restore_code);

        // Copy $0100-$01FF to allocated blocks
        // Block 1: $0100-$011F + $FFF0-$FFFF (48 bytes)
        let mut temp = [0u8; 48];
        temp[0..32].copy_from_slice(&ram[0x0100..0x0120]);
        temp[32..48].copy_from_slice(&ram[0xFFF0..0x10000]);
        let addr = blocks[0].address as usize;
        ram[addr..addr + 48].copy_from_slice(&temp);

        // Block 2: $0120-$013F + $F8-$FF (40 bytes)
        let mut temp = [0u8; 40];
        temp[0..32].copy_from_slice(&ram[0x0120..0x0140]);
        temp[32..40].copy_from_slice(&ram[0x00F8..0x0100]);
        let addr = blocks[1].address as usize;
        ram[addr..addr + 40].copy_from_slice(&temp);

        // Blocks 3-8: 32 bytes each
        let ranges = [
            (0x0140, 0x0160, 2),
            (0x0160, 0x0180, 3),
            (0x0180, 0x01A0, 4),
            (0x01A0, 0x01C0, 5),
            (0x01C0, 0x01E0, 6),
            (0x01E0, 0x0200, 7),
        ];

        for &(start, end, idx) in &ranges {
            let mut temp = [0u8; 32];
            temp.copy_from_slice(&ram[start..end]);
            let addr = blocks[idx].address as usize;
            ram[addr..addr + 32].copy_from_slice(&temp);
        }

        // Write block 9 complete code (with patched JMP)
        ram[block9_addr as usize..block9_addr as usize + block9_code.len()]
            .copy_from_slice(&block9_code);

        // Add block 9 to blocks list
        blocks.push(BlockAllocation {
            address: block9_addr,
            original_value: block9_fill,
            size: exact_block9_size
        });

        Ok(PatchMem {
            blocks,
            block9_addr,
        })
    }

    pub fn get_block9_addr(&self) -> u16 {
        self.block9_addr
    }

    /// Generate block 9 final code with conservative Action Replay optimization
    fn generate_block9_final(
        blocks: &[BlockAllocation],
        f8_ff: &[u8; 8],
        snap: &C64Snapshot,
    ) -> Result<Vec<u8>, PatchError> {
        let mut code = Self::generate_block9_core(blocks)?;

        // Restore $F8-$FF
        for i in 0..8 {
            code.extend_from_slice(&[0xA9, f8_ff[i]]);
            code.extend_from_slice(&[0x85, 0xF8 + i as u8]);
        }

        // OPTIMIZATION 1: Restore stack pointer here (Action Replay style!)
        code.extend_from_slice(&[0xA2, snap.cpu.sp]); // LDX #SP
        code.push(0x9A); // TXS

        // OPTIMIZATION 2: Load X with CPU port DDR (saves 2 bytes in $01xx)
        code.extend_from_slice(&[0xA2, snap.mem.cpu_port_dir]); // LDX #DDR

        // OPTIMIZATION 3: Load Y with $FF for VIC clear (saves 3 bytes in $01xx)
        code.extend_from_slice(&[0xA0, 0xFF]); // LDY #$FF

        // OPTIMIZATION 4: Load A with snapshot A value (saves 2 bytes in $01xx)
        code.extend_from_slice(&[0xA9, snap.cpu.a]); // LDA #A

        // Jump to restore code (placeholder - will be patched in new())
        code.extend_from_slice(&[0x4C, 0x00, 0x00]); // JMP $0000

        Ok(code)
    }

    /// Generate block 9 core (unchanged from original)
    fn generate_block9_core(blocks: &[BlockAllocation]) -> Result<Vec<u8>, PatchError> {
        let mut code = Vec::new();

        // Copy blocks 1-8 back to $0100-$01FF
        for i in 0..8 {
            let dst = 0x0100u16 + ((i as u16) * 32);
            code.extend_from_slice(&[0xA2, 31]);
            let loop_start = code.len();
            code.extend_from_slice(&[
                0xBD, blocks[i].address as u8, (blocks[i].address >> 8) as u8
            ]);
            code.extend_from_slice(&[
                0x9D, (dst & 0xFF) as u8, (dst >> 8) as u8
            ]);
            code.push(0xCA);
            let offset = ((loop_start as isize) - (code.len() as isize + 2)) as u8;
            code.extend_from_slice(&[0x10, offset]);
        }

        // Restore $FFF0-$FFFF from block 1 offset +32
        code.extend_from_slice(&[0xA2, 0x0F]);
        let loop2 = code.len();
        let addr = blocks[0].address + 32;
        code.extend_from_slice(&[
            0xBD, addr as u8, (addr >> 8) as u8
        ]);
        code.extend_from_slice(&[0x9D, 0xF0, 0xFF]);
        code.push(0xCA);
        let offset = ((loop2 as isize) - (code.len() as isize + 2)) as u8;
        code.extend_from_slice(&[0x10, offset]);

        // Clean blocks 1-8 (CRITICAL!)
        for i in 0..8 {
            let addr = blocks[i].address;
            let size = blocks[i].size;
            let value = blocks[i].original_value;

            if size > 256 {
                return Err(PatchError::CodeTooLarge(
                    format!("Block {} size {} exceeds 256 bytes", i+1, size)
                ));
            }

            code.extend_from_slice(&[0xA9, value]);
            code.extend_from_slice(&[0xA2, 0x00]);
            let fill = code.len();
            code.extend_from_slice(&[
                0x9D, addr as u8, (addr >> 8) as u8
            ]);
            code.push(0xE8);
            code.extend_from_slice(&[0xE0, size as u8]);
            let offset = ((fill as isize) - (code.len() as isize + 2)) as u8;
            code.extend_from_slice(&[0xD0, offset]);
        }

        Ok(code)
    }

    /// Generate restore code with conservative optimizations
    fn generate_restore_code(
        snap: &C64Snapshot,
        block9_addr: u16,
        exact_block9_size: u16,
        block9_fill: u8
    ) -> Result<Vec<u8>, PatchError> {
        let mut code = Vec::new();

        // At this point from block 9:
        // - Stack pointer is already restored
        // - X = CPU port DDR
        // - Y = $FF
        // - A = snapshot A value

        // Wipe block 9 (MUST happen here, after block 9 has run!)
        if exact_block9_size > 0 && exact_block9_size <= 256 {
            // Save A (we'll need it later)
            code.push(0x48); // PHA

            code.extend_from_slice(&[0xA9, block9_fill]); // LDA #fill_value
            code.extend_from_slice(&[0xA2, 0x00]); // LDX #$00
            let wipe_loop = code.len();
            code.extend_from_slice(&[
                0x9D, (block9_addr & 0xFF) as u8, (block9_addr >> 8) as u8
            ]); // STA block9_addr,X
            code.push(0xE8); // INX
            code.extend_from_slice(&[0xE0, exact_block9_size as u8]); // CPX #size
            let offset = ((wipe_loop as isize) - (code.len() as isize + 2)) as u8;
            code.extend_from_slice(&[0xD0, offset]); // BNE loop

            // Restore registers from block 9
            code.extend_from_slice(&[0xA2, snap.mem.cpu_port_dir]); // LDX #DDR (restore X)
            code.extend_from_slice(&[0xA0, 0xFF]); // LDY #$FF (restore Y)
            code.push(0x68); // PLA (restore A)
        }

        // OPTIMIZATION: Use X from block 9 (saves 2 bytes)
        code.extend_from_slice(&[0x86, 0x00]); // STX $00 (instead of LDA #DDR + STA)

        // Enable I/O
        code.extend_from_slice(&[0xA9, 0x35]);
        code.extend_from_slice(&[0x85, 0x01]);

        // VIC IRQ - Disable first (will be moved to main in make_prg_asm.rs)
        code.extend_from_slice(&[0xA9, 0x00]);
        code.extend_from_slice(&[0x8D, 0x1A, 0xD0]);

        // OPTIMIZATION: Use Y from block 9 (saves 3 bytes)
        code.extend_from_slice(&[0x8C, 0x19, 0xD0]); // STY $D019 (instead of LDA #$FF + STA)

        // Drain CIA interrupts (CRITICAL - after ~5 sec decompression!)
        code.extend_from_slice(&[0xAD, 0x0D, 0xDC]);
        code.extend_from_slice(&[0xAD, 0x0D, 0xDD]);

        // Clear VIC IRQ again
        code.extend_from_slice(&[0xA9, 0xFF]);
        code.extend_from_slice(&[0x8D, 0x19, 0xD0]);

        // Enable VIC IRQ
        code.extend_from_slice(&[0xA9, snap.vic.registers[0x1A]]);
        code.extend_from_slice(&[0x8D, 0x1A, 0xD0]);

        // Drain CIA again
        code.extend_from_slice(&[0xAD, 0x0D, 0xDC]);
        code.extend_from_slice(&[0xAD, 0x0D, 0xDD]);

        // Enable CIA interrupts if needed
        if snap.cia1.ier != 0 {
            code.extend_from_slice(&[0xA9, snap.cia1.ier | 0x80]);
            code.extend_from_slice(&[0x8D, 0x0D, 0xDC]);
        }
        if snap.cia2.ier != 0 {
            code.extend_from_slice(&[0xA9, snap.cia2.ier | 0x80]);
            code.extend_from_slice(&[0x8D, 0x0D, 0xDD]);
        }

        // Start CIA timers (CRITICAL!)
        code.extend_from_slice(&[0xA9, snap.cia1.cra]);
        code.extend_from_slice(&[0x8D, 0x0E, 0xDC]);
        code.extend_from_slice(&[0xA9, snap.cia1.crb]);
        code.extend_from_slice(&[0x8D, 0x0F, 0xDC]);
        code.extend_from_slice(&[0xA9, snap.cia2.cra]);
        code.extend_from_slice(&[0x8D, 0x0E, 0xDD]);
        code.extend_from_slice(&[0xA9, snap.cia2.crb]);
        code.extend_from_slice(&[0x8D, 0x0F, 0xDD]);

        // Restore original CPU port data
        code.extend_from_slice(&[0xA9, snap.mem.cpu_port_data]);
        code.extend_from_slice(&[0x85, 0x01]);

        // Build RTI frame: PCH, PCL, P
        code.extend_from_slice(&[0xA9, (snap.cpu.pc >> 8) as u8]);
        code.push(0x48);
        code.extend_from_slice(&[0xA9, (snap.cpu.pc & 0xFF) as u8]);
        code.push(0x48);
        code.extend_from_slice(&[0xA9, snap.cpu.p]);
        code.push(0x48);

        // Load final X and Y registers
        code.extend_from_slice(&[0xA2, snap.cpu.x]);
        code.extend_from_slice(&[0xA0, snap.cpu.y]);

        // OPTIMIZATION: A already has correct value from block 9! (saves 2 bytes)
        // Don't need: LDA #snap.cpu.a

        // RTI
        code.push(0x40);

        Ok(code)
    }
}
