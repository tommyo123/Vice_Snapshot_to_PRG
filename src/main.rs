//! Converts VICE 3.6-3.10 x64sc snapshot images (VSF) to C64 PRG files, EasyFlash CRT or Magic Desk CRT cartridges.
//!
// Copyright (c) 2025 Tommy Olsen
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
use std::path::Path;

use vice_snapshot_to_prg_converter::config::{Config, CrtConfig, VERSION};
use vice_snapshot_to_prg_converter::convert_snapshot::ConvertSnapshot;
use vice_snapshot_to_prg_converter::convert_snapshot_crt::ConvertSnapshotCRT;
use vice_snapshot_to_prg_converter::convert_snapshot_magic_desk_crt::ConvertSnapshotMagicDeskCRT;

const WINDOW_WIDTH: i32 = 720;
const WINDOW_HEIGHT: i32 = 720;
const MARGIN: i32 = 25;
const FIELD_HEIGHT: i32 = 35;
const BUTTON_HEIGHT: i32 = 40;
const BUTTON_WIDTH: i32 = 120;
const BROWSE_BTN_WIDTH: i32 = 60;
const TAB_HEIGHT: i32 = 490;

fn main() {
    let app = app::App::default().with_scheme(app::Scheme::Gtk);

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
        .with_label(&format!("VICE 3.6-3.10 x64sc Snapshot to PRG/CRT Converter v{}", VERSION));
    window.make_resizable(false);

    if let Ok(icon) = SvgImage::from_data(icon_svg) {
        window.set_icon(Some(icon));
    }

    let mut y_pos = MARGIN;

    // Create tabs
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
        .with_label("Select VICE snapshot image:");
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
        .with_label("Select VICE snapshot image:");
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
    let status_buffer_rc = Rc::new(RefCell::new(status_buffer));
    let tabs_rc = Rc::new(RefCell::new(tabs.clone()));

    // Extra RAM blocks for allocation failures (shared between PRG and CRT)
    // Each block is (address, count) - cleared on snapshot change or tab switch
    let extra_ram_blocks_rc: Rc<RefCell<Vec<(u16, u16)>>> = Rc::new(RefCell::new(Vec::new()));

    // CRT cartridge type callback - disable LOAD/SAVE for Magic Desk
    {
        let hook_check = crt_hook_check_rc.clone();
        let auto_location_check = crt_auto_location_check_rc.clone();
        let addr_field = crt_addr_field_rc.clone();
        let include_field = crt_include_field_rc.clone();
        let include_btn = crt_include_btn_rc.clone();

        crt_type_choice.clone().set_callback(move |choice| {
            let is_magic_desk = choice.value() == 1;
            if is_magic_desk {
                // Magic Desk: force-uncheck and disable LOAD/SAVE hooking
                hook_check.borrow_mut().set_checked(false);
                hook_check.borrow_mut().deactivate();
                auto_location_check.borrow_mut().deactivate();
                addr_field.borrow_mut().deactivate();
                include_field.borrow_mut().deactivate();
                include_btn.borrow_mut().deactivate();
            } else {
                // EasyFlash: re-enable hook checkbox
                hook_check.borrow_mut().activate();
            }
        });
    }

    // CRT hook checkbox callback - enable/disable related fields
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
                // addr_field only enabled if auto_location is NOT checked
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

    // CRT auto location checkbox callback - enable/disable addr field
    {
        let addr_field = crt_addr_field_rc.clone();
        let hook_check = crt_hook_check_rc.clone();

        crt_auto_location_check.clone().set_callback(move |check| {
            // Only toggle addr_field if hook is enabled
            if hook_check.borrow().is_checked() {
                if check.is_checked() {
                    addr_field.borrow_mut().deactivate();
                } else {
                    addr_field.borrow_mut().activate();
                }
            }
        });
    }

    // CRT address field - format and validate on change
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
                        // Format with $ prefix
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

        prg_input_btn.set_callback(move |_| {
            let mut chooser = NativeFileChooser::new(dialog::NativeFileChooserType::BrowseFile);
            chooser.set_title("Select VICE Snapshot Image");
            chooser.set_filter("VSF Files\t*.vsf\nAll Files\t*");

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

                // Default output = same name as input but with .prg extension
                let suggested_output = filename.with_extension("prg");
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
                let preset = input.with_extension("prg");
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

        crt_input_btn.set_callback(move |_| {
            let mut chooser = NativeFileChooser::new(dialog::NativeFileChooserType::BrowseFile);
            chooser.set_title("Select VICE Snapshot Image");
            chooser.set_filter("VSF Files\t*.vsf\nAll Files\t*");

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

                // Default output = same name as input but with .crt extension
                let suggested_output = filename.with_extension("crt");
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
                let preset = input.with_extension("crt");
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
        let status_buffer = status_buffer_rc.clone();
        let tabs = tabs_rc.clone();
        let extra_blocks = extra_ram_blocks_rc.clone();

        convert_btn.set_callback(move |btn| {
            let tabs_val = tabs.borrow();
            let active_tab = tabs_val.value().map(|w| w.label()).unwrap_or_default();
            let is_crt = active_tab.contains("CRT");

            status_buffer.borrow_mut().set_text("");

            if is_crt {
                // CRT conversion
                let input_path = crt_input.borrow().value();
                let output_path = crt_output.borrow().value();
                let cart_name = crt_name.borrow().value();
                let is_magic_desk = crt_type.borrow().value() == 1;
                let hook_enabled = crt_hook.borrow().is_checked() && !is_magic_desk;
                let auto_location = crt_auto_location.borrow().is_checked();
                let addr_text = crt_addr.borrow().value();
                let include_dir = crt_include.borrow().value();
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

                // Validate include directory when hook is enabled (EasyFlash only)
                if hook_enabled && !is_magic_desk && include_dir.is_empty() {
                    status_buffer.borrow_mut().set_text("Error: Include directory is required when LOAD/SAVE hooking is enabled.\n\nPlease select a directory containing PRG files to embed.");
                    return;
                }

                if hook_enabled && !is_magic_desk && !include_dir.is_empty() && !Path::new(&include_dir).is_dir() {
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

                    let result = CrtConfig::auto().map_err(|e| e.to_string()).and_then(|mut config| {
                        if !cart_name.is_empty() {
                            config.cartridge_name = Some(cart_name.clone());
                        }
                        if hook_enabled && !is_magic_desk && !include_dir.is_empty() {
                            config.include_dir = Some(include_dir.clone());
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
                        let conversion_result = if is_magic_desk {
                            let converter = ConvertSnapshotMagicDeskCRT::with_extra_blocks(config, current_blocks);
                            converter.convert(&input_path, &output_path)
                        } else {
                            let converter = ConvertSnapshotCRT::with_extra_blocks(config, current_blocks);
                            converter.convert(&input_path, &output_path)
                        };

                        let _ = cleanup_work_dir(&work_path);
                        conversion_result
                    });

                    match result {
                        Ok(()) => {
                            // Success - clear extra blocks
                            extra_blocks.borrow_mut().clear();
                            let success_msg = format!(
                                "Success!\n\nSnapshot successfully converted to {} CRT:\n{}",
                                cart_type_name, output_path
                            );
                            status_buffer.borrow_mut().set_text(&success_msg);
                            break;
                        }
                        Err(e) => {
                            if is_allocation_error(&e) {
                                // Allocation failure - offer to add RAM block
                                status_buffer.borrow_mut().set_text(&format!("Conversion failed:\n\n{}", e));

                                let choice = dialog::choice2_default(
                                    &format!("{}\n\nWould you like to add a free RAM block manually?", e),
                                    "No",
                                    "Yes",
                                    ""
                                );

                                if choice == Some(1) {
                                    // User wants to add a block
                                    if let Some((addr, count)) = show_add_ram_block_dialog() {
                                        extra_blocks.borrow_mut().push((addr, count));
                                        let end_addr = addr + count - 1;
                                        let mut buf = status_buffer.borrow_mut();
                                        buf.append(&format!(
                                            "\nAdded extra RAM block: ${:04X}-${:04X} ({} bytes)\n",
                                            addr, end_addr, count
                                        ));
                                        buf.append("Retrying conversion...\n\n");
                                        // Continue loop to retry
                                        continue;
                                    }
                                }
                                // User cancelled or didn't add block
                                break;
                            } else {
                                // Other error - don't retry
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

                    let config_result = Config::auto();

                    let result = match config_result {
                        Ok(config) => {
                            let work_path = config.work_path.clone();

                            let converter = ConvertSnapshot::with_extra_blocks(config, current_blocks);
                            let conversion_result = converter.convert(&input_path, &output_path);

                            let cleanup_result = cleanup_work_dir(&work_path);

                            match (conversion_result, cleanup_result) {
                                (Ok(()), Ok(())) => Ok(()),
                                (Ok(()), Err(cleanup_err)) => {
                                    Err(format!("Conversion succeeded, but failed to clean up temporary directory:\n{}", cleanup_err))
                                },
                                (Err(conv_err), Ok(())) => Err(conv_err),
                                (Err(conv_err), Err(_)) => Err(conv_err),
                            }
                        },
                        Err(e) => Err(format!("Failed to initialize configuration: {}", e)),
                    };

                    match result {
                        Ok(()) => {
                            // Success - clear extra blocks
                            extra_blocks.borrow_mut().clear();
                            let success_msg = format!(
                                "Success!\n\nSnapshot image successfully converted to:\n{}",
                                output_path
                            );
                            status_buffer.borrow_mut().set_text(&success_msg);
                            break;
                        }
                        Err(e) => {
                            if is_allocation_error(&e) {
                                // Allocation failure - offer to add RAM block
                                status_buffer.borrow_mut().set_text(&format!("Conversion failed:\n\n{}", e));

                                let choice = dialog::choice2_default(
                                    &format!("{}\n\nWould you like to add a free RAM block manually?", e),
                                    "No",
                                    "Yes",
                                    ""
                                );

                                if choice == Some(1) {
                                    // User wants to add a block
                                    if let Some((addr, count)) = show_add_ram_block_dialog() {
                                        extra_blocks.borrow_mut().push((addr, count));
                                        let end_addr = addr + count - 1;
                                        let mut buf = status_buffer.borrow_mut();
                                        buf.append(&format!(
                                            "\nAdded extra RAM block: ${:04X}-${:04X} ({} bytes)\n",
                                            addr, end_addr, count
                                        ));
                                        buf.append("Retrying conversion...\n\n");
                                        // Continue loop to retry
                                        continue;
                                    }
                                }
                                // User cancelled or didn't add block
                                break;
                            } else {
                                // Other error - don't retry
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

    quit_btn.set_callback(|_| {
        app::quit();
    });

    window.set_callback(|_| {
        if app::event() == enums::Event::Close {
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
        r#"VICE 3.6-3.10 x64sc Snapshot to PRG/CRT Converter v{}

Copyright (c) 2025 Tommy Olsen
Licensed under the MIT License.

===============================================================

OVERVIEW

Converts VICE 3.6-3.10 x64sc emulator snapshots (.vsf files) into:
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
- CBM80 boot, permanent kill via $DE00 bit 7
- No LOAD/SAVE hooking (use EasyFlash for that)

LOAD/SAVE Hooking (EasyFlash only):
When enabled, you can embed PRG files that can be loaded:
  LOAD "FILENAME",8,1

The cartridge intercepts KERNAL LOAD/SAVE vectors and serves
files from ROM banks instead of disk.

===============================================================

QUICK START

1. In VICE 3.6-3.10 x64sc monitor (Alt+H), run:
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

MANUAL RAM BLOCKS

If conversion fails due to insufficient free memory, you can
manually specify RAM regions to use. A dialog will appear
offering to add a free RAM block.

Enter the hex address range (e.g., $0800 to $08FF) for memory
you know is unused by the program. The specified region will
be zeroed and made available for allocation.

===============================================================

IMPORTANT LIMITATIONS

- Only works with VICE 3.6-3.10 x64sc snapshots
- Memory MUST be initialized before snapshot (f 0000 ffff 00)
- Do NOT use "Smart attach..." feature in VICE
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

/// Clean up the temporary work directory
fn cleanup_work_dir(work_path: &Path) -> Result<(), String> {
    if work_path.exists() {
        std::fs::remove_dir_all(work_path)
            .map_err(|e| format!("Failed to remove work directory {:?}: {}", work_path, e))?;
    }
    Ok(())
}
