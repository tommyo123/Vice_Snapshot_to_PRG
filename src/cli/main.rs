//! Command-line interface for VICE Snapshot to PRG/CRT Converter
//!
//! Usage: vice-snapshot-to-prg-converter-cli [OPTIONS] <input.vsf> <output>
//!
// Copyright (c) 2025-2026 Tommy Olsen
// Licensed under the MIT License.

use std::env;
use std::path::Path;
use std::process;

use vice_snapshot_to_prg_converter::config::{Config, CrtConfig, VERSION, InputMode, FreezeMethod};
use vice_snapshot_to_prg_converter::convert_snapshot::ConvertSnapshot;
use vice_snapshot_to_prg_converter::convert_snapshot_crt::ConvertSnapshotCRT;
use vice_snapshot_to_prg_converter::convert_snapshot_magic_desk_crt::ConvertSnapshotMagicDeskCRT;
use vice_snapshot_to_prg_converter::util::paths_refer_to_same_file;

#[derive(Debug, PartialEq)]
enum OutputFormat {
    Prg,
    Crt,
    MagicDeskCrt,
}

struct CliArgs {
    input_path: String,
    output_path: String,
    format: OutputFormat,
    cartridge_name: Option<String>,
    include_dir: Option<String>,
    hook_addr: Option<u16>,
    input_mode: InputMode,
    clear_poweron_ram: bool,
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

    if !cli_args.input_path.to_lowercase().ends_with(".vsf")
        && !matches!(cli_args.input_mode, InputMode::Freeze(_))
    {
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
        OutputFormat::Crt | OutputFormat::MagicDeskCrt if !output_lower.ends_with(".crt") => {
            eprintln!("Warning: Output file does not have .crt extension");
            eprintln!();
        }
        _ => {}
    }

    // Warn if CRT-only options used with PRG
    if cli_args.format == OutputFormat::Prg {
        if cli_args.include_dir.is_some() {
            eprintln!("Warning: --include-dir is only used with EasyFlash CRT format, ignoring");
            eprintln!();
        }
        if cli_args.hook_addr.is_some() {
            eprintln!("Warning: --hook-addr is only used with the CRT formats, ignoring");
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

    // Never let the output clobber the source file.
    if paths_refer_to_same_file(&cli_args.input_path, &cli_args.output_path) {
        eprintln!(
            "Error: Output file is the same as the input file: {}",
            cli_args.input_path
        );
        eprintln!("Choose a different output filename so the source is not overwritten.");
        process::exit(1);
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
        OutputFormat::Crt => "EasyFlash CRT",
        OutputFormat::MagicDeskCrt => "Magic Desk CRT",
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
    // Print the hook address only when it is actually honored
    // (a CRT format with an include dir).
    if let Some(addr) = cli_args.hook_addr {
        if cli_args.format != OutputFormat::Prg && cli_args.include_dir.is_some() {
            println!("Hook:    ${:04X}", addr);
        }
    }
    println!();
    println!("Converting...");

    let result = match cli_args.format {
        OutputFormat::Prg => convert_prg(&cli_args),
        OutputFormat::Crt => convert_crt(&cli_args),
        OutputFormat::MagicDeskCrt => convert_magic_desk_crt(&cli_args),
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
    let mut input_mode: Option<InputMode> = None;
    let mut clear_poweron_ram = false;
    let mut positional: Vec<String> = Vec::new();

    let mut i = 1;
    while i < args.len() {
        let arg = &args[i];

        match arg.as_str() {
            "--prg" => {
                if format.is_some() {
                    return Err("Cannot specify multiple format flags".to_string());
                }
                format = Some(OutputFormat::Prg);
            }
            "--crt" => {
                if format.is_some() {
                    return Err("Cannot specify multiple format flags".to_string());
                }
                format = Some(OutputFormat::Crt);
            }
            "--magic-desk" => {
                if format.is_some() {
                    return Err("Cannot specify multiple format flags".to_string());
                }
                format = Some(OutputFormat::MagicDeskCrt);
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
            "--vsf" => {
                if input_mode.is_some() {
                    return Err("Cannot combine --vsf and --freezer".to_string());
                }
                input_mode = Some(InputMode::Vsf);
            }
            "--freezer" => {
                i += 1;
                if i >= args.len() {
                    return Err("--freezer requires a value (auto|ar|isepic|fc3)".to_string());
                }
                if input_mode.is_some() {
                    return Err("Cannot combine --vsf and --freezer".to_string());
                }
                let method = match args[i].to_lowercase().as_str() {
                    "auto" => FreezeMethod::Auto,
                    "ar" | "self" | "ss5" | "fm" | "expert" => FreezeMethod::SelfRestoring,
                    "isepic" => FreezeMethod::Isepic,
                    "fc3" => FreezeMethod::Fc3,
                    other => {
                        return Err(format!(
                            "Unknown --freezer value '{}' (use auto|ar|isepic|fc3)",
                            other
                        ))
                    }
                };
                input_mode = Some(InputMode::Freeze(method));
            }
            "--clear-poweron-ram" => {
                clear_poweron_ram = true;
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
        input_mode: input_mode.unwrap_or(InputMode::Auto),
        clear_poweron_ram,
    })
}

/// Print the result of the power-on RAM pattern pass.
fn report_poweron_clear(cli_args: &CliArgs, cleared: u32) {
    if cli_args.clear_poweron_ram {
        println!("Power-on RAM pattern cleared: {} bytes", cleared);
    } else {
        println!("Power-on RAM pattern clearing: off");
    }
}

fn convert_prg(cli_args: &CliArgs) -> Result<(), String> {
    let mut config = Config::auto()
        .map_err(|e| format!("Failed to initialize: {}", e))?
        .with_clear_poweron(cli_args.clear_poweron_ram);
    config.input_mode = cli_args.input_mode;

    let work_path = config.work_path.clone();
    let converter = ConvertSnapshot::new(config);
    let result = converter.convert(&cli_args.input_path, &cli_args.output_path);
    if result.is_ok() {
        report_poweron_clear(cli_args, converter.poweron_cleared());
    }

    let _ = cleanup_work_dir(&work_path);
    result
}

fn convert_crt(cli_args: &CliArgs) -> Result<(), String> {
    let mut config = CrtConfig::auto()
        .map_err(|e| format!("Failed to initialize: {}", e))?;
    config.base_config.input_mode = cli_args.input_mode;
    config.base_config.clear_poweron_ram = cli_args.clear_poweron_ram;

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
    if result.is_ok() {
        report_poweron_clear(cli_args, converter.poweron_cleared());
    }

    let _ = cleanup_work_dir(&work_path);
    result
}

fn convert_magic_desk_crt(cli_args: &CliArgs) -> Result<(), String> {
    let mut config = CrtConfig::auto()
        .map_err(|e| format!("Failed to initialize: {}", e))?;
    config.base_config.input_mode = cli_args.input_mode;
    config.base_config.clear_poweron_ram = cli_args.clear_poweron_ram;

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
    let converter = ConvertSnapshotMagicDeskCRT::new(config);
    let result = converter.convert(&cli_args.input_path, &cli_args.output_path);
    if result.is_ok() {
        report_poweron_clear(cli_args, converter.poweron_cleared());
    }

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
    println!("  Converts VICE snapshot files (.vsf) to:");
    println!("  - PRG: Self-restoring C64 PRG files");
    println!("  - CRT: EasyFlash cartridge files (with optional LOAD/SAVE hooking)");
    println!("  - CRT: Magic Desk cartridge files (8K cart mode, ROML only)");
    println!();
    println!("  Output format is auto-detected from file extension, or use --prg/--crt/--magic-desk.");
    println!("  Existing output files are overwritten without prompting.");
    println!();
    println!("ARGUMENTS:");
    println!("  <input.vsf>   Path to input VICE snapshot file");
    println!("  <output>      Path to output file (.prg or .crt)");
    println!();
    println!("OPTIONS:");
    println!("  --prg                Force PRG format output");
    println!("  --crt                Force EasyFlash CRT format output");
    println!("  --magic-desk         Force Magic Desk CRT format output");
    println!("  --name <name>        Cartridge name (CRT only, max 32 chars)");
    println!("  --include-dir <dir>  Include PRG files from directory (EasyFlash or Magic Desk)");
    println!("  --hook-addr <hex>    LOAD/SAVE hook address (EasyFlash or Magic Desk, overrides");
    println!("                       the automatic placement: $0100 when the snapshot's stack");
    println!("                       pointer allows it, otherwise $0334)");
    println!("  --vsf                Force VSF snapshot input (do not treat as a freeze)");
    println!("  --freezer <type>     Convert a cartridge freeze; type = auto|ar|isepic|fc3");
    println!("                         auto   = detect the freezer automatically");
    println!("                         ar     = Action Replay / Super Snapshot 5 / Freeze Machine / Expert");
    println!("                         isepic = ISEPIC (feed the '-name' data file)");
    println!("                         fc3    = Final Cartridge III (feed 'fc'; '-fc' auto-found)");
    println!("                       (default: auto-detect freeze, else VSF)");
    println!("  --clear-poweron-ram  HIGHLY EXPERIMENTAL, off by default. Zeroes RAM regions");
    println!("                       still holding the C64 power-on pattern (64+ exact pattern");
    println!("                       bytes) to create free blocks. Misdetection could lose");
    println!("                       program data; only enable if you understand the risk");
    println!("  -h, --help           Show this help message");
    println!();
    println!("EXAMPLES:");
    println!("  {} snapshot.vsf output.prg", name);
    println!("  {} snapshot.vsf output.crt", name);
    println!("  {} --crt --name \"My Game\" snapshot.vsf game.crt", name);
    println!("  {} --crt --include-dir ./files snapshot.vsf game.crt", name);
    println!("  {} --crt --include-dir ./files --hook-addr $0334 snapshot.vsf game.crt", name);
    println!("  {} --magic-desk --name \"My Game\" snapshot.vsf game.crt", name);
    println!("  {} --magic-desk --include-dir ./files snapshot.vsf game.crt", name);
    println!("  {} --freezer ar  freeze.prg out.prg            (force Action Replay family)", name);
    println!("  {} --freezer fc3 fc.prg out.crt --crt          (Final Cartridge III freeze)", name);
    println!();
    println!("IMPORTANT:");
    println!("  - Memory MUST be initialized before snapshot (f 0000 ffff 00)");
    println!("  - --clear-poweron-ram is an experimental alternative; see above");
    println!();
    println!("For more information:");
    println!("  https://github.com/tommyo123/Vice_Snapshot_to_PRG");
    println!();
}
