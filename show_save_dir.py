#!/usr/bin/env python3
"""
C64 EasyFlash Cartridge EFS Directory Viewer
Parses the SAVE banks of an EasyFlash cartridge (.crt) containing drunella's libefs 
filesystem and prints the directory, including overwritten and deleted files.
"""

import argparse
import sys
import struct
import os

# Mode constants from libefs
MODE_LHLH = 0xD0
MODE_LLLL = 0xB0
MODE_HHHH = 0xD4

def petscii_to_ascii(b):
    """Convert PETSCII-encoded bytes to ASCII string, showing non-printables as dots."""
    chars = []
    for x in b:
        if x == 0:
            break
        # standard uppercase/graphic C64 PETSCII mapping
        if 0x41 <= x <= 0x5A:
            chars.append(chr(x)) # Uppercase A-Z
        elif 0xC1 <= x <= 0xDA:
            chars.append(chr(x - 0x80).lower()) # Shifted A-Z -> lowercase a-z
        elif 32 <= x < 127:
            chars.append(chr(x))
        else:
            chars.append('.')
    return "".join(chars).strip()

def parse_crt(file_path):
    """Parse CRT file and return dictionaries of LOROM and ROMH banks."""
    if not os.path.exists(file_path):
        print(f"Error: File not found: {file_path}", file=sys.stderr)
        sys.exit(1)
        
    with open(file_path, "rb") as f:
        data = f.read()
        
    if len(data) < 64:
        print("Error: File too small to be a CRT cartridge", file=sys.stderr)
        sys.exit(1)
        
    signature = data[0:16]
    if not signature.startswith(b"C64 CARTRIDGE"):
        print("Error: Invalid CRT signature", file=sys.stderr)
        sys.exit(1)
        
    header_len = struct.unpack(">I", data[16:20])[0]
    hw_type = struct.unpack(">H", data[22:24])[0]
    
    if hw_type != 32:
        print(f"Warning: Cartridge hardware type is {hw_type}, expected 32 (EasyFlash)", file=sys.stderr)
        
    cart_name = data[32:64].split(b'\x00')[0].decode('ascii', errors='ignore').strip()
    
    roml_banks = {}
    romh_banks = {}
    
    offset = header_len
    while offset < len(data):
        if offset + 16 > len(data):
            break
            
        chip_sig = data[offset:offset+4]
        if chip_sig != b"CHIP":
            print(f"Error: Invalid CHIP packet signature at offset {offset}", file=sys.stderr)
            break
            
        packet_len = struct.unpack(">I", data[offset+4:offset+8])[0]
        # chip_type = struct.unpack(">H", data[offset+8:offset+10])[0] # 2 = Flash ROM
        bank = struct.unpack(">H", data[offset+10:offset+12])[0]
        load_addr = struct.unpack(">H", data[offset+12:offset+14])[0]
        # data_len = struct.unpack(">H", data[offset+14:offset+16])[0]
        
        chip_data = data[offset+16:offset+packet_len]
        
        if load_addr == 0xE000:
            romh_banks[bank] = chip_data
        elif load_addr == 0x8000:
            roml_banks[bank] = chip_data
            
        offset += packet_len
        
    return cart_name, hw_type, roml_banks, romh_banks

def parse_efs_config(romh_banks):
    """Extract libefs configuration from bank 0 HIROM if present."""
    bank0_hirom = romh_banks.get(0)
    if not bank0_hirom or len(bank0_hirom) < 8192:
        return None
        
    # LIBEFS config block is at offset 0x1B18 of bank 0 HIROM
    sig_offset = 0x1B18
    sig = bank0_hirom[sig_offset:sig_offset+6]
    if sig != b"LIBEFS":
        return None
        
    def parse_area(offset):
        area_bytes = bank0_hirom[offset:offset+6]
        return {
            'dir_bank': area_bytes[0],
            'dir_high': area_bytes[1],
            'files_bank': area_bytes[2],
            'files_high': area_bytes[3],
            'num_banks': area_bytes[4],
            'mode': area_bytes[5],
        }
        
    return {
        'area0': parse_area(0x1B22),
        'area1': parse_area(0x1B28),
        'area2': parse_area(0x1B2E),
    }

def print_directory(title, dir_bytes):
    """Parse and print directory entries from the 6 KB directory bytes."""
    print(f"\n==============================================================================")
    print(f" {title}")
    print(f"==============================================================================")
    
    entries = []
    for i in range(256):
        offset = i * 24
        entry = dir_bytes[offset:offset+24]
        
        # Check if empty/erased (flash is 0xFF)
        if entry == b'\xff' * 24 or entry[0] == 0xFF:
            continue
            
        name = petscii_to_ascii(entry[0:16])
        flags = entry[16]
        bank = entry[17]
        bank_high = entry[18]
        start_offset = entry[19] | (entry[20] << 8)
        size = entry[21] | (entry[22] << 8) | (entry[23] << 16)
        
        # libefs deletes/overwrites by writing 0x00 to flags or clearing type
        is_deleted_flag = (flags == 0) or ((flags & 0x1F) == 0)
        
        entries.append({
            'slot': i,
            'name': name,
            'flags': flags,
            'bank': bank,
            'offset': start_offset,
            'size': size,
            'is_deleted_flag': is_deleted_flag,
        })
        
    if not entries:
        print("  (Directory is empty)")
        return
        
    # Compute entry status:
    # - "Overwritten": if a later non-deleted entry with the same name exists
    # - "Deleted": if explicitly marked deleted or the latest version is deleted
    # - "Active": if it's the latest version and is not marked deleted
    for i, entry in enumerate(entries):
        later_active_duplicate = False
        for later_entry in entries[i+1:]:
            if later_entry['name'] == entry['name'] and not later_entry['is_deleted_flag']:
                later_active_duplicate = True
                break
                
        if later_active_duplicate:
            entry['status'] = "Overwritten"
        elif entry['is_deleted_flag']:
            entry['status'] = "Deleted"
        else:
            # Check if there is a later deleted duplicate which deleted this file
            later_deleted_duplicate = False
            for later_entry in entries[i+1:]:
                if later_entry['name'] == entry['name'] and later_entry['is_deleted_flag']:
                    later_deleted_duplicate = True
                    break
            if later_deleted_duplicate:
                entry['status'] = "Overwritten"
            else:
                entry['status'] = "Active"
                
    # Sort or display in sequence
    print(f"{'Slot':<6} {'File Name':<18} {'Size (Bytes)':<12} {'Size (Hex)':<10} {'Bank':<6} {'Offset':<8} {'Flags':<6} {'Status':<12}")
    print("-" * 78)
    for entry in entries:
        size_dec = entry['size']
        size_hex = f"${size_dec:04X}"
        offset_hex = f"${entry['offset']:04X}"
        flags_hex = f"${entry['flags']:02X}"
        bank_str = f"{entry['bank']}"
        
        print(f"{entry['slot']:<6} {entry['name']:<18} {size_dec:<12} {size_hex:<10} {bank_str:<6} {offset_hex:<8} {flags_hex:<6} {entry['status']:<12}")

def main():
    parser = argparse.ArgumentParser(description="View directory structure and overwritten files in C64 EasyFlash libefs SAVE cartridges.")
    parser.add_argument("crt_file", help="Path to the EasyFlash .crt file")
    parser.add_argument("--show-ro", action="store_true", help="Also display read-only Area 0 files")
    args = parser.parse_args()
    
    cart_name, hw_type, roml_banks, romh_banks = parse_crt(args.crt_file)
    
    print(f"Cartridge Image: {args.crt_file}")
    print(f"Name: {cart_name}")
    
    config = parse_efs_config(romh_banks)
    
    area1_is_lorom = False
    area2_is_lorom = False
    area0_is_lorom = False
    
    if config:
        print("Detected configuration: libefs config block present in bank 0.")
        area1_bank = config['area1']['dir_bank']
        area1_banks = config['area1']['num_banks']
        area1_is_lorom = (config['area1']['dir_high'] == 0x80)
        
        area2_bank = config['area2']['dir_bank']
        area2_banks = config['area2']['num_banks']
        area2_is_lorom = (config['area2']['dir_high'] == 0x80)
        
        area0_bank = config['area0']['dir_bank']
        area0_is_lorom = (config['area0']['dir_high'] == 0x80)
    else:
        print("Detected configuration: None (using default save area banks 48-55 / 56-63).")
        area1_bank = 48
        area1_banks = 8
        area2_bank = 56
        area2_banks = 8
        area0_bank = 0
        
    # Show Area 0 if requested
    if args.show_ro:
        banks_map = roml_banks if area0_is_lorom else romh_banks
        bank0_dir = banks_map.get(area0_bank)
        if bank0_dir and len(bank0_dir) >= 0x1800:
            print_directory("Read-Only Area 0 (System/Defaults)", bank0_dir[:0x1800])
        else:
            print("\nRead-Only Area 0: bank 0 data missing or too small.")
            
    # Show Area 1 (Writable Ping)
    banks_map = roml_banks if area1_is_lorom else romh_banks
    first_bank_area1 = banks_map.get(area1_bank)
    if first_bank_area1:
        type_str = "LOROM" if area1_is_lorom else "HIROM"
        print_directory(f"Save Area 1 ({type_str}, Banks {area1_bank}-{area1_bank + area1_banks - 1})", first_bank_area1[:0x1800])
    else:
        print(f"\nSave Area 1 (Bank {area1_bank}): Bank data not found in CRT.")
        
    # Show Area 2 (Writable Pong)
    banks_map = roml_banks if area2_is_lorom else romh_banks
    first_bank_area2 = banks_map.get(area2_bank)
    if first_bank_area2:
        type_str = "LOROM" if area2_is_lorom else "HIROM"
        print_directory(f"Save Area 2 ({type_str}, Banks {area2_bank}-{area2_bank + area2_banks - 1})", first_bank_area2[:0x1800])
    else:
        print(f"\nSave Area 2 (Bank {area2_bank}): Bank data not found in CRT.")

if __name__ == "__main__":
    main()
