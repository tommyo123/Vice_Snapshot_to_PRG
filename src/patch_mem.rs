//! Memory patcher for C64 snapshot restoration - Block 10 optimization
//!
//! Optimized two-block architecture:
//! - Block 9: Core restore + wipe blocks 1-8 + jump to block 10
//! - Block 10: Restore SP + wipe block 9 + restore $00 + build RTI frame + preload A/X/Y + jump to $01xx
//! - $01xx: Wipe block 10 + minimal restore + RTI
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

        // Preserve $F8-$FF before patching
        let mut f8_ff = [0u8; 8];
        f8_ff.copy_from_slice(&snap.mem.ram[0xF8..=0xFF]);

        // Generate block 9 with placeholder JMP to block 10
        let mut block9_code = Self::generate_block9_final(&blocks, &f8_ff)?;
        let exact_block9_size = block9_code.len() as u16;

        if exact_block9_size > 255 {
            return Err(PatchError::CodeTooLarge(
                format!("Block 9 is {} bytes (max 255)", exact_block9_size)
            ));
        }

        // Allocate block 9
        let (block9_addr, block9_fill) = match ram_finder.allocate(exact_block9_size) {
            Some((addr, value)) => (addr, value),
            None => {
                return Err(PatchError::AllocationFailed(
                    format!("Failed to allocate block 9 ({} bytes). Try with a cleaner snapshot", exact_block9_size)
                ));
            }
        };

        // Generate block 10 FIRST TIME with dummy fill value to get size
        let temp_block10_code = Self::generate_block10(snap, block9_addr, exact_block9_size, block9_fill, 0)?;
        let exact_block10_size = temp_block10_code.len() as u16;

        if exact_block10_size > 255 {
            return Err(PatchError::CodeTooLarge(
                format!("Block 10 is {} bytes (max 255)", exact_block10_size)
            ));
        }

        // Allocate block 10
        let (_block10_addr, block10_fill) = match ram_finder.allocate(exact_block10_size) {
            Some((addr, value)) => (addr, value),
            None => {
                return Err(PatchError::AllocationFailed(
                    format!("Failed to allocate block 10 ({} bytes). Try with a cleaner snapshot", exact_block10_size)
                ));
            }
        };

        // Generate block 10 SECOND TIME with correct fill value
        let mut block10_code = Self::generate_block10(snap, block9_addr, exact_block9_size, block9_fill, exact_block10_size)?;
        let block10_addr = _block10_addr;

        // Generate restore code
        let restore_code = Self::generate_restore_code(snap, block10_addr, exact_block10_size)?;
        let code_len = restore_code.len() as u16;

        // Calculate placement for restore code in $01xx
        const SAFETY_MARGIN: u16 = 6;
        let ideal_end = 0x0100 + (sp as u16).saturating_sub(SAFETY_MARGIN);
        let ideal_start = ideal_end.saturating_sub(code_len);

        let code_start = if ideal_start < 0x0100 {
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

        // CRITICAL: Patch JMP addresses
        // Block 9 → Block 10
        let jmp9_offset = block9_code.len() - 3;
        block9_code[jmp9_offset + 1] = (block10_addr & 0xFF) as u8;
        block9_code[jmp9_offset + 2] = (block10_addr >> 8) as u8;

        // Block 10 → $01xx restore code
        let jmp10_offset = block10_code.len() - 3;
        block10_code[jmp10_offset + 1] = (code_start & 0xFF) as u8;
        block10_code[jmp10_offset + 2] = (code_start >> 8) as u8;

        // Patch restore code into RAM
        let code_start_usize = code_start as usize;
        let code_end_usize = code_start_usize + restore_code.len();
        ram[code_start_usize..code_end_usize].copy_from_slice(&restore_code);

        // Copy $0100-$01FF to allocated blocks
        let mut temp = [0u8; 48];
        temp[0..32].copy_from_slice(&ram[0x0100..0x0120]);
        temp[32..48].copy_from_slice(&ram[0xFFF0..0x10000]);
        let addr = blocks[0].address as usize;
        ram[addr..addr + 48].copy_from_slice(&temp);

        let mut temp = [0u8; 40];
        temp[0..32].copy_from_slice(&ram[0x0120..0x0140]);
        temp[32..40].copy_from_slice(&ram[0x00F8..0x0100]);
        let addr = blocks[1].address as usize;
        ram[addr..addr + 40].copy_from_slice(&temp);

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

        // Write block 9 (with patched JMP to block 10)
        ram[block9_addr as usize..block9_addr as usize + block9_code.len()]
            .copy_from_slice(&block9_code);

        // Write block 10 (with patched JMP to $01xx)
        ram[block10_addr as usize..block10_addr as usize + block10_code.len()]
            .copy_from_slice(&block10_code);

        // Add blocks to list
        blocks.push(BlockAllocation {
            address: block9_addr,
            original_value: block9_fill,
            size: exact_block9_size
        });

        blocks.push(BlockAllocation {
            address: block10_addr,
            original_value: block10_fill,
            size: exact_block10_size
        });

        Ok(PatchMem {
            blocks,
            block9_addr,
        })
    }

    pub fn get_block9_addr(&self) -> u16 {
        self.block9_addr
    }

    /// Generate block 9 - clean restore, no register setup
    fn generate_block9_final(
        blocks: &[BlockAllocation],
        f8_ff: &[u8; 8],
    ) -> Result<Vec<u8>, PatchError> {
        let mut code = Self::generate_block9_core(blocks)?;

        // Restore $F8-$FF
        for i in 0..8 {
            code.extend_from_slice(&[0xA9, f8_ff[i]]);
            code.extend_from_slice(&[0x85, 0xF8 + i as u8]);
        }

        // Jump to block 10 (placeholder - will be patched)
        code.extend_from_slice(&[0x4C, 0x00, 0x00]); // JMP $0000

        Ok(code)
    }

    /// Generate block 9 core (unchanged)
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

        // Restore $FFF0-$FFFF
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

        // Clean blocks 1-8
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

    /// Generate block 10 - does heavy lifting!
    /// - Restores SP FIRST (critical!)
    /// - Wipes block 9
    /// - Restores $00 (CPU port DDR)
    /// - Builds RTI frame
    /// - Preloads A/X/Y for $01xx with correct values
    fn generate_block10(
        snap: &C64Snapshot,
        block9_addr: u16,
        block9_size: u16,
        block9_fill: u8,
        block10_size: u16,
    ) -> Result<Vec<u8>, PatchError> {
        let mut code = Vec::new();

        // CRITICAL: Restore stack pointer FIRST before anything else!
        code.extend_from_slice(&[0xA2, snap.cpu.sp]); // LDX #SP
        code.push(0x9A); // TXS

        // Wipe block 9
        if block9_size > 0 && block9_size <= 256 {
            code.extend_from_slice(&[0xA9, block9_fill]); // LDA #fill
            code.extend_from_slice(&[0xA2, 0x00]); // LDX #$00
            let wipe_loop = code.len();
            code.extend_from_slice(&[
                0x9D, (block9_addr & 0xFF) as u8, (block9_addr >> 8) as u8
            ]); // STA block9,X
            code.push(0xE8); // INX
            code.extend_from_slice(&[0xE0, block9_size as u8]); // CPX #size
            code.push(0xD0); // BNE
            let offset = ((wipe_loop as isize) - ((code.len() + 1) as isize)) as i8;
            code.push(offset as u8);
        }

        // Restore $00 (CPU port DDR) - SAFE! 99.99% sane values
        code.extend_from_slice(&[0xA9, snap.mem.cpu_port_dir]);
        code.extend_from_slice(&[0x85, 0x00]); // STA $00

        // Build RTI frame (stack is now valid!)
        code.extend_from_slice(&[0xA9, (snap.cpu.pc >> 8) as u8]);
        code.push(0x48); // PHA - PC high
        code.extend_from_slice(&[0xA9, (snap.cpu.pc & 0xFF) as u8]);
        code.push(0x48); // PHA - PC low
        code.extend_from_slice(&[0xA9, snap.cpu.p]);
        code.push(0x48); // PHA - P register

        // Preload A, X, Y for $01xx
        // A = 0x00 (fill value for wipe)
        code.extend_from_slice(&[0xA9, 0x00]); // LDA #$00

        // X = CPU port data (for STX $01 in $01xx)
        code.extend_from_slice(&[0xA2, snap.mem.cpu_port_data]); // LDX #cpu_port_data

        // Y = correct counter value based on wipe strategy
        if block10_size == 256 || block10_size > 128 {
            code.extend_from_slice(&[0xA0, 0xFF]); // LDY #$FF (for BPL)
        } else {
            let counter = block10_size.saturating_sub(1) as u8;
            code.extend_from_slice(&[0xA0, counter]); // LDY #size-1 (for BNE)
        }

        // Jump to $01xx restore code (placeholder - will be patched)
        code.extend_from_slice(&[0x4C, 0x00, 0x00]); // JMP $0000

        Ok(code)
    }

    /// Generate minimal restore code using preloaded A/X/Y from block 10
    /// At entry: A=0x00 (fill), X=cpu_port_data, Y=counter
    /// RTI frame already on stack: [P][PCL][PCH]
    fn generate_restore_code(
        snap: &C64Snapshot,
        block10_addr: u16,
        block10_size: u16,
    ) -> Result<Vec<u8>, PatchError> {
        let mut code = Vec::new();

        // At entry: A=0x00 (fill), X=cpu_port_data, Y=counter (already correct!)

        // Wipe block 10 - Y already has correct value from block 10!
        if block10_size == 256 || block10_size > 128 {
            // Use BPL (Y=$FF from block 10)
            let wipe_loop = code.len();
            code.extend_from_slice(&[
                0x99, (block10_addr & 0xFF) as u8, (block10_addr >> 8) as u8
            ]); // STA block10,Y
            code.push(0x88); // DEY
            code.push(0x10); // BPL
            let offset = ((wipe_loop as isize) - ((code.len() + 1) as isize)) as i8;
            code.push(offset as u8);
        } else {
            // Use BNE (Y=size-1 from block 10)
            let wipe_loop = code.len();
            code.extend_from_slice(&[
                0x99, (block10_addr & 0xFF) as u8, (block10_addr >> 8) as u8
            ]); // STA block10,Y
            code.push(0x88); // DEY
            code.push(0xD0); // BNE
            let offset = ((wipe_loop as isize) - ((code.len() + 1) as isize)) as i8;
            code.push(offset as u8);
        }

        // Restore $01 using X (preloaded from block 10!)
        code.extend_from_slice(&[0x86, 0x01]); // STX $01

        // VIC IRQ - Disable first
        code.extend_from_slice(&[0xA9, 0x00]);
        code.extend_from_slice(&[0x8D, 0x1A, 0xD0]);

        // Clear VIC IRQ
        code.extend_from_slice(&[0xA9, 0xFF]);
        code.extend_from_slice(&[0x8D, 0x19, 0xD0]);

        // Drain CIA interrupts (CRITICAL!)
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

        // Start CIA timers
        code.extend_from_slice(&[0xA9, snap.cia1.cra]);
        code.extend_from_slice(&[0x8D, 0x0E, 0xDC]);
        code.extend_from_slice(&[0xA9, snap.cia1.crb]);
        code.extend_from_slice(&[0x8D, 0x0F, 0xDC]);
        code.extend_from_slice(&[0xA9, snap.cia2.cra]);
        code.extend_from_slice(&[0x8D, 0x0E, 0xDD]);
        code.extend_from_slice(&[0xA9, snap.cia2.crb]);
        code.extend_from_slice(&[0x8D, 0x0F, 0xDD]);

        // Load final X, Y, and A registers (CRITICAL - must be last!)
        code.extend_from_slice(&[0xA2, snap.cpu.x]);
        code.extend_from_slice(&[0xA0, snap.cpu.y]);
        code.extend_from_slice(&[0xA9, snap.cpu.a]); // MUST reload A!

        // RTI
        code.push(0x40);

        Ok(code)
    }
}
