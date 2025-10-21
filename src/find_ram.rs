//! RAM free block finder and allocator
//!
//! Scans C64 RAM for contiguous sequences of identical byte values (RLE-style)
//! and provides allocation tracking for those sequences.
//!
//! Only tracks sequences of 32 or more consecutive identical bytes in the
//! $0200-$FFEF range (avoiding zero page, stack, and system vectors).
//!
// Copyright (c) 2025 Tommy Olsen
// Licensed under the MIT License.

#![allow(dead_code)]

#[derive(Debug, Clone)]
pub struct RamBlock {
    pub address: u16,
    pub value: u8,
    pub count: u16,
}

pub struct FindRam {
    blocks: Vec<RamBlock>,
}

impl FindRam {
    /// Scan RAM from $0200-$FFEF for sequences of 32+ identical consecutive bytes
    pub fn new(ram: &[u8; 65536]) -> Self {
        let mut blocks = Vec::new();

        const START_ADDR: usize = 0x0200;
        const END_ADDR: usize = 0xFFEF;
        const MIN_SEQUENCE_LEN: usize = 32;

        let mut addr = START_ADDR;

        while addr <= END_ADDR {
            let current_value = ram[addr];
            let mut count = 1;

            while addr + count <= END_ADDR && ram[addr + count] == current_value {
                count += 1;
            }

            if count >= MIN_SEQUENCE_LEN {
                blocks.push(RamBlock {
                    address: addr as u16,
                    value: current_value,
                    count: count as u16,
                });
                addr += count;
            } else {
                addr += 1;
            }
        }

        FindRam { blocks }
    }

    /// Find the maximum contiguous sequence length available (0 if none)
    pub fn find_max(&self) -> u16 {
        self.blocks
            .iter()
            .map(|block| block.count)
            .max()
            .unwrap_or(0)
    }

    /// Allocate a block of the specified size using best-fit algorithm
    ///
    /// Searches for the smallest available block that fits the requested size.
    /// The block is either removed (exact match) or split (larger than needed).
    ///
    /// Returns Some((address, value)) on success, None if no suitable block exists
    pub fn allocate(&mut self, requested_count: u16) -> Option<(u16, u8)> {
        if requested_count == 0 {
            return None;
        }

        let best_match = self.blocks
            .iter()
            .enumerate()
            .filter(|(_, block)| block.count >= requested_count)
            .min_by_key(|(_, block)| block.count);

        if let Some((index, _)) = best_match {
            let block = &self.blocks[index];
            let allocated_address = block.address;
            let allocated_value = block.value;
            let remaining_count = block.count - requested_count;

            if remaining_count == 0 {
                self.blocks.remove(index);
            } else {
                let new_address = block.address + requested_count;
                self.blocks[index] = RamBlock {
                    address: new_address,
                    value: allocated_value,
                    count: remaining_count,
                };
            }

            Some((allocated_address, allocated_value))
        } else {
            None
        }
    }

    pub fn block_count(&self) -> usize {
        self.blocks.len()
    }

    pub fn total_free_bytes(&self) -> u32 {
        self.blocks.iter().map(|b| b.count as u32).sum()
    }

    pub fn blocks(&self) -> &[RamBlock] {
        &self.blocks
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_sequences() {
        let mut ram = [0u8; 65536];

        // Create a sequence of 64 zeros at $2500
        for i in 0x2500..0x2540 {
            ram[i] = 0x00;
        }

        // Create a sequence of 32 $21 values at $3000
        for i in 0x3000..0x3020 {
            ram[i] = 0x21;
        }

        // Create a sequence of only 16 values (should be ignored)
        for i in 0x4000..0x4010 {
            ram[i] = 0xFF;
        }

        let finder = FindRam::new(&ram);

        // Should find 2 blocks (ignoring the 16-byte sequence)
        assert_eq!(finder.block_count(), 2);

        // Maximum should be 64
        assert_eq!(finder.find_max(), 64);
    }

    #[test]
    fn test_allocate_exact_match() {
        let mut ram = [0u8; 65536];

        // 32 zeros at $2500
        for i in 0x2500..0x2520 {
            ram[i] = 0x00;
        }

        let mut finder = FindRam::new(&ram);

        // Allocate exactly 32 bytes
        let result = finder.allocate(32);
        assert_eq!(result, Some((0x2500, 0x00)));

        // Block should be removed
        assert_eq!(finder.block_count(), 0);
    }

    #[test]
    fn test_allocate_partial() {
        let mut ram = [0u8; 65536];

        // 64 zeros at $5000
        for i in 0x5000..0x5040 {
            ram[i] = 0x00;
        }

        let mut finder = FindRam::new(&ram);

        // Allocate 32 bytes from 64-byte block
        let result = finder.allocate(32);
        assert_eq!(result, Some((0x5000, 0x00)));

        // Should have 1 block remaining with 32 bytes at $5020
        assert_eq!(finder.block_count(), 1);
        assert_eq!(finder.blocks()[0].address, 0x5020);
        assert_eq!(finder.blocks()[0].count, 32);
        assert_eq!(finder.blocks()[0].value, 0x00);
    }

    #[test]
    fn test_allocate_best_fit() {
        let mut ram = [0u8; 65536];

        // 100 zeros at $2000
        for i in 0x2000..0x2064 {
            ram[i] = 0x00;
        }

        // 50 zeros at $3000
        for i in 0x3000..0x3032 {
            ram[i] = 0x00;
        }

        let mut finder = FindRam::new(&ram);

        // Request 40 bytes - should pick the 50-byte block (closest fit)
        let result = finder.allocate(40);
        assert_eq!(result, Some((0x3000, 0x00)));

        // Should have 2 blocks: original 100-byte and remaining 10-byte
        assert_eq!(finder.block_count(), 2);
    }

    #[test]
    fn test_allocate_not_found() {
        let mut ram = [0u8; 65536];

        // Only 32 zeros available
        for i in 0x2500..0x2520 {
            ram[i] = 0x00;
        }

        let mut finder = FindRam::new(&ram);

        // Request more than available
        let result = finder.allocate(64);
        assert_eq!(result, None);
    }

    #[test]
    fn test_ignores_area_below_0x200() {
        let mut ram = [0u8; 65536];

        // Fill entire zero page and stack with zeros (should be ignored)
        for i in 0x0000..0x0200 {
            ram[i] = 0x00;
        }

        let finder = FindRam::new(&ram);

        // Should find nothing below $0200
        assert_eq!(finder.block_count(), 0);
    }
}
