//! RAM free block finder and allocator
//!
//! Scans C64 RAM for contiguous sequences of identical byte values (RLE-style)
//! and provides allocation tracking for those sequences.
//!
//! Only tracks sequences of 32 or more consecutive identical bytes in the
//! $0200-$FFEF range (avoiding zero page, stack, and system vectors).
//!
// Copyright (c) 2025-2026 Tommy Olsen
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
        Self::with_extra_blocks(ram, &[])
    }

    /// Scan RAM and add extra manually specified blocks
    pub fn with_extra_blocks(ram: &[u8; 65536], extra_blocks: &[(u16, u16)]) -> Self {
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

        // Add extra manually specified blocks (address, count) with value 0
        for &(address, count) in extra_blocks {
            if count >= 32 {
                blocks.push(RamBlock {
                    address,
                    value: 0,
                    count,
                });
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

    /// Expected byte of the C64 power-on RAM pattern at `addr`.
    ///
    /// A freshly powered C64 (and VICE's default/Smart-Attach RAM init) does not
    /// come up all-zero; it comes up in a fixed pattern of $00 and $FF bytes.
    /// Empirically (VICE 3.10) the pattern is: 4-byte runs of $00 or $FF that
    /// alternate (phase offset by 2 bytes) and invert every 8 KB, e.g.
    ///
    /// ```text
    /// $2000: 00 00 FF FF FF FF 00 00  00 00 FF FF FF FF 00 00 ...
    /// $4000: FF FF 00 00 00 00 FF FF  ...   (inverted in the next 8 KB block)
    /// ```
    ///
    /// which is `start $FF  XOR  (run-of-4, +2 phase)  XOR  (invert every 8 KB)`.
    /// Because the runs are only 4 bytes long they fall below the 32-byte
    /// threshold of the free-block scan, so such memory looks "used" even though
    /// the program never touched it. See [`clear_poweron_pattern`].
    pub fn poweron_pattern_byte(addr: u16) -> u8 {
        let a = addr as u32;
        let mut v: u8 = 0xFF; // start value
        if (((a + 2) / 4) & 1) != 0 {
            v ^= 0xFF; // 4-byte value run, phase-shifted by 2
        }
        if ((a / 8192) & 1) != 0 {
            v ^= 0xFF; // whole 8 KB block inverted
        }
        v
    }

    /// Zero every region of RAM that still holds the C64 power-on pattern.
    ///
    /// This automates, for the common case, the manual "clear RAM" step the tool
    /// otherwise asks for (`f 0000 ffff 00` in the VICE monitor): regions left in
    /// their power-on state are RAM the program never used, so zeroing them is
    /// safe and turns them into large uniform blocks the allocator can use (and
    /// which compress to almost nothing).
    ///
    /// Detection is a strict, byte-exact match against [`poweron_pattern_byte`]
    /// over the same $0200-$FFEF range the free scan uses. Only maximal matching
    /// spans of at least [`MIN_PATTERN_SPAN`] bytes are cleared: a mismatching
    /// byte is never touched, it only splits the span. Program data is affected
    /// only if it reproduces the exact global phase for 64+ contiguous bytes
    /// (possible for e.g. an aligned charset of long $00/$FF runs), which is
    /// why callers expose the pass as an option (`clear_poweron_ram`,
    /// `--clear-poweron-ram`) instead of running it unconditionally.
    ///
    /// Returns the number of bytes cleared.
    pub fn clear_poweron_pattern(ram: &mut [u8; 65536]) -> u32 {
        const START: usize = 0x0200;
        const END: usize = 0xFFEF; // inclusive, matches the free-block scan range
        const MIN_PATTERN_SPAN: usize = 64;

        let mut cleared = 0u32;
        let mut addr = START;
        while addr <= END {
            if ram[addr] == Self::poweron_pattern_byte(addr as u16) {
                let span_start = addr;
                while addr <= END && ram[addr] == Self::poweron_pattern_byte(addr as u16) {
                    addr += 1;
                }
                if addr - span_start >= MIN_PATTERN_SPAN {
                    for b in &mut ram[span_start..addr] {
                        *b = 0;
                    }
                    cleared += (addr - span_start) as u32;
                }
            } else {
                addr += 1;
            }
        }
        cleared
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

    /// A 64 KB background with no run of 32+ identical bytes, so the scan only
    /// reports the sequences a test explicitly plants (real RAM is never a single
    /// uniform value the way `[0u8; 65536]` is).
    fn varied_ram() -> [u8; 65536] {
        let mut ram = [0u8; 65536];
        for (i, b) in ram.iter_mut().enumerate() {
            *b = i as u8;
        }
        ram
    }

    #[test]
    fn test_find_sequences() {
        let mut ram = varied_ram();

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
        let mut ram = varied_ram();

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
        let mut ram = varied_ram();

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
        let mut ram = varied_ram();

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
        let mut ram = varied_ram();

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
        let mut ram = varied_ram();

        // Fill entire zero page and stack with zeros (should be ignored)
        for i in 0x0000..0x0200 {
            ram[i] = 0x00;
        }

        let finder = FindRam::new(&ram);

        // Should find nothing below $0200
        assert_eq!(finder.block_count(), 0);
    }

    #[test]
    fn poweron_pattern_matches_observed_vice_bytes() {
        // Bytes observed in a fresh VICE 3.10 C64 boot.
        assert_eq!(FindRam::poweron_pattern_byte(0x2000), 0x00);
        assert_eq!(FindRam::poweron_pattern_byte(0x2001), 0x00);
        assert_eq!(FindRam::poweron_pattern_byte(0x2002), 0xFF);
        assert_eq!(FindRam::poweron_pattern_byte(0x2005), 0xFF);
        assert_eq!(FindRam::poweron_pattern_byte(0x2006), 0x00);
        assert_eq!(FindRam::poweron_pattern_byte(0x4000), 0xFF);
        assert_eq!(FindRam::poweron_pattern_byte(0x4002), 0x00);
        assert_eq!(FindRam::poweron_pattern_byte(0xC000), 0xFF);
    }

    #[test]
    fn clears_poweron_pattern_but_not_program_data() {
        let mut ram = [0u8; 65536];
        // A real pattern region (with a couple of sparse anomalies, as VICE does).
        for a in 0x2000..0x3000 {
            ram[a] = FindRam::poweron_pattern_byte(a as u16);
        }
        ram[0x2480] = 0x08; // isolated random byte
        ram[0x2503] = 0x42;
        // Program data that must NOT be mistaken for the pattern:
        for a in 0x4000..0x4100 {
            ram[a] = 0xAB; // arbitrary
        }
        for a in 0x5000..0x5100 {
            ram[a] = 0xFF; // solid $FF (free, but not the alternating pattern)
        }

        let cleared = FindRam::clear_poweron_pattern(&mut ram);

        // Most of the 4 KB pattern region is zeroed.
        assert!(cleared > 0x0E00, "cleared only {cleared} bytes");
        assert_eq!(ram[0x2800], 0);
        assert_eq!(ram[0x2002], 0);
        // Program data is untouched.
        assert_eq!(ram[0x4080], 0xAB);
        assert_eq!(ram[0x5080], 0xFF);
        // After clearing, the region is a large free block the scan can use.
        let finder = FindRam::new(&ram);
        assert!(finder.find_max() >= 0x0800);
    }

    #[test]
    fn poweron_clear_never_zeroes_interior_program_bytes() {
        // Program-written bytes inside a pattern region must never be zeroed;
        // a mismatch ends the span instead of being absorbed into it.
        let mut ram = [0u8; 65536];
        for a in 0x2000..0x3000 {
            ram[a] = FindRam::poweron_pattern_byte(a as u16);
        }
        // A 3-byte counter/flag/pointer a game left in the middle of the region.
        ram[0x2800] = 0x12;
        ram[0x2801] = 0x34;
        ram[0x2802] = 0x56;

        let cleared = FindRam::clear_poweron_pattern(&mut ram);

        assert!(cleared > 0, "pattern region should still be cleared");
        assert_eq!(ram[0x2800], 0x12, "program byte was zeroed");
        assert_eq!(ram[0x2801], 0x34, "program byte was zeroed");
        assert_eq!(ram[0x2802], 0x56, "program byte was zeroed");
        // Both sides of the program bytes are cleared.
        assert_eq!(ram[0x27FF], 0);
        assert_eq!(ram[0x2803], 0);
    }

    #[test]
    fn poweron_clear_leaves_uninitialized_short_runs_alone() {
        // A region that is NOT the pattern (all $00 program data) yields no
        // pattern match longer than the 4-byte pattern phase, so nothing is
        // cleared by the pattern pass (the normal scan handles all-$00 anyway).
        let mut ram = [0u8; 65536];
        for a in 0x6000..0x6100 {
            ram[a] = 0x55; // not $00/$FF at all
        }
        let cleared = FindRam::clear_poweron_pattern(&mut ram);
        assert_eq!(cleared, 0);
        assert_eq!(ram[0x6080], 0x55);
    }
}
