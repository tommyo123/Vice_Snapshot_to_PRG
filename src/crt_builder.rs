//! CRT cartridge file builder
//!
//! Creates C64 cartridge files (.crt) with multiple banks for EasyFlash format.
//!
// Copyright (c) 2025 Tommy Olsen
// Licensed under the MIT License.

use std::fs::File;
use std::io::Write;

/// Supported cartridge types
#[derive(Debug, Clone, Copy)]
pub enum CartridgeType {
    /// EasyFlash cartridge (hardware type 32)
    /// Ultimax mode: ROML @ $8000-$9FFF, ROMH @ $E000-$FFFF
    EasyFlash,
}

impl CartridgeType {
    pub fn hardware_type(&self) -> u16 {
        match self {
            CartridgeType::EasyFlash => 32,
        }
    }

    pub fn exrom(&self) -> u8 {
        match self {
            CartridgeType::EasyFlash => 1,
        }
    }

    pub fn game(&self) -> u8 {
        match self {
            CartridgeType::EasyFlash => 0,
        }
    }
}

pub const BANK_SIZE_8K: usize = 8192;
pub const LOAD_ADDRESS_ROML: u16 = 0x8000;
pub const LOAD_ADDRESS_ROMH: u16 = 0xE000;

/// Builder for C64 cartridge files (.crt)
pub struct CRTBuilder {
    cartridge_type: CartridgeType,
    name: String,
    banks: Vec<Box<[u8; BANK_SIZE_8K]>>,
    banks_romh: Vec<Option<Box<[u8; BANK_SIZE_8K]>>>,
}

impl CRTBuilder {
    /// Create a new CRT builder
    ///
    /// # Arguments
    /// * `cartridge_type` - Type of cartridge (EasyFlash)
    /// * `initial_banks` - Number of banks to create initially
    /// * `name` - Cartridge name (max 32 characters, will be converted to uppercase)
    pub fn new(cartridge_type: CartridgeType, initial_banks: usize, name: &str) -> Result<Self, String> {
        if initial_banks == 0 {
            return Err("Must have at least one bank".to_string());
        }
        if name.len() > 32 {
            return Err("Name cannot be longer than 32 characters".to_string());
        }

        let mut builder = Self {
            cartridge_type,
            name: name.to_uppercase(),
            banks: Vec::new(),
            banks_romh: Vec::new(),
        };

        for _ in 0..initial_banks {
            builder.add_bank();
        }

        Ok(builder)
    }

    /// Add a new bank and return the bank number
    pub fn add_bank(&mut self) -> usize {
        self.banks.push(Box::new([0u8; BANK_SIZE_8K]));
        self.banks_romh.push(None);
        self.banks.len() - 1
    }

    /// Get the number of banks
    pub fn bank_count(&self) -> usize {
        self.banks.len()
    }

    /// Get a mutable reference to a bank's data
    pub fn get_bank_mut(&mut self, bank_number: usize) -> Result<&mut [u8; BANK_SIZE_8K], String> {
        let max_bank = self.banks.len().saturating_sub(1);
        self.banks
            .get_mut(bank_number)
            .map(|b| &mut **b)
            .ok_or_else(|| format!("Bank {} does not exist. Valid banks: 0-{}", bank_number, max_bank))
    }

    /// Get an immutable reference to a bank's data
    pub fn get_bank(&self, bank_number: usize) -> Result<&[u8; BANK_SIZE_8K], String> {
        self.banks
            .get(bank_number)
            .map(|b| &**b)
            .ok_or_else(|| format!("Bank {} does not exist. Valid banks: 0-{}", bank_number, self.banks.len().saturating_sub(1)))
    }

    /// Set ROMH data for a bank
    /// ROMH appears at $E000-$FFFF in Ultimax mode
    pub fn set_bank_romh(&mut self, bank_number: usize, data: &[u8]) -> Result<(), String> {
        if bank_number >= self.banks.len() {
            return Err(format!("Bank {} does not exist. Valid banks: 0-{}", bank_number, self.banks.len().saturating_sub(1)));
        }
        if data.len() != BANK_SIZE_8K {
            return Err(format!("ROMH data must be exactly 8KB (got {} bytes)", data.len()));
        }

        let mut romh_data = Box::new([0u8; BANK_SIZE_8K]);
        romh_data.copy_from_slice(data);
        self.banks_romh[bank_number] = Some(romh_data);
        Ok(())
    }

    /// Get ROMH data for a bank (if set)
    pub fn get_bank_romh(&self, bank_number: usize) -> Option<&[u8; BANK_SIZE_8K]> {
        self.banks_romh.get(bank_number)?.as_ref().map(|b| &**b)
    }

    /// Fill a bank with data starting at the given offset
    pub fn fill_bank(&mut self, bank_number: usize, data: &[u8], offset: usize) -> Result<(), String> {
        let bank = self.get_bank_mut(bank_number)?;
        if offset + data.len() > BANK_SIZE_8K {
            return Err(format!(
                "Data does not fit in bank ({} bytes + offset {} > {})",
                data.len(),
                offset,
                BANK_SIZE_8K
            ));
        }
        bank[offset..offset + data.len()].copy_from_slice(data);
        Ok(())
    }

    /// Clear a bank with a specific byte value
    pub fn clear_bank(&mut self, bank_number: usize, value: u8) -> Result<(), String> {
        let bank = self.get_bank_mut(bank_number)?;
        bank.fill(value);
        Ok(())
    }

    /// Generate the complete CRT file data
    pub fn generate_crt_data(&self) -> Vec<u8> {
        let mut output = Vec::new();

        // Write file header
        output.extend_from_slice(&self.create_file_header());

        // Write CHIP packets for each bank
        for (index, bank) in self.banks.iter().enumerate() {
            // ROML @ $8000-$9FFF (8 KB)
            output.extend_from_slice(&self.create_chip_packet(index, LOAD_ADDRESS_ROML, &**bank));

            // ROMH @ $E000-$FFFF (8 KB) - if present
            if let Some(romh_data) = &self.banks_romh[index] {
                output.extend_from_slice(&self.create_chip_packet(index, LOAD_ADDRESS_ROMH, &**romh_data));
            }
        }

        output
    }

    /// Write the CRT file to disk
    pub fn make_crt(&self, output_file: &str) -> Result<(), String> {
        let crt_data = self.generate_crt_data();
        let mut file = File::create(output_file)
            .map_err(|e| format!("Failed to create CRT file: {}", e))?;
        file.write_all(&crt_data)
            .map_err(|e| format!("Failed to write CRT data: {}", e))?;
        Ok(())
    }

    /// Create CRT file header (64 bytes)
    fn create_file_header(&self) -> [u8; 64] {
        let mut header = [0u8; 64];

        // Signature: "C64 CARTRIDGE   " (16 bytes)
        header[0..16].copy_from_slice(b"C64 CARTRIDGE   ");

        // Header length: 0x00000040 (64 bytes) - big endian
        header[16..20].copy_from_slice(&0x00000040u32.to_be_bytes());

        // Version: 0x0100 - big endian
        header[20..22].copy_from_slice(&0x0100u16.to_be_bytes());

        // Hardware type - big endian
        header[22..24].copy_from_slice(&self.cartridge_type.hardware_type().to_be_bytes());

        // EXROM line
        header[24] = self.cartridge_type.exrom();

        // GAME line
        header[25] = self.cartridge_type.game();

        // Reserved (6 bytes) - already zeros

        // Cartridge name (32 bytes, null-terminated)
        let name_bytes = self.name.as_bytes();
        let copy_len = name_bytes.len().min(31);
        header[32..32 + copy_len].copy_from_slice(&name_bytes[..copy_len]);
        // Rest already filled with zeros

        header
    }

    /// Create a CHIP packet with explicit start address
    fn create_chip_packet(&self, bank_number: usize, start_address: u16, data: &[u8]) -> Vec<u8> {
        let packet_size = 16 + data.len();
        let mut packet = vec![0u8; packet_size];

        // Signature: "CHIP" (4 bytes)
        packet[0..4].copy_from_slice(b"CHIP");

        // Packet length (4 bytes) - big endian
        packet[4..8].copy_from_slice(&(packet_size as u32).to_be_bytes());

        // Chip type: 2 = Flash ROM (EasyFlash uses type 2 for both ROML and ROMH)
        packet[8..10].copy_from_slice(&2u16.to_be_bytes());

        // Bank number (2 bytes) - big endian
        packet[10..12].copy_from_slice(&(bank_number as u16).to_be_bytes());

        // Starting address (2 bytes) - big endian
        packet[12..14].copy_from_slice(&start_address.to_be_bytes());

        // ROM length (2 bytes) - big endian
        packet[14..16].copy_from_slice(&(data.len() as u16).to_be_bytes());

        // ROM data
        packet[16..].copy_from_slice(data);

        packet
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_crt_builder() {
        let builder = CRTBuilder::new(CartridgeType::EasyFlash, 8, "Test Cartridge").unwrap();
        assert_eq!(builder.bank_count(), 8);
    }

    #[test]
    fn test_add_bank() {
        let mut builder = CRTBuilder::new(CartridgeType::EasyFlash, 1, "Test").unwrap();
        assert_eq!(builder.bank_count(), 1);
        builder.add_bank();
        assert_eq!(builder.bank_count(), 2);
    }

    #[test]
    fn test_fill_bank() {
        let mut builder = CRTBuilder::new(CartridgeType::EasyFlash, 1, "Test").unwrap();
        let data = [0x12, 0x34, 0x56];
        builder.fill_bank(0, &data, 0).unwrap();
        let bank = builder.get_bank(0).unwrap();
        assert_eq!(&bank[0..3], &data);
    }
}
