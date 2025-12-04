//! Command-line interface for VICE Snapshot to PRG/CRT Converter
//!
//! Usage: vice-snapshot-to-prg-converter-cli [OPTIONS] <input.vsf> <output>
//!
// Copyright (c) 2025 Tommy Olsen
// Licensed under the MIT License.

use std::env;
use std::path::Path;
use std::process;

use vice_snapshot_to_prg_converter::config::{Config, CrtConfig, VERSION};
use vice_snapshot_to_prg_converter::convert_snapshot::ConvertSnapshot;
use vice_snapshot_to_prg_converter::convert_snapshot_crt::ConvertSnapshotCRT;

#[derive(Debug, PartialEq)]
enum OutputFormat {
    Prg,
    Crt,
}

struct CliArgs {
    input_path: String,
    output_path: String,
    format: OutputFormat,
    cartridge_name: Option<String>,
    include_dir: Option<String>,
    hook_addr: Option<u16>,
}

fn main() {
    let args: Vec<String> = env::args().collect();

    // Check for help flag first
    if args.contains(&"--help".to_string()) || args.contains(&"-h".to_string()) {
        print_usage(&args[0]);
        process::exit(0);
    }

    let cli_args = match parse_args(&args) {
        Ok(args) => args,
        Err(e) => {
            eprintln!("Error: {}", e);
            eprintln!();
            print_usage(&args[0]);
            process::exit(1);
        }
    };

    // Validate input file
    if !Path::new(&cli_args.input_path).exists() {
        eprintln!("Error: Input file not found: {}", cli_args.input_path);
        process::exit(1);
    }

    if !cli_args.input_path.to_lowercase().ends_with(".vsf") {
        eprintln!("Warning: Input file does not have .vsf extension");
        eprintln!();
    }

    // Validate output extension matches format
    let output_lower = cli_args.output_path.to_lowercase();
    match cli_args.format {
        OutputFormat::Prg if !output_lower.ends_with(".prg") => {
            eprintln!("Warning: Output file does not have .prg extension");
            eprintln!();
        }
        OutputFormat::Crt if !output_lower.ends_with(".crt") => {
            eprintln!("Warning: Output file does not have .crt extension");
            eprintln!();
        }
        _ => {}
    }

    // Warn if CRT-only options used with PRG
    if cli_args.format == OutputFormat::Prg {
        if cli_args.include_dir.is_some() {
            eprintln!("Warning: --include-dir is only used with CRT format, ignoring");
            eprintln!();
        }
        if cli_args.hook_addr.is_some() {
            eprintln!("Warning: --hook-addr is only used with CRT format, ignoring");
            eprintln!();
        }
    }

    // Warn if hook-addr used without include-dir
    if cli_args.hook_addr.is_some() && cli_args.include_dir.is_none() {
        eprintln!("Warning: --hook-addr requires --include-dir, ignoring");
        eprintln!();
    }

    // Validate include directory exists
    if let Some(ref dir) = cli_args.include_dir {
        let path = Path::new(dir);
        if !path.exists() {
            eprintln!("Error: Include directory not found: {}", dir);
            process::exit(1);
        }
        if !path.is_dir() {
            eprintln!("Error: Include path is not a directory: {}", dir);
            process::exit(1);
        }
    }

    // Handle existing output file
    if Path::new(&cli_args.output_path).exists() {
        println!("Output file exists, overwriting: {}", cli_args.output_path);
        if let Err(e) = std::fs::remove_file(&cli_args.output_path) {
            eprintln!("Error: Failed to delete existing output file: {}", e);
            process::exit(1);
        }
    }

    let format_str = match cli_args.format {
        OutputFormat::Prg => "PRG",
        OutputFormat::Crt => "CRT",
    };

    println!("VICE Snapshot to PRG/CRT Converter v{} (CLI)", VERSION);
    println!();
    println!("Input:  {}", cli_args.input_path);
    println!("Output: {} ({})", cli_args.output_path, format_str);
    if let Some(ref name) = cli_args.cartridge_name {
        println!("Name:   {}", name);
    }
    if let Some(ref dir) = cli_args.include_dir {
        println!("Include: {}", dir);
    }
    if let Some(addr) = cli_args.hook_addr {
        println!("Hook:    ${:04X}", addr);
    }
    println!();
    println!("Converting...");

    let result = match cli_args.format {
        OutputFormat::Prg => convert_prg(&cli_args),
        OutputFormat::Crt => convert_crt(&cli_args),
    };

    match result {
        Ok(()) => {
            println!();
            println!("Success!");
            println!("  Snapshot converted to: {}", cli_args.output_path);
            println!();
            process::exit(0);
        }
        Err(e) => {
            eprintln!();
            eprintln!("Conversion failed:");
            eprintln!("  {}", e);
            eprintln!();
            process::exit(1);
        }
    }
}

fn parse_args(args: &[String]) -> Result<CliArgs, String> {
    let mut format: Option<OutputFormat> = None;
    let mut cartridge_name: Option<String> = None;
    let mut include_dir: Option<String> = None;
    let mut hook_addr: Option<u16> = None;
    let mut positional: Vec<String> = Vec::new();

    let mut i = 1;
    while i < args.len() {
        let arg = &args[i];

        match arg.as_str() {
            "--prg" => {
                if format.is_some() {
                    return Err("Cannot specify both --prg and --crt".to_string());
                }
                format = Some(OutputFormat::Prg);
            }
            "--crt" => {
                if format.is_some() {
                    return Err("Cannot specify both --prg and --crt".to_string());
                }
                format = Some(OutputFormat::Crt);
            }
            "--name" => {
                i += 1;
                if i >= args.len() {
                    return Err("--name requires a value".to_string());
                }
                let name = &args[i];
                if name.len() > 32 {
                    return Err("Cartridge name cannot exceed 32 characters".to_string());
                }
                cartridge_name = Some(name.clone());
            }
            "--include-dir" => {
                i += 1;
                if i >= args.len() {
                    return Err("--include-dir requires a path".to_string());
                }
                include_dir = Some(args[i].clone());
            }
            "--hook-addr" => {
                i += 1;
                if i >= args.len() {
                    return Err("--hook-addr requires a hex address".to_string());
                }
                let addr_str = args[i].trim_start_matches('$').trim_start_matches("0x");
                let addr = u16::from_str_radix(addr_str, 16)
                    .map_err(|_| format!("Invalid hex address: {}", args[i]))?;
                hook_addr = Some(addr);
            }
            _ if arg.starts_with('-') => {
                return Err(format!("Unknown option: {}", arg));
            }
            _ => {
                positional.push(arg.clone());
            }
        }
        i += 1;
    }

    if positional.len() != 2 {
        return Err("Expected exactly 2 arguments: <input.vsf> <output>".to_string());
    }

    let input_path = positional[0].clone();
    let output_path = positional[1].clone();

    // Auto-detect format from output extension if not specified
    let format = format.unwrap_or_else(|| {
        if output_path.to_lowercase().ends_with(".crt") {
            OutputFormat::Crt
        } else {
            OutputFormat::Prg
        }
    });

    Ok(CliArgs {
        input_path,
        output_path,
        format,
        cartridge_name,
        include_dir,
        hook_addr,
    })
}

fn convert_prg(cli_args: &CliArgs) -> Result<(), String> {
    let config = Config::auto()
        .map_err(|e| format!("Failed to initialize: {}", e))?;

    let work_path = config.work_path.clone();
    let converter = ConvertSnapshot::new(config);
    let result = converter.convert(&cli_args.input_path, &cli_args.output_path);

    let _ = cleanup_work_dir(&work_path);
    result
}

fn convert_crt(cli_args: &CliArgs) -> Result<(), String> {
    let mut config = CrtConfig::auto()
        .map_err(|e| format!("Failed to initialize: {}", e))?;

    if let Some(ref name) = cli_args.cartridge_name {
        config = config.with_cartridge_name(name);
    }

    if let Some(ref dir) = cli_args.include_dir {
        config = config.with_include_dir(dir);
    }

    if let Some(addr) = cli_args.hook_addr {
        config = config.with_trampoline_address(addr);
    }

    let work_path = config.base_config.work_path.clone();
    let converter = ConvertSnapshotCRT::new(config);
    let result = converter.convert(&cli_args.input_path, &cli_args.output_path);

    let _ = cleanup_work_dir(&work_path);
    result
}

fn cleanup_work_dir(work_path: &Path) -> Result<(), String> {
    if work_path.exists() {
        std::fs::remove_dir_all(work_path)
            .map_err(|e| format!("Failed to remove work directory {:?}: {}", work_path, e))?;
    }
    Ok(())
}

fn print_usage(program_name: &str) {
    let name = Path::new(program_name)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("vice-snapshot-to-prg-converter-cli");

    println!("VICE Snapshot to PRG/CRT Converter v{} (CLI)", VERSION);
    println!();
    println!("USAGE:");
    println!("  {} [OPTIONS] <input.vsf> <output>", name);
    println!();
    println!("DESCRIPTION:");
    println!("  Converts VICE 3.6-3.9 x64sc snapshot files (.vsf) to:");
    println!("  - PRG: Self-restoring C64 PRG files");
    println!("  - CRT: EasyFlash cartridge files");
    println!();
    println!("  Output format is auto-detected from file extension, or use --prg/--crt.");
    println!("  Existing output files are overwritten without prompting.");
    println!();
    println!("ARGUMENTS:");
    println!("  <input.vsf>   Path to input VICE snapshot file");
    println!("  <output>      Path to output file (.prg or .crt)");
    println!();
    println!("OPTIONS:");
    println!("  --prg                Force PRG format output");
    println!("  --crt                Force EasyFlash CRT format output");
    println!("  --name <name>        Cartridge name (CRT only, max 32 chars)");
    println!("  --include-dir <dir>  Include PRG files from directory (CRT only)");
    println!("  --hook-addr <hex>    LOAD/SAVE hook address (CRT only, overrides auto)");
    println!("  -h, --help           Show this help message");
    println!();
    println!("EXAMPLES:");
    println!("  {} snapshot.vsf output.prg", name);
    println!("  {} snapshot.vsf output.crt", name);
    println!("  {} --crt --name \"My Game\" snapshot.vsf game.crt", name);
    println!("  {} --crt --include-dir ./files snapshot.vsf game.crt", name);
    println!("  {} --crt --include-dir ./files --hook-addr $0334 snapshot.vsf game.crt", name);
    println!();
    println!("IMPORTANT:");
    println!("  - Only works with VICE 3.6-3.9 x64sc snapshots");
    println!("  - Memory MUST be initialized before snapshot (f 0000 ffff 00)");
    println!();
    println!("For more information:");
    println!("  https://github.com/tommyo123/Vice_Snapshot_to_PRG");
    println!();
}
