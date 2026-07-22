//! Converts VICE snapshot images (VSF) to C64 PRG files, EasyFlash CRT or Magic Desk CRT cartridges.
//!
// Copyright (c) 2025-2026 Tommy Olsen
// Licensed under the MIT License.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use fltk::{prelude::*, *};
use fltk::button::{Button, CheckButton};
use fltk::dialog::NativeFileChooser;
use fltk::enums::{Align, Color, FrameType};
use fltk::frame::Frame;
use fltk::group::Tabs;
use fltk::image::SvgImage;
use fltk::input::Input;
use fltk::text::{TextBuffer, TextDisplay};
use fltk::window::Window;
use std::cell::RefCell;
use std::rc::Rc;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use vice_snapshot_to_prg_converter::config::{finish_conversion, Config, CrtConfig, VERSION, InputMode, FreezeMethod, PackFormat, WorkDirGuard};
use vice_snapshot_to_prg_converter::convert_snapshot::ConvertSnapshot;
use vice_snapshot_to_prg_converter::convert_snapshot_crt::ConvertSnapshotCRT;
use vice_snapshot_to_prg_converter::convert_snapshot_magic_desk_crt::ConvertSnapshotMagicDeskCRT;
use vice_snapshot_to_prg_converter::progress::{is_cancelled_error, Progress};
use vice_snapshot_to_prg_converter::util::paths_refer_to_same_file;

const WINDOW_WIDTH: i32 = 720;
const WINDOW_HEIGHT: i32 = 885;
const MARGIN: i32 = 25;
const FIELD_HEIGHT: i32 = 35;
const BUTTON_HEIGHT: i32 = 40;
const BUTTON_WIDTH: i32 = 120;
const BROWSE_BTN_WIDTH: i32 = 60;
const TAB_HEIGHT: i32 = 490;

/// File-browser filter for the input file, with the relevant type shown FIRST
/// (the default the dialog displays). `input_type`: 0 = VSF, 1 = Cartridge freeze.
fn input_browse_filter(input_type: i32) -> &'static str {
    if input_type == 1 {
        "Cartridge Freezes\t*.prg\nVSF Snapshots\t*.vsf\nAll Files\t*"
    } else {
        "VSF Snapshots\t*.vsf\nCartridge Freezes\t*.prg\nAll Files\t*"
    }
}

/// Suggested output path for a chosen input: same directory and file stem with
/// a `_vs` suffix (so the source file is never proposed as the output) and the
/// given extension. e.g. `game.vsf` + "prg" -> `game_vs.prg`.
fn suggested_output_path(input: &Path, ext: &str) -> PathBuf {
    let stem = input
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let file_name = format!("{}_vs.{}", stem, ext);
    match input.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.join(file_name),
        _ => PathBuf::from(file_name),
    }
}

/// Map the two input-type GUI choices to an [`InputMode`].
/// `input_type`: 0 = VSF snapshot, 1 = Cartridge freeze.
/// `freezer`: 0 = Auto-detect, 1 = self-restoring (AR/SS5/FM/Expert), 2 = ISEPIC, 3 = FC3.
fn read_input_mode(input_type: i32, freezer: i32) -> InputMode {
    if input_type == 0 {
        InputMode::Vsf
    } else {
        InputMode::Freeze(match freezer {
            1 => FreezeMethod::SelfRestoring,
            2 => FreezeMethod::Isepic,
            3 => FreezeMethod::Fc3,
            _ => FreezeMethod::Auto,
        })
    }
}

fn main() {
    let app = app::App::default().with_scheme(app::Scheme::Oxy);

    let icon_svg = r##"<svg width="256" height="256" viewBox="0 0 24 24" fill="none"
     stroke="#000000" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"
     xmlns="http://www.w3.org/2000/svg">
  <!-- rounded tile (breadbin grey-beige, RAL 1019 approx #A48F7A) -->
  <rect x="2.2" y="2.2" width="19.6" height="19.6" rx="3.2"
        fill="#A48F7A" stroke="#000000"/>
  <!-- chip body (slightly lower) -->
  <rect x="5.8" y="8.3" width="7.4" height="7.4" rx="1.2" fill="#000000"/>
  <!-- chip pins (left) -->
  <line x1="5.8" y1="9.2" x2="4.4" y2="9.2"/>
  <line x1="5.8" y1="10.8" x2="4.4" y2="10.8"/>
  <line x1="5.8" y1="12.4" x2="4.4" y2="12.4"/>
  <line x1="5.8" y1="14.0" x2="4.4" y2="14.0"/>
  <!-- chip pins (right) -->
  <line x1="13.2" y1="9.2"  x2="14.6" y2="9.2"/>
  <line x1="13.2" y1="10.8" x2="14.6" y2="10.8"/>
  <line x1="13.2" y1="12.4" x2="14.6" y2="12.4"/>
  <line x1="13.2" y1="14.0" x2="14.6" y2="14.0"/>
  <!-- play arrow (green, slightly lower) -->
  <polygon points="16.2,9.2 20.2,11.5 16.2,13.8" fill="#27C93F" stroke="none"/>
</svg>"##;

    let mut window = Window::default()
        .with_size(WINDOW_WIDTH, WINDOW_HEIGHT)
        .with_label(&format!("VICE Snapshot to PRG/CRT Converter v{}", VERSION));
    window.make_resizable(false);

    if let Ok(icon) = SvgImage::from_data(icon_svg) {
        window.set_icon(Some(icon));
    }

    let mut y_pos = MARGIN;

    let tabs = Tabs::default()
        .with_pos(MARGIN - 5, y_pos)
        .with_size(WINDOW_WIDTH - 2 * MARGIN + 10, TAB_HEIGHT);

    // ==================== PRG Tab ====================
    let prg_tab = group::Group::default()
        .with_pos(MARGIN, y_pos + 25)
        .with_size(WINDOW_WIDTH - 2 * MARGIN, TAB_HEIGHT - 30)
        .with_label("PRG Output");

    let mut prg_y = y_pos + 45;

    let mut prg_input_label = Frame::default()
        .with_pos(MARGIN, prg_y)
        .with_size(WINDOW_WIDTH - 2 * MARGIN, 25)
        .with_label("Select input file (VSF snapshot or cartridge freeze):");
    prg_input_label.set_label_size(13);
    prg_input_label.set_align(enums::Align::Left | enums::Align::Inside);

    prg_y += 30;

    let prg_input_field = Input::default()
        .with_pos(MARGIN, prg_y)
        .with_size(WINDOW_WIDTH - 2 * MARGIN - BROWSE_BTN_WIDTH - 10, FIELD_HEIGHT);

    let mut prg_input_btn = Button::default()
        .with_pos(WINDOW_WIDTH - MARGIN - BROWSE_BTN_WIDTH, prg_y)
        .with_size(BROWSE_BTN_WIDTH, FIELD_HEIGHT)
        .with_label("Browse...");

    prg_y += FIELD_HEIGHT + 20;

    let mut prg_output_label = Frame::default()
        .with_pos(MARGIN, prg_y)
        .with_size(WINDOW_WIDTH - 2 * MARGIN, 25)
        .with_label("Select output C64 PRG file:");
    prg_output_label.set_label_size(13);
    prg_output_label.set_align(enums::Align::Left | enums::Align::Inside);

    prg_y += 30;

    let prg_output_field = Input::default()
        .with_pos(MARGIN, prg_y)
        .with_size(WINDOW_WIDTH - 2 * MARGIN - BROWSE_BTN_WIDTH - 10, FIELD_HEIGHT);

    let mut prg_output_btn = Button::default()
        .with_pos(WINDOW_WIDTH - MARGIN - BROWSE_BTN_WIDTH, prg_y)
        .with_size(BROWSE_BTN_WIDTH, FIELD_HEIGHT)
        .with_label("Browse...");

    prg_tab.end();

    // ==================== CRT Tab ====================
    let crt_tab = group::Group::default()
        .with_pos(MARGIN, y_pos + 25)
        .with_size(WINDOW_WIDTH - 2 * MARGIN, TAB_HEIGHT - 30)
        .with_label("CRT Output");

    let mut crt_y = y_pos + 45;

    let mut crt_input_label = Frame::default()
        .with_pos(MARGIN, crt_y)
        .with_size(WINDOW_WIDTH - 2 * MARGIN, 25)
        .with_label("Select input file (VSF snapshot or cartridge freeze):");
    crt_input_label.set_label_size(13);
    crt_input_label.set_align(enums::Align::Left | enums::Align::Inside);

    crt_y += 30;

    let crt_input_field = Input::default()
        .with_pos(MARGIN, crt_y)
        .with_size(WINDOW_WIDTH - 2 * MARGIN - BROWSE_BTN_WIDTH - 10, FIELD_HEIGHT);

    let mut crt_input_btn = Button::default()
        .with_pos(WINDOW_WIDTH - MARGIN - BROWSE_BTN_WIDTH, crt_y)
        .with_size(BROWSE_BTN_WIDTH, FIELD_HEIGHT)
        .with_label("Browse...");

    crt_y += FIELD_HEIGHT + 20;

    let mut crt_output_label = Frame::default()
        .with_pos(MARGIN, crt_y)
        .with_size(WINDOW_WIDTH - 2 * MARGIN, 25)
        .with_label("Select output CRT file:");
    crt_output_label.set_label_size(13);
    crt_output_label.set_align(enums::Align::Left | enums::Align::Inside);

    crt_y += 30;

    let crt_output_field = Input::default()
        .with_pos(MARGIN, crt_y)
        .with_size(WINDOW_WIDTH - 2 * MARGIN - BROWSE_BTN_WIDTH - 10, FIELD_HEIGHT);

    let mut crt_output_btn = Button::default()
        .with_pos(WINDOW_WIDTH - MARGIN - BROWSE_BTN_WIDTH, crt_y)
        .with_size(BROWSE_BTN_WIDTH, FIELD_HEIGHT)
        .with_label("Browse...");

    crt_y += FIELD_HEIGHT + 20;

    // Cartridge type selection
    let mut crt_type_label = Frame::default()
        .with_pos(MARGIN, crt_y)
        .with_size(120, 25)
        .with_label("Cartridge type:");
    crt_type_label.set_label_size(13);
    crt_type_label.set_align(enums::Align::Left | enums::Align::Inside);

    let mut crt_type_choice = menu::Choice::default()
        .with_pos(MARGIN + 125, crt_y)
        .with_size(160, 25);
    crt_type_choice.add_choice("EasyFlash|Magic Desk");
    crt_type_choice.set_value(0); // Default: EasyFlash

    crt_y += 35;

    // Cartridge name
    let mut crt_name_label = Frame::default()
        .with_pos(MARGIN, crt_y)
        .with_size(WINDOW_WIDTH - 2 * MARGIN, 25)
        .with_label("Cartridge name (max 32 characters):");
    crt_name_label.set_label_size(13);
    crt_name_label.set_align(enums::Align::Left | enums::Align::Inside);

    crt_y += 30;

    let crt_name_field = Input::default()
        .with_pos(MARGIN, crt_y)
        .with_size(WINDOW_WIDTH - 2 * MARGIN, FIELD_HEIGHT);

    crt_y += FIELD_HEIGHT + 15;

    // LOAD/SAVE hooking checkbox
    let crt_hook_check = CheckButton::default()
        .with_pos(MARGIN, crt_y)
        .with_size(300, 25)
        .with_label("Enable LOAD/SAVE hooking");

    crt_y += 30;

    // Auto location checkbox (default: checked, but initially disabled)
    let mut crt_auto_location_check = CheckButton::default()
        .with_pos(MARGIN + 20, crt_y)
        .with_size(250, 25)
        .with_label("Auto location (based on SP)");
    crt_auto_location_check.set_checked(true);
    crt_auto_location_check.deactivate(); // Disabled until hook is enabled

    // Manual address field (initially disabled)
    let mut crt_addr_label = Frame::default()
        .with_pos(MARGIN + 280, crt_y)
        .with_size(120, 25)
        .with_label("Start address:");
    crt_addr_label.set_label_size(12);
    crt_addr_label.set_align(enums::Align::Left | enums::Align::Inside);

    let mut crt_addr_field = Input::default()
        .with_pos(MARGIN + 400, crt_y)
        .with_size(80, 25);
    crt_addr_field.set_value("$0100");
    crt_addr_field.deactivate(); // Disabled until hook is enabled and auto is off

    crt_y += 35;

    // Include directory for PRG files (initially disabled)
    let mut crt_include_label = Frame::default()
        .with_pos(MARGIN, crt_y)
        .with_size(WINDOW_WIDTH - 2 * MARGIN, 25)
        .with_label("Include directory (PRG files to embed):");
    crt_include_label.set_label_size(13);
    crt_include_label.set_align(enums::Align::Left | enums::Align::Inside);

    crt_y += 30;

    let mut crt_include_field = Input::default()
        .with_pos(MARGIN, crt_y)
        .with_size(WINDOW_WIDTH - 2 * MARGIN - BROWSE_BTN_WIDTH - 10, FIELD_HEIGHT);
    crt_include_field.deactivate(); // Disabled until hook is enabled

    let mut crt_include_btn = Button::default()
        .with_pos(WINDOW_WIDTH - MARGIN - BROWSE_BTN_WIDTH, crt_y)
        .with_size(BROWSE_BTN_WIDTH, FIELD_HEIGHT)
        .with_label("Browse...");
    crt_include_btn.deactivate(); // Disabled until hook is enabled

    crt_tab.end();
    tabs.end();

    y_pos += TAB_HEIGHT + 10;

    // ==================== Input file type (shared by PRG and CRT) ====================
    let mut input_type_label = Frame::default()
        .with_pos(MARGIN, y_pos)
        .with_size(105, 25)
        .with_label("Input file type:");
    input_type_label.set_label_size(13);
    input_type_label.set_align(enums::Align::Left | enums::Align::Inside);

    let mut input_type_choice = menu::Choice::default()
        .with_pos(MARGIN + 110, y_pos)
        .with_size(160, 25);
    input_type_choice.add_choice("VSF snapshot|Cartridge freeze");
    input_type_choice.set_value(0); // Default: VSF snapshot

    let mut freezer_label = Frame::default()
        .with_pos(MARGIN + 285, y_pos)
        .with_size(95, 25)
        .with_label("Force freezer:");
    freezer_label.set_label_size(13);
    freezer_label.set_align(enums::Align::Left | enums::Align::Inside);

    let mut freezer_choice = menu::Choice::default()
        .with_pos(MARGIN + 380, y_pos)
        .with_size(WINDOW_WIDTH - (MARGIN + 380) - MARGIN, 25);
    // FLTK treats '/' as a submenu separator, so a label with '/' becomes a
    // nested submenu. Commas keep this one flat menu item.
    freezer_choice.add_choice("Auto-detect|Self-restoring (AR, SS5, FM, Expert)|ISEPIC|Final Cartridge III");
    freezer_choice.set_value(0); // Default: Auto-detect
    freezer_choice.deactivate(); // Enabled only for "Cartridge freeze"

    y_pos += 45;

    // Shared option (all output formats): zero RAM regions still holding the
    // C64 power-on pattern so they become usable free blocks. Highly
    // experimental and off by default; confirmed via a dialog when enabled.
    let clear_poweron_check = CheckButton::default()
        .with_pos(MARGIN, y_pos)
        .with_size(WINDOW_WIDTH - 2 * MARGIN, 25)
        .with_label("Clear power-on RAM pattern (HIGHLY EXPERIMENTAL, may lose data)");
    clear_poweron_check.set_checked(false);

    y_pos += 30;

    // Shared option (all output formats): compression format for the snapshot blocks.
    let mut format_label = Frame::default()
        .with_pos(MARGIN, y_pos)
        .with_size(105, 25)
        .with_label("Compression:");
    format_label.set_label_size(13);
    format_label.set_align(enums::Align::Left | enums::Align::Inside);

    let mut format_choice = menu::Choice::default()
        .with_pos(MARGIN + 110, y_pos)
        .with_size(300, 25);
    // Labels in PackFormat::all() order; the selected index maps back to that array.
    let format_labels: Vec<&str> = PackFormat::all().iter().map(|f| f.label()).collect();
    format_choice.add_choice(&format_labels.join("|"));
    format_choice.set_value(0); // Default: LZSA1

    y_pos += 45;

    // Status display (shared)
    let mut status_label = Frame::default()
        .with_pos(MARGIN, y_pos)
        .with_size(WINDOW_WIDTH - 2 * MARGIN, 25)
        .with_label("Status:");
    status_label.set_label_size(13);
    status_label.set_align(enums::Align::Left | enums::Align::Inside);

    y_pos += 30;

    let status_height = WINDOW_HEIGHT - y_pos - BUTTON_HEIGHT - 30;

    let status_buffer = TextBuffer::default();
    let mut status_display = TextDisplay::default()
        .with_pos(MARGIN, y_pos)
        .with_size(WINDOW_WIDTH - 2 * MARGIN, status_height);
    status_display.set_buffer(status_buffer.clone());
    status_display.wrap_mode(text::WrapMode::AtBounds, 0);
    status_display.set_frame(FrameType::DownBox);

    let button_y = WINDOW_HEIGHT - BUTTON_HEIGHT - 20;
    let button_spacing = 10;
    let total_button_width = 3 * BUTTON_WIDTH + 2 * button_spacing;
    let start_x = (WINDOW_WIDTH - total_button_width) / 2;

    let convert_x = start_x;
    let help_x = start_x + BUTTON_WIDTH + button_spacing;
    let quit_x = start_x + 2 * BUTTON_WIDTH + 2 * button_spacing;

    let mut convert_btn = Button::default()
        .with_pos(convert_x, button_y)
        .with_size(BUTTON_WIDTH, BUTTON_HEIGHT)
        .with_label("Convert");
    convert_btn.set_color(Color::from_rgb(70, 130, 180));
    convert_btn.set_label_color(Color::White);

    let mut help_btn = Button::default()
        .with_pos(help_x, button_y)
        .with_size(BUTTON_WIDTH, BUTTON_HEIGHT)
        .with_label("Help");

    let mut quit_btn = Button::default()
        .with_pos(quit_x, button_y)
        .with_size(BUTTON_WIDTH, BUTTON_HEIGHT)
        .with_label("Quit");

    window.end();
    window.show();

    // Shared state
    let prg_input_field_rc = Rc::new(RefCell::new(prg_input_field.clone()));
    let prg_output_field_rc = Rc::new(RefCell::new(prg_output_field.clone()));
    let crt_input_field_rc = Rc::new(RefCell::new(crt_input_field.clone()));
    let crt_output_field_rc = Rc::new(RefCell::new(crt_output_field.clone()));
    let crt_name_field_rc = Rc::new(RefCell::new(crt_name_field.clone()));
    let crt_type_choice_rc = Rc::new(RefCell::new(crt_type_choice.clone()));
    let crt_hook_check_rc = Rc::new(RefCell::new(crt_hook_check.clone()));
    let crt_auto_location_check_rc = Rc::new(RefCell::new(crt_auto_location_check.clone()));
    let crt_addr_field_rc = Rc::new(RefCell::new(crt_addr_field.clone()));
    let crt_include_field_rc = Rc::new(RefCell::new(crt_include_field.clone()));
    let crt_include_btn_rc = Rc::new(RefCell::new(crt_include_btn.clone()));
    let clear_poweron_check_rc = Rc::new(RefCell::new(clear_poweron_check.clone()));
    let status_buffer_rc = Rc::new(RefCell::new(status_buffer));
    let tabs_rc = Rc::new(RefCell::new(tabs.clone()));
    let input_type_choice_rc = Rc::new(RefCell::new(input_type_choice.clone()));
    let freezer_choice_rc = Rc::new(RefCell::new(freezer_choice.clone()));
    let format_choice_rc = Rc::new(RefCell::new(format_choice.clone()));

    // Confirm before enabling the experimental power-on RAM clearing pass;
    // decline leaves it unchecked.
    {
        let check = clear_poweron_check_rc.clone();
        clear_poweron_check.clone().set_callback(move |c| {
            if c.is_checked() {
                let choice = dialog::choice2_default(
                    "Clear power-on RAM pattern is HIGHLY EXPERIMENTAL.\n\n\
                     It zeroes RAM regions it believes are untouched power-on \
                     RAM. A misdetection could destroy program data in the \
                     converted output.\n\nAre you sure you want to enable it?",
                    "Cancel",
                    "Enable",
                    "",
                );
                if choice != Some(1) {
                    check.borrow_mut().set_checked(false);
                }
            }
        });
    }

    // Extra RAM blocks for allocation failures (shared between PRG and CRT)
    // Each block is (address, count); cleared on snapshot change or tab switch
    let extra_ram_blocks_rc: Rc<RefCell<Vec<(u16, u16)>>> = Rc::new(RefCell::new(Vec::new()));

    // Input-type callback: the "Force freezer" override is only meaningful when
    // converting a cartridge freeze, so enable it only for that choice.
    {
        let freezer = freezer_choice_rc.clone();
        input_type_choice.clone().set_callback(move |c| {
            if c.value() == 1 {
                freezer.borrow_mut().activate();
            } else {
                freezer.borrow_mut().deactivate();
            }
        });
    }

    // CRT cartridge type callback
    //
    // EasyFlash and Magic Desk both support LOAD/SAVE hooking with embedded PRG
    // files and use the same trampoline placement (auto from the snapshot's
    // stack pointer, or a manual address). The hook controls follow the hook
    // checkbox for both types.
    {
        let hook_check = crt_hook_check_rc.clone();
        let auto_location_check = crt_auto_location_check_rc.clone();
        let addr_field = crt_addr_field_rc.clone();

        crt_type_choice.clone().set_callback(move |_choice| {
            hook_check.borrow_mut().activate();
            if hook_check.borrow().is_checked() {
                auto_location_check.borrow_mut().activate();
                if auto_location_check.borrow().is_checked() {
                    addr_field.borrow_mut().deactivate();
                } else {
                    addr_field.borrow_mut().activate();
                }
            } else {
                auto_location_check.borrow_mut().deactivate();
                addr_field.borrow_mut().deactivate();
            }
        });
    }

    // CRT hook checkbox callback: enable or disable related fields
    {
        let auto_location_check = crt_auto_location_check_rc.clone();
        let addr_field = crt_addr_field_rc.clone();
        let include_field = crt_include_field_rc.clone();
        let include_btn = crt_include_btn_rc.clone();

        crt_hook_check.clone().set_callback(move |check| {
            if check.is_checked() {
                // Enable all related fields
                auto_location_check.borrow_mut().activate();
                include_field.borrow_mut().activate();
                include_btn.borrow_mut().activate();
                if !auto_location_check.borrow().is_checked() {
                    addr_field.borrow_mut().activate();
                }
            } else {
                // Disable all related fields
                auto_location_check.borrow_mut().deactivate();
                addr_field.borrow_mut().deactivate();
                include_field.borrow_mut().deactivate();
                include_btn.borrow_mut().deactivate();
            }
        });
    }

    // CRT auto location checkbox callback: enable or disable the address field
    {
        let addr_field = crt_addr_field_rc.clone();
        let hook_check = crt_hook_check_rc.clone();

        crt_auto_location_check.clone().set_callback(move |check| {
            if hook_check.borrow().is_checked() {
                if check.is_checked() {
                    addr_field.borrow_mut().deactivate();
                } else {
                    addr_field.borrow_mut().activate();
                }
            }
        });
    }

    // CRT address field: format and validate on change
    {
        let addr_field = crt_addr_field_rc.clone();

        crt_addr_field.clone().handle(move |_, ev| {
            if ev == enums::Event::Unfocus {
                let mut field = addr_field.borrow_mut();
                let text = field.value();
                let cleaned = text.trim()
                    .trim_start_matches('$')
                    .trim_start_matches("0x")
                    .trim_start_matches("0X");

                if !cleaned.is_empty() {
                    if let Ok(mut addr) = u16::from_str_radix(cleaned, 16) {
                        // Clamp to valid range: $0100 - $FF00
                        if addr < 0x0100 {
                            addr = 0x0100;
                        }
                        if addr > 0xFF00 {
                            addr = 0xFF00;
                        }
                        field.set_value(&format!("${:04X}", addr));
                    }
                }
            }
            false
        });
    }

    // PRG input browse
    {
        let input_field = prg_input_field_rc.clone();
        let output_field = prg_output_field_rc.clone();
        let extra_blocks = extra_ram_blocks_rc.clone();
        let itc = input_type_choice_rc.clone();

        prg_input_btn.set_callback(move |_| {
            let mut chooser = NativeFileChooser::new(dialog::NativeFileChooserType::BrowseFile);
            chooser.set_title("Select input file");
            chooser.set_filter(input_browse_filter(itc.borrow().value()));

            let current = input_field.borrow().value();
            if !current.is_empty() {
                if let Some(parent) = Path::new(&current).parent() {
                    let _ = chooser.set_directory(&parent.to_path_buf());
                }
            }

            chooser.show();
            let filename = chooser.filename();

            if !filename.as_os_str().is_empty() {
                let path_str = filename.to_string_lossy().to_string();
                input_field.borrow_mut().set_value(&path_str);

                // Clear extra RAM blocks when snapshot changes
                extra_blocks.borrow_mut().clear();

                // Default output = input stem + "_vs" + .prg, so the source is
                // never overwritten by the suggested filename.
                let suggested_output = suggested_output_path(&filename, "prg");
                output_field.borrow_mut().set_value(&suggested_output.to_string_lossy());
            }
        });
    }

    // PRG output browse
    {
        let input_field = prg_input_field_rc.clone();
        let output_field = prg_output_field_rc.clone();

        prg_output_btn.set_callback(move |_| {
            let mut chooser = NativeFileChooser::new(dialog::NativeFileChooserType::BrowseSaveFile);
            chooser.set_title("Save PRG File As");
            chooser.set_filter("PRG Files\t*.prg\nAll Files\t*");
            chooser.set_option(dialog::FileDialogOptions::SaveAsConfirm);

            let input_path = input_field.borrow().value();
            if !input_path.is_empty() {
                let input = Path::new(&input_path);
                if let Some(parent) = input.parent() {
                    let _ = chooser.set_directory(&parent.to_path_buf());
                }
                let preset = suggested_output_path(input, "prg");
                if let Some(name) = preset.file_name() {
                    chooser.set_preset_file(&name.to_string_lossy());
                }
            }

            chooser.show();
            let filename = chooser.filename();

            if !filename.as_os_str().is_empty() {
                output_field.borrow_mut().set_value(&filename.to_string_lossy());
            }
        });
    }

    // CRT input browse
    {
        let input_field = crt_input_field_rc.clone();
        let output_field = crt_output_field_rc.clone();
        let extra_blocks = extra_ram_blocks_rc.clone();
        let itc = input_type_choice_rc.clone();

        crt_input_btn.set_callback(move |_| {
            let mut chooser = NativeFileChooser::new(dialog::NativeFileChooserType::BrowseFile);
            chooser.set_title("Select input file");
            chooser.set_filter(input_browse_filter(itc.borrow().value()));

            let current = input_field.borrow().value();
            if !current.is_empty() {
                if let Some(parent) = Path::new(&current).parent() {
                    let _ = chooser.set_directory(&parent.to_path_buf());
                }
            }

            chooser.show();
            let filename = chooser.filename();

            if !filename.as_os_str().is_empty() {
                let path_str = filename.to_string_lossy().to_string();
                input_field.borrow_mut().set_value(&path_str);

                // Clear extra RAM blocks when snapshot changes
                extra_blocks.borrow_mut().clear();

                // Default output = input stem + "_vs" + .crt, so the source is
                // never overwritten by the suggested filename.
                let suggested_output = suggested_output_path(&filename, "crt");
                output_field.borrow_mut().set_value(&suggested_output.to_string_lossy());
            }
        });
    }

    // CRT output browse
    {
        let input_field = crt_input_field_rc.clone();
        let output_field = crt_output_field_rc.clone();

        crt_output_btn.set_callback(move |_| {
            let mut chooser = NativeFileChooser::new(dialog::NativeFileChooserType::BrowseSaveFile);
            chooser.set_title("Save CRT File As");
            chooser.set_filter("CRT Files\t*.crt\nAll Files\t*");
            chooser.set_option(dialog::FileDialogOptions::SaveAsConfirm);

            let input_path = input_field.borrow().value();
            if !input_path.is_empty() {
                let input = Path::new(&input_path);
                if let Some(parent) = input.parent() {
                    let _ = chooser.set_directory(&parent.to_path_buf());
                }
                let preset = suggested_output_path(input, "crt");
                if let Some(name) = preset.file_name() {
                    chooser.set_preset_file(&name.to_string_lossy());
                }
            }

            chooser.show();
            let filename = chooser.filename();

            if !filename.as_os_str().is_empty() {
                output_field.borrow_mut().set_value(&filename.to_string_lossy());
            }
        });
    }

    // CRT include directory browse
    {
        let include_field = crt_include_field_rc.clone();

        crt_include_btn.set_callback(move |_| {
            let mut chooser = NativeFileChooser::new(dialog::NativeFileChooserType::BrowseDir);
            chooser.set_title("Select Directory with PRG Files");

            chooser.show();
            let filename = chooser.filename();

            if !filename.as_os_str().is_empty() {
                include_field.borrow_mut().set_value(&filename.to_string_lossy());
            }
        });
    }

    help_btn.set_callback(|_| {
        show_help_window();
    });

    // Convert button
    {
        let prg_input = prg_input_field_rc.clone();
        let prg_output = prg_output_field_rc.clone();
        let crt_input = crt_input_field_rc.clone();
        let crt_output = crt_output_field_rc.clone();
        let crt_name = crt_name_field_rc.clone();
        let crt_type = crt_type_choice_rc.clone();
        let crt_hook = crt_hook_check_rc.clone();
        let crt_auto_location = crt_auto_location_check_rc.clone();
        let crt_addr = crt_addr_field_rc.clone();
        let crt_include = crt_include_field_rc.clone();
        let clear_poweron = clear_poweron_check_rc.clone();
        let status_buffer = status_buffer_rc.clone();
        let tabs = tabs_rc.clone();
        let extra_blocks = extra_ram_blocks_rc.clone();
        let input_type = input_type_choice_rc.clone();
        let freezer = freezer_choice_rc.clone();
        let format_sel = format_choice_rc.clone();

        convert_btn.set_callback(move |btn| {
            // Read the active tab and release the borrow: the event loop keeps running while a
            // conversion is in flight, so no RefCell may stay borrowed across it.
            let active_tab = {
                let tabs_val = tabs.borrow();
                tabs_val.value().map(|w| w.label()).unwrap_or_default()
            };
            let is_crt = active_tab.contains("CRT");

            // How to interpret the input file (VSF vs cartridge freeze + forced method).
            let input_mode = read_input_mode(input_type.borrow().value(), freezer.borrow().value());

            // Selected compression format (index maps to PackFormat::all()).
            let all_formats = PackFormat::all();
            let pack_format = all_formats
                .get(format_sel.borrow().value().max(0) as usize)
                .copied()
                .unwrap_or_default();

            status_buffer.borrow_mut().set_text("");

            if is_crt {
                // CRT conversion
                let input_path = crt_input.borrow().value();
                let output_path = crt_output.borrow().value();
                let cart_name = crt_name.borrow().value();
                let is_magic_desk = crt_type.borrow().value() == 1;
                let hook_enabled = crt_hook.borrow().is_checked();
                let auto_location = crt_auto_location.borrow().is_checked();
                let addr_text = crt_addr.borrow().value();
                let include_dir = crt_include.borrow().value();
                let clear_poweron_ram = clear_poweron.borrow().is_checked();
                let cart_type_name = if is_magic_desk { "Magic Desk" } else { "EasyFlash" };

                if input_path.is_empty() {
                    status_buffer.borrow_mut().set_text("Error: Please select an input VSF file");
                    return;
                }

                if output_path.is_empty() {
                    status_buffer.borrow_mut().set_text("Error: Please specify an output CRT filename");
                    return;
                }

                if !Path::new(&input_path).exists() {
                    let msg = format!("Error: Input file not found:\n{}", input_path);
                    status_buffer.borrow_mut().set_text(&msg);
                    return;
                }

                // Never overwrite the source: refuse when output == input.
                if paths_refer_to_same_file(&input_path, &output_path) {
                    status_buffer.borrow_mut().set_text(
                        "Error: The output file is the same as the input file.\n\nChoose a different output filename so the source is not overwritten.",
                    );
                    return;
                }

                // Validate include directory when hook is enabled
                if hook_enabled && include_dir.is_empty() {
                    status_buffer.borrow_mut().set_text("Error: Include directory is required when LOAD/SAVE hooking is enabled.\n\nPlease select a directory containing PRG files to embed.");
                    return;
                }

                if hook_enabled && !include_dir.is_empty() && !Path::new(&include_dir).is_dir() {
                    let msg = format!("Error: Include directory not found:\n{}", include_dir);
                    status_buffer.borrow_mut().set_text(&msg);
                    return;
                }

                if Path::new(&output_path).exists() {
                    let choice = dialog::choice2_default(
                        &format!("The output file already exists:\n\n{}\n\nDo you want to overwrite it?", output_path),
                        "Cancel",
                        "Overwrite",
                        ""
                    );

                    if choice != Some(1) {
                        status_buffer.borrow_mut().set_text("Conversion cancelled by user.");
                        return;
                    }

                    if let Err(e) = std::fs::remove_file(&output_path) {
                        let msg = format!("Error: Failed to delete existing file:\n{}", e);
                        status_buffer.borrow_mut().set_text(&msg);
                        return;
                    }
                }

                btn.deactivate();

                // Conversion loop with retry on allocation failure
                loop {
                    let current_blocks = extra_blocks.borrow().clone();
                    let blocks_count = current_blocks.len();

                    if blocks_count > 0 {
                        status_buffer.borrow_mut().set_text(&format!(
                            "Converting snapshot to {} CRT...\nUsing {} extra RAM block(s)\n",
                            cart_type_name, blocks_count
                        ));
                    } else {
                        status_buffer.borrow_mut().set_text(&format!(
                            "Converting snapshot to {} CRT...\n", cart_type_name
                        ));
                    }
                    app::awake();

                    let progress = Progress::new();
                    let result = {
                        let progress_for_job = progress.clone();
                        let (input_path, output_path) = (input_path.clone(), output_path.clone());
                        let (cart_name, include_dir, addr_text) =
                            (cart_name.clone(), include_dir.clone(), addr_text.clone());
                        run_with_progress(
                            &format!("Converting to {} CRT", cart_type_name),
                            &progress,
                            move || {
                                let mut config = CrtConfig::auto().map_err(|e| e.to_string())?;
                                config.base_config.input_mode = input_mode;
                                config.base_config.clear_poweron_ram = clear_poweron_ram;
                                config.base_config.pack_format = pack_format;
                                config.base_config.progress = progress_for_job;
                                if !cart_name.is_empty() {
                                    config.cartridge_name = Some(cart_name);
                                }
                                if hook_enabled && !include_dir.is_empty() {
                                    config.include_dir = Some(include_dir);
                                    config.patch_load_save = true;
                                    config.auto_location = auto_location;

                                    // Parse manual trampoline address if not using auto location
                                    if !auto_location && !addr_text.is_empty() {
                                        let cleaned = addr_text.trim()
                                            .trim_start_matches('$')
                                            .trim_start_matches("0x")
                                            .trim_start_matches("0X");
                                        if let Ok(addr) = u16::from_str_radix(cleaned, 16) {
                                            if addr >= 0x0100 {
                                                config.trampoline_address = Some(addr);
                                            }
                                        }
                                    }
                                }

                                let work_path = config.base_config.work_path.clone();
                                let outcome = {
                                    let _temp = WorkDirGuard::new(work_path.clone());
                                    if is_magic_desk {
                                        let converter = ConvertSnapshotMagicDeskCRT::with_extra_blocks(config, current_blocks);
                                        converter.convert(&input_path, &output_path)
                                            .map(|_| converter.poweron_cleared())
                                    } else {
                                        let converter = ConvertSnapshotCRT::with_extra_blocks(config, current_blocks);
                                        converter.convert(&input_path, &output_path)
                                            .map(|_| converter.poweron_cleared())
                                    }
                                };
                                finish_conversion(outcome, &work_path, &output_path)
                            },
                        )
                    };

                    match result {
                        Ok(poweron_cleared) => {
                            extra_blocks.borrow_mut().clear();
                            let mut success_msg = format!(
                                "Success!\n\nSnapshot successfully converted to {} CRT:\n{}",
                                cart_type_name, output_path
                            );
                            if clear_poweron_ram {
                                success_msg.push_str(&format!(
                                    "\n\nPower-on RAM pattern cleared: {} bytes",
                                    poweron_cleared
                                ));
                            } else {
                                success_msg.push_str("\n\nPower-on RAM pattern clearing: off");
                            }
                            status_buffer.borrow_mut().set_text(&success_msg);
                            break;
                        }
                        Err(e) if is_cancelled_error(&e) => {
                            status_buffer.borrow_mut().set_text(
                                "Conversion cancelled.\n\nNo converted file was produced, \
                                 and the temporary files have been removed.",
                            );
                            break;
                        }
                        Err(e) => {
                            if is_allocation_error(&e) {
                                // Allocation failure: offer to add a RAM block
                                status_buffer.borrow_mut().set_text(&format!("Conversion failed:\n\n{}", e));

                                let choice = dialog::choice2_default(
                                    &format!("{}\n\nWould you like to add a free RAM block manually?", e),
                                    "No",
                                    "Yes",
                                    ""
                                );

                                if choice == Some(1) {
                                    if let Some((addr, count)) = show_add_ram_block_dialog() {
                                        extra_blocks.borrow_mut().push((addr, count));
                                        let end_addr = addr + count - 1;
                                        let mut buf = status_buffer.borrow_mut();
                                        buf.append(&format!(
                                            "\nAdded extra RAM block: ${:04X}-${:04X} ({} bytes)\n",
                                            addr, end_addr, count
                                        ));
                                        buf.append("Retrying conversion...\n\n");
                                        continue;
                                    }
                                }
                                // User cancelled or didn't add block
                                break;
                            } else {
                                // Other error: no retry
                                let error_msg = format!("Conversion failed:\n\n{}", e);
                                status_buffer.borrow_mut().set_text(&error_msg);
                                break;
                            }
                        }
                    }
                }

                btn.activate();
            } else {
                // PRG conversion
                let input_path = prg_input.borrow().value();
                let output_path = prg_output.borrow().value();

                if input_path.is_empty() {
                    status_buffer.borrow_mut().set_text("Error: Please select an input VSF file");
                    return;
                }

                if output_path.is_empty() {
                    status_buffer.borrow_mut().set_text("Error: Please specify an output PRG filename");
                    return;
                }

                if !Path::new(&input_path).exists() {
                    let msg = format!("Error: Input file not found:\n{}", input_path);
                    status_buffer.borrow_mut().set_text(&msg);
                    return;
                }

                // Never overwrite the source: refuse when output == input.
                if paths_refer_to_same_file(&input_path, &output_path) {
                    status_buffer.borrow_mut().set_text(
                        "Error: The output file is the same as the input file.\n\nChoose a different output filename so the source is not overwritten.",
                    );
                    return;
                }

                if Path::new(&output_path).exists() {
                    let choice = dialog::choice2_default(
                        &format!("The output file already exists:\n\n{}\n\nDo you want to overwrite it?", output_path),
                        "Cancel",
                        "Overwrite",
                        ""
                    );

                    if choice != Some(1) {
                        status_buffer.borrow_mut().set_text("Conversion cancelled by user.");
                        return;
                    }

                    if let Err(e) = std::fs::remove_file(&output_path) {
                        let msg = format!("Error: Failed to delete existing file:\n{}", e);
                        status_buffer.borrow_mut().set_text(&msg);
                        return;
                    }
                }

                btn.deactivate();

                // Conversion loop with retry on allocation failure
                loop {
                    let current_blocks = extra_blocks.borrow().clone();
                    let blocks_count = current_blocks.len();

                    if blocks_count > 0 {
                        status_buffer.borrow_mut().set_text(&format!(
                            "Converting snapshot image...\nUsing {} extra RAM block(s)\n",
                            blocks_count
                        ));
                    } else {
                        status_buffer.borrow_mut().set_text("Converting snapshot image...\n");
                    }
                    app::awake();

                    let clear_poweron_ram = clear_poweron.borrow().is_checked();

                    let progress = Progress::new();
                    let result = {
                        let progress_for_job = progress.clone();
                        let (input_path, output_path) = (input_path.clone(), output_path.clone());
                        run_with_progress("Converting snapshot", &progress, move || {
                            let mut config = Config::auto().map_err(|e| {
                                format!("Failed to initialize configuration: {}", e)
                            })?;
                            config.input_mode = input_mode;
                            config.clear_poweron_ram = clear_poweron_ram;
                            config.pack_format = pack_format;
                            config.progress = progress_for_job;
                            let work_path = config.work_path.clone();

                            let outcome = {
                                let _temp = WorkDirGuard::new(work_path.clone());
                                let converter = ConvertSnapshot::with_extra_blocks(config, current_blocks);
                                converter.convert(&input_path, &output_path)
                                    .map(|_| converter.poweron_cleared())
                            };
                            finish_conversion(outcome, &work_path, &output_path)
                        })
                    };

                    match result {
                        Ok(poweron_cleared) => {
                            extra_blocks.borrow_mut().clear();
                            let mut success_msg = format!(
                                "Success!\n\nSnapshot image successfully converted to:\n{}",
                                output_path
                            );
                            if clear_poweron_ram {
                                success_msg.push_str(&format!(
                                    "\n\nPower-on RAM pattern cleared: {} bytes",
                                    poweron_cleared
                                ));
                            } else {
                                success_msg.push_str("\n\nPower-on RAM pattern clearing: off");
                            }
                            status_buffer.borrow_mut().set_text(&success_msg);
                            break;
                        }
                        Err(e) if is_cancelled_error(&e) => {
                            status_buffer.borrow_mut().set_text(
                                "Conversion cancelled.\n\nNo converted file was produced, \
                                 and the temporary files have been removed.",
                            );
                            break;
                        }
                        Err(e) => {
                            if is_allocation_error(&e) {
                                // Allocation failure: offer to add a RAM block
                                status_buffer.borrow_mut().set_text(&format!("Conversion failed:\n\n{}", e));

                                let choice = dialog::choice2_default(
                                    &format!("{}\n\nWould you like to add a free RAM block manually?", e),
                                    "No",
                                    "Yes",
                                    ""
                                );

                                if choice == Some(1) {
                                    if let Some((addr, count)) = show_add_ram_block_dialog() {
                                        extra_blocks.borrow_mut().push((addr, count));
                                        let end_addr = addr + count - 1;
                                        let mut buf = status_buffer.borrow_mut();
                                        buf.append(&format!(
                                            "\nAdded extra RAM block: ${:04X}-${:04X} ({} bytes)\n",
                                            addr, end_addr, count
                                        ));
                                        buf.append("Retrying conversion...\n\n");
                                        continue;
                                    }
                                }
                                // User cancelled or didn't add block
                                break;
                            } else {
                                // Other error: no retry
                                let error_msg = format!("Conversion failed:\n\n{}", e);
                                status_buffer.borrow_mut().set_text(&error_msg);
                                break;
                            }
                        }
                    }
                }

                btn.activate();
            }
        });
    }

    // `app::quit()` only hides the windows. A running conversion polls the quit flag, so set
    // it first to let the conversion stop and clean up.
    quit_btn.set_callback(|_| {
        app::program_should_quit(true);
        app::quit();
    });

    window.set_callback(|_| {
        if app::event() == enums::Event::Close {
            app::program_should_quit(true);
            app::quit();
        }
    });

    // Clear extra RAM blocks when tab changes
    {
        let extra_blocks = extra_ram_blocks_rc.clone();
        tabs.clone().set_callback(move |_| {
            extra_blocks.borrow_mut().clear();
        });
    }

    app.run().unwrap();
}

/// Parse hex address string with or without $ prefix
/// Returns None if invalid or out of range ($0100-$FFFF)
fn parse_hex_address(text: &str) -> Option<u16> {
    let cleaned = text.trim()
        .trim_start_matches('$')
        .trim_start_matches("0x")
        .trim_start_matches("0X");
    if cleaned.is_empty() {
        return None;
    }
    match u16::from_str_radix(cleaned, 16) {
        Ok(value) if value >= 0x0100 => Some(value),
        _ => None,
    }
}

/// Show dialog to add a free RAM block manually
/// Returns Some((address, count)) if user provided valid input, None if cancelled
fn show_add_ram_block_dialog() -> Option<(u16, u16)> {
    let dialog_width = 450;
    let dialog_height = 140;

    let mut dialog = Window::default()
        .with_size(dialog_width, dialog_height)
        .with_label("Add Free RAM Block");
    dialog.make_modal(true);
    dialog.set_pos(
        (app::screen_size().0 as i32 - dialog_width) / 2,
        (app::screen_size().1 as i32 - dialog_height) / 2,
    );

    // Header text
    let mut header = Frame::default()
        .with_pos(15, 15)
        .with_size(dialog_width - 30, 25)
        .with_label("Specify address range for free RAM block:");
    header.set_label_size(13);
    header.set_align(Align::Left | Align::Inside);

    // Row with "Add free ram block from $" [field] "to $" [field]
    let mut from_label = Frame::default()
        .with_pos(15, 50)
        .with_size(170, 25)
        .with_label("Add free RAM block from $");
    from_label.set_label_size(12);
    from_label.set_align(Align::Left | Align::Inside);

    let from_field = Input::default()
        .with_pos(185, 50)
        .with_size(70, 25);

    let mut to_label = Frame::default()
        .with_pos(265, 50)
        .with_size(40, 25)
        .with_label("to $");
    to_label.set_label_size(12);
    to_label.set_align(Align::Left | Align::Inside);

    let to_field = Input::default()
        .with_pos(305, 50)
        .with_size(70, 25);

    // Buttons
    let mut ok_btn = Button::default()
        .with_pos(dialog_width / 2 - 110, dialog_height - 45)
        .with_size(100, 35)
        .with_label("OK");
    ok_btn.set_color(Color::from_rgb(70, 130, 180));
    ok_btn.set_label_color(Color::White);

    let mut cancel_btn = Button::default()
        .with_pos(dialog_width / 2 + 10, dialog_height - 45)
        .with_size(100, 35)
        .with_label("Cancel");

    dialog.end();
    dialog.show();

    // Result tracking
    let result: Rc<RefCell<Option<(u16, u16)>>> = Rc::new(RefCell::new(None));

    // OK button callback
    {
        let from_field = from_field.clone();
        let to_field = to_field.clone();
        let result = result.clone();
        let mut dialog = dialog.clone();

        ok_btn.set_callback(move |_| {
            let from_text = from_field.value();
            let to_text = to_field.value();

            if let (Some(from_addr), Some(to_addr)) = (parse_hex_address(&from_text), parse_hex_address(&to_text)) {
                if to_addr >= from_addr {
                    let count = to_addr - from_addr + 1;
                    *result.borrow_mut() = Some((from_addr, count));
                    dialog.hide();
                } else {
                    dialog::alert_default("'From' address must be less than or equal to 'to' address.");
                }
            } else {
                dialog::alert_default("Please enter valid hexadecimal addresses (range $0100-$FFFF).");
            }
        });
    }

    // Cancel button callback
    {
        let mut dialog = dialog.clone();
        cancel_btn.set_callback(move |_| {
            dialog.hide();
        });
    }

    // Window close callback
    {
        let mut dialog_ref = dialog.clone();
        dialog.set_callback(move |_| {
            if app::event() == enums::Event::Close {
                dialog_ref.hide();
            }
        });
    }

    while dialog.shown() {
        app::wait();
    }

    result.borrow().clone()
}

/// Check if an error message indicates an allocation failure
fn is_allocation_error(error_msg: &str) -> bool {
    error_msg.contains("Failed to allocate block")
}

/// Show help window with usage instructions
fn show_help_window() {
    let help_width = 640;
    let help_height = 600;

    let mut help_window = Window::default()
        .with_size(help_width, help_height)
        .with_label(&format!("Help - VICE Snapshot to PRG/CRT Converter v{}", VERSION));
    help_window.make_resizable(false);
    help_window.set_pos(
        (app::screen_size().0 as i32 - help_width) / 2,
        (app::screen_size().1 as i32 - help_height) / 2,
    );

    let help_text = format!(
        r#"VICE Snapshot to PRG/CRT Converter v{}

Copyright (c) 2025-2026 Tommy Olsen
Licensed under the MIT License.

===============================================================

OVERVIEW

Converts VICE emulator snapshots (.vsf files) into:
- Self-restoring PRG files (run on real C64 hardware)
- EasyFlash CRT cartridges (boot directly from cartridge)
- Magic Desk CRT cartridges (8K cart mode, ROML only)

===============================================================

PRG OUTPUT

Creates a standard C64 PRG file that can be loaded and run:
  LOAD "yourfile.prg",8,1
  RUN

===============================================================

CRT OUTPUT

EasyFlash:
- Ultimax mode: ROML + ROMH
- Optional LOAD/SAVE hooking for embedded PRG files
- Files placed in "Include directory" can be LOADed from BASIC

Magic Desk:
- 8K cart mode: ROML only ($8000-$9FFF)
- CBM80 boot; $DE00 bit 7 banks the cart out (reversible)
- Optional LOAD/SAVE hooking for embedded PRG files

LOAD/SAVE Hooking (EasyFlash and Magic Desk):
When enabled, you can embed PRG files that can be loaded:
  LOAD "FILENAME",8,1

The cartridge intercepts KERNAL LOAD/SAVE vectors and serves
files from ROM banks instead of disk. The small trampoline in
C64 RAM is auto-placed from the snapshot's stack pointer
($0100 when the stack allows it, otherwise the cassette
buffer at $0334); uncheck "Auto location" to set the address
manually.

===============================================================

QUICK START

1. In the VICE monitor (Alt+H), run:
   f 0000 ffff 00
   reset
   x (exit monitor)

2. Load your program normally (avoid "Smart attach...")

3. Take snapshot: File -> Save snapshot image (.vsf)

4. In this converter:
   - Select input .vsf file
   - Choose output format (PRG or CRT tab)
   - Configure options
   - Click Convert

===============================================================

CLEAR POWER-ON RAM PATTERN (EXPERIMENTAL, OFF BY DEFAULT)

A freshly powered C64 (and VICE's default RAM init) holds a
fixed $00/$FF pattern, not zeros. Untouched pattern RAM looks
"used" to the free-block scan, which can cause allocation
failures. When enabled, regions still holding the exact
power-on pattern (64+ bytes) are zeroed and become usable free
space - automating the manual "f 0000 ffff 00" step for
snapshots taken without it.

This is HIGHLY EXPERIMENTAL: a misdetection could zero real
program data. It is off by default and asks for confirmation
when enabled. The manual "f 0000 ffff 00" step remains the
reliable way to prepare a snapshot.

===============================================================

MANUAL RAM BLOCKS

If conversion fails due to insufficient free memory, you can
manually specify RAM regions to use. A dialog will appear
offering to add a free RAM block.

Enter the hex address range (e.g., $0800 to $08FF) for memory
you know is unused by the program. The specified region will
be zeroed and made available for allocation.

===============================================================

IMPORTANT LIMITATIONS

- Memory MUST be initialized before snapshot (f 0000 ffff 00)
- Do NOT use "Smart attach..." feature in VICE
- "Clear power-on RAM pattern" is an experimental, opt-in
  alternative to the above; see the section above
"#, VERSION);

    let mut text_buffer = TextBuffer::default();
    text_buffer.set_text(&help_text);

    let mut text_display = TextDisplay::default()
        .with_pos(15, 15)
        .with_size(help_width - 30, help_height - 70);
    text_display.set_buffer(text_buffer);
    text_display.wrap_mode(text::WrapMode::AtBounds, 0);
    text_display.set_frame(FrameType::DownBox);

    let mut close_btn = Button::default()
        .with_pos((help_width - 100) / 2, help_height - 45)
        .with_size(100, 35)
        .with_label("Close");

    help_window.end();
    help_window.make_modal(true);
    help_window.show();

    close_btn.set_callback({
        let mut win = help_window.clone();
        move |_| {
            win.hide();
        }
    });

    while help_window.shown() {
        app::wait();
    }
}

/// Run `job` on a worker thread while the event loop keeps running, showing a modal window that
/// names the current step and offers Cancel.
///
/// The worker is always joined before this returns, so by the time the caller sees the result the
/// job has finished unwinding and its temporary files are gone. Cancellation is cooperative: the
/// job notices the flag between steps, so a cancel takes effect once the current step completes.
fn run_with_progress<F>(title: &str, progress: &Progress, job: F) -> Result<u32, String>
where
    F: FnOnce() -> Result<u32, String> + Send + 'static,
{
    let (tx, rx) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        // A panic in the job must still produce a result, otherwise the loop below would wait
        // forever. The work-directory guard runs during the unwind either way.
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(job))
            .unwrap_or_else(|_| Err("Conversion failed: internal error.".to_string()));
        let _ = tx.send(outcome);
        app::awake();
    });

    const W: i32 = 470;
    const H: i32 = 175;
    let mut win = Window::default().with_size(W, H).with_label(title);

    let mut step_frame = Frame::default()
        .with_pos(20, 22)
        .with_size(W - 40, 26)
        .with_label("Starting...");
    step_frame.set_align(Align::Left | Align::Inside);
    step_frame.set_label_size(14);

    let mut note = Frame::default()
        .with_pos(20, 54)
        .with_size(W - 40, 44)
        .with_label("The slower compression formats can take a while.");
    note.set_align(Align::Left | Align::Inside | Align::Wrap);
    note.set_label_size(12);

    let mut cancel_btn = Button::default()
        .with_pos((W - 120) / 2, H - 54)
        .with_size(120, 32)
        .with_label("Cancel");

    win.end();
    win.make_modal(true);

    // Acknowledge a cancel request on screen. The window stays open: the worker still owns the
    // work directory and has to unwind before the result can be reported.
    let acknowledge = {
        let progress = progress.clone();
        let mut note = note.clone();
        let mut cancel_btn = cancel_btn.clone();
        move || {
            progress.cancel();
            cancel_btn.deactivate();
            cancel_btn.set_label("Cancelling");
            note.set_label("Cancelling - the step in progress has to finish first.");
        }
    };

    // The title-bar close means cancel, never quit, and gives the same feedback as the button.
    win.set_callback({
        let mut acknowledge = acknowledge.clone();
        move |_| acknowledge()
    });
    cancel_btn.set_callback({
        let mut acknowledge = acknowledge.clone();
        move |_| acknowledge()
    });

    win.show();

    let mut shown = String::new();
    let outcome = loop {
        match rx.try_recv() {
            Ok(result) => break result,
            // The worker always sends before it ends, so a closed channel means it died
            // without reporting. Bail out rather than pump events forever.
            Err(mpsc::TryRecvError::Disconnected) => {
                break Err("Conversion failed: the worker stopped unexpectedly.".to_string())
            }
            Err(mpsc::TryRecvError::Empty) => {}
        }
        let step = progress.current_step();
        if step != shown {
            step_frame.set_label(&step);
            shown = step;
        }
        // A short timeout keeps the step label live while the worker is busy. `Ok(false)` only
        // means the timeout expired with nothing to dispatch, so it carries no meaning here;
        // `Err` is an interrupted wait, where a brief pause avoids a spin.
        if app::wait_for(0.1).is_err() {
            std::thread::sleep(Duration::from_millis(30));
        }
        // Check the quit flag here. Ask the job to stop, but keep pumping events until it
        // returns so its cleanup runs.
        if app::should_program_quit() {
            progress.cancel();
        }
    };

    win.hide();
    // fltk widgets are not freed when their Rust handle drops, so the window and its children
    // (and the callbacks holding the Progress clones) have to be released explicitly.
    Window::delete(win);
    let _ = worker.join();
    outcome
}
