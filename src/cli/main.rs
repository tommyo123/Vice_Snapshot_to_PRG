//! Command-line interface for VICE Snapshot to PRG Converter
//!
//! Usage: vice-snapshot-to-prg-converter-cli <input.vsf> <output.prg>
//!
//! This program is unlicensed and dedicated to the public domain.
//! Developed by Tommy Olsen.

use std::env;
use std::path::Path;
use std::process;

// Import the library crate modules
use vice_snapshot_to_prg_converter::config::{Config, VERSION};
use vice_snapshot_to_prg_converter::convert_snapshot::ConvertSnapshot;

fn main() {
    let args: Vec<String> = env::args().collect();

    // Check for help flag or wrong number of arguments
    if args.len() != 3 || args.contains(&"--help".to_string()) || args.contains(&"-h".to_string()) {
        print_usage(&args[0]);
        process::exit(if args.len() == 2 && (args[1] == "--help" || args[1] == "-h") { 0 } else { 1 });
    }

    let input_path = &args[1];
    let output_path = &args[2];

    // Validate input file exists
    if !Path::new(input_path).exists() {
        eprintln!("Error: Input file not found: {}", input_path);
        eprintln!();
        print_usage(&args[0]);
        process::exit(1);
    }

    // Validate input file extension
    if !input_path.to_lowercase().ends_with(".vsf") {
        eprintln!("Warning: Input file does not have .vsf extension");
        eprintln!("         Expected a VICE snapshot file");
        eprintln!();
    }

    // Validate output file extension
    if !output_path.to_lowercase().ends_with(".prg") {
        eprintln!("Warning: Output file does not have .prg extension");
        eprintln!();
    }

    // Delete output file if it exists (no prompting in CLI mode)
    if Path::new(output_path).exists() {
        println!("Output file exists, overwriting: {}", output_path);
        if let Err(e) = std::fs::remove_file(output_path) {
            eprintln!("Error: Failed to delete existing output file: {}", e);
            process::exit(1);
        }
    }

    println!("VICE Snapshot to PRG Converter v{} (CLI)", VERSION);
    println!();
    println!("Input:  {}", input_path);
    println!("Output: {}", output_path);
    println!();
    println!("Converting...");

    // Create config with automatic paths
    let config = match Config::auto() {
        Ok(cfg) => cfg,
        Err(e) => {
            eprintln!("Error: Failed to initialize: {}", e);
            process::exit(1);
        }
    };

    let work_path = config.work_path.clone();

    // Perform conversion
    let converter = ConvertSnapshot::new(config);
    let result = converter.convert(input_path, output_path);

    // Clean up work directory
    let _ = cleanup_work_dir(&work_path);

    // Handle result
    match result {
        Ok(()) => {
            println!();
            println!("✓ Success!");
            println!("  Snapshot converted to: {}", output_path);
            println!();
            process::exit(0);
        }
        Err(e) => {
            eprintln!();
            eprintln!("✗ Conversion failed:");
            eprintln!("  {}", e);
            eprintln!();
            process::exit(1);
        }
    }
}

fn print_usage(program_name: &str) {
    let name = Path::new(program_name)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("vice-snapshot-to-prg-converter-cli");

    println!("VICE Snapshot to PRG Converter v{} (CLI)", VERSION);
    println!();
    println!("USAGE:");
    println!("  {} <input.vsf> <output.prg>", name);
    println!();
    println!("DESCRIPTION:");
    println!("  Converts VICE 3.6-3.9 x64sc snapshot files (.vsf) to self-restoring");
    println!("  C64 PRG files that run on real Commodore 64 hardware.");
    println!();
    println!("  The output file will be overwritten without prompting if it exists.");
    println!();
    println!("ARGUMENTS:");
    println!("  <input.vsf>   Path to input VICE snapshot file");
    println!("  <output.prg>  Path to output C64 PRG file");
    println!();
    println!("OPTIONS:");
    println!("  -h, --help    Show this help message");
    println!();
    println!("EXAMPLES:");
    println!("  {} snapshot.vsf output.prg", name);
    println!("  {} ./saves/game.vsf ./prg/game.prg", name);
    println!();
    println!("IMPORTANT:");
    println!("  - Only works with VICE 3.9 x64sc snapshots");
    println!("  - Memory MUST be initialized before snapshot (f 0000 ffff 00)");
    println!("  - Do NOT use \"Smart attach...\" feature in VICE");
    println!();
    println!("For more information:");
    println!("  https://github.com/tommyo123/Vice_Snapshot_to_PRG");
    println!();
}

/// Clean up the temporary work directory
fn cleanup_work_dir(work_path: &Path) -> Result<(), String> {
    if work_path.exists() {
        std::fs::remove_dir_all(work_path)
            .map_err(|e| format!("Failed to remove work directory {:?}: {}", work_path, e))?;
    }
    Ok(())
}
