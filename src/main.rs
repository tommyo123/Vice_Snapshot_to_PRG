//! Converts VICE 3.9 x64sc snapshot images (VSF) to C64 PRG files.
//!
//! This program is unlicensed and dedicated to the public domain.
//! Developed by Tommy Olsen.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod parse_vsf;
mod make_prg_asm;
mod asm6502;
mod config;
mod find_ram;
mod patch_mem;
mod convert_snapshot;

use fltk::{prelude::*, *};
use fltk::button::Button;
use fltk::dialog::NativeFileChooser;
use fltk::enums::{Color, FrameType};
use fltk::frame::Frame;
use fltk::image::SvgImage;
use fltk::input::Input;
use fltk::text::{TextBuffer, TextDisplay};
use fltk::window::Window;
use std::cell::RefCell;
use std::rc::Rc;
use std::path::Path;

use config::{Config, VERSION};
use convert_snapshot::ConvertSnapshot;

const WINDOW_WIDTH: i32 = 720;
const WINDOW_HEIGHT: i32 = 580;
const MARGIN: i32 = 25;
const FIELD_HEIGHT: i32 = 35;
const BUTTON_HEIGHT: i32 = 40;
const BUTTON_WIDTH: i32 = 120;
const BROWSE_BTN_WIDTH: i32 = 60;

fn main() {
    let app = app::App::default().with_scheme(app::Scheme::Gtk);

    // Custom C64 chip icon
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

    // Create main window with version number
    let mut window = Window::default()
        .with_size(WINDOW_WIDTH, WINDOW_HEIGHT)
        .with_label(&format!("VICE 3.9 x64sc Snapshot to PRG Converter v{}", VERSION));
    window.make_resizable(false);

    // Set custom icon
    if let Ok(icon) = SvgImage::from_data(icon_svg) {
        window.set_icon(Some(icon));
    }

    let mut y_pos = MARGIN;

    // Input file section
    let mut input_label = Frame::default()
        .with_pos(MARGIN, y_pos)
        .with_size(WINDOW_WIDTH - 2 * MARGIN, 25)
        .with_label("Select VICE snapshot image:");
    input_label.set_label_size(13);
    input_label.set_align(enums::Align::Left | enums::Align::Inside);

    y_pos += 30;

    let input_field = Input::default()
        .with_pos(MARGIN, y_pos)
        .with_size(WINDOW_WIDTH - 2 * MARGIN - BROWSE_BTN_WIDTH - 10, FIELD_HEIGHT);

    let mut input_btn = Button::default()
        .with_pos(WINDOW_WIDTH - MARGIN - BROWSE_BTN_WIDTH, y_pos)
        .with_size(BROWSE_BTN_WIDTH, FIELD_HEIGHT)
        .with_label("Browse...");

    y_pos += FIELD_HEIGHT + 20;

    // Output file section
    let mut output_label = Frame::default()
        .with_pos(MARGIN, y_pos)
        .with_size(WINDOW_WIDTH - 2 * MARGIN, 25)
        .with_label("Select output C64 PRG file:");
    output_label.set_label_size(13);
    output_label.set_align(enums::Align::Left | enums::Align::Inside);

    y_pos += 30;

    let output_field = Input::default()
        .with_pos(MARGIN, y_pos)
        .with_size(WINDOW_WIDTH - 2 * MARGIN - BROWSE_BTN_WIDTH - 10, FIELD_HEIGHT);

    let mut output_btn = Button::default()
        .with_pos(WINDOW_WIDTH - MARGIN - BROWSE_BTN_WIDTH, y_pos)
        .with_size(BROWSE_BTN_WIDTH, FIELD_HEIGHT)
        .with_label("Browse...");

    y_pos += FIELD_HEIGHT + 20;

    // Status section
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

    // Action buttons - three buttons symmetrically placed
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

    // Create shared state for callbacks
    let input_field_rc = Rc::new(RefCell::new(input_field.clone()));
    let output_field_rc = Rc::new(RefCell::new(output_field.clone()));
    let status_buffer_rc = Rc::new(RefCell::new(status_buffer));
    let convert_btn_rc = Rc::new(RefCell::new(convert_btn.clone()));

    // Input file browse button callback
    {
        let input_field = input_field_rc.clone();
        let output_field = output_field_rc.clone();

        input_btn.set_callback(move |_| {
            let mut chooser = NativeFileChooser::new(dialog::NativeFileChooserType::BrowseFile);
            chooser.set_title("Select VICE Snapshot Image");
            chooser.set_filter("VSF Files\t*.vsf\nAll Files\t*");

            // Set initial directory if current value exists
            let current = input_field.borrow().value();
            if !current.is_empty() {
                if let Some(parent) = Path::new(&current).parent() {
                    let parent_str = parent.to_string_lossy().to_string();
                    let _ = chooser.set_directory(&parent_str);
                }
            }

            chooser.show();
            let filename = chooser.filename();

            if !filename.as_os_str().is_empty() {
                let path_str = filename.to_string_lossy().to_string();
                input_field.borrow_mut().set_value(&path_str);

                // Auto-suggest output filename in same directory if output is empty or default
                let output_val = output_field.borrow().value();
                if output_val.is_empty() || output_val == "output.prg" {
                    if let Some(parent) = filename.parent() {
                        let suggested_output = parent.join("output.prg");
                        output_field.borrow_mut().set_value(&suggested_output.to_string_lossy());
                    }
                }
            }
        });
    }

    // Output file browse button callback
    {
        let input_field = input_field_rc.clone();
        let output_field = output_field_rc.clone();

        output_btn.set_callback(move |_| {
            let mut chooser = NativeFileChooser::new(dialog::NativeFileChooserType::BrowseSaveFile);
            chooser.set_title("Save PRG File As");
            chooser.set_filter("PRG Files\t*.prg\nAll Files\t*");
            chooser.set_option(dialog::FileDialogOptions::SaveAsConfirm);

            // Set directory based on input file if available
            let input_path = input_field.borrow().value();
            if !input_path.is_empty() {
                if let Some(parent) = Path::new(&input_path).parent() {
                    let parent_str = parent.to_string_lossy().to_string();
                    let _ = chooser.set_directory(&parent_str);
                    chooser.set_preset_file("output.prg");
                }
            }

            chooser.show();
            let filename = chooser.filename();

            if !filename.as_os_str().is_empty() {
                output_field.borrow_mut().set_value(&filename.to_string_lossy());
            }
        });
    }

    // Help button callback
    help_btn.set_callback(|_| {
        show_help_window();
    });

    // Convert button callback
    {
        let input_field = input_field_rc.clone();
        let output_field = output_field_rc.clone();
        let status_buffer = status_buffer_rc.clone();
        let convert_btn = convert_btn_rc.clone();

        convert_btn.borrow_mut().set_callback(move |btn| {
            let input_path = input_field.borrow().value();
            let output_path = output_field.borrow().value();

            // Clear status
            status_buffer.borrow_mut().set_text("");

            // Validate inputs
            if input_path.is_empty() {
                status_buffer.borrow_mut().set_text("✗ Error: Please select an input VSF file");
                return;
            }

            if output_path.is_empty() {
                status_buffer.borrow_mut().set_text("✗ Error: Please specify an output PRG filename");
                return;
            }

            // Check if input file exists
            if !Path::new(&input_path).exists() {
                let msg = format!("✗ Error: Input file not found:\n{}", input_path);
                status_buffer.borrow_mut().set_text(&msg);
                return;
            }

            // Disable convert button during processing
            btn.deactivate();
            status_buffer.borrow_mut().set_text("Converting snapshot image...\n");
            app::awake();

            // Create config with automatic paths
            let config_result = Config::auto();

            let result = match config_result {
                Ok(config) => {
                    let work_path = config.work_path.clone();

                    // Perform conversion
                    let converter = ConvertSnapshot::new(config);
                    let conversion_result = converter.convert(&input_path, &output_path);

                    // Clean up work directory regardless of success or failure
                    let cleanup_result = cleanup_work_dir(&work_path);

                    // Return conversion result, but add cleanup warning if needed
                    match (conversion_result, cleanup_result) {
                        (Ok(()), Ok(())) => Ok(()),
                        (Ok(()), Err(cleanup_err)) => {
                            // Conversion succeeded but cleanup failed
                            Err(format!("Conversion succeeded, but failed to clean up temporary directory:\n{}", cleanup_err))
                        },
                        (Err(conv_err), Ok(())) => Err(conv_err),
                        (Err(conv_err), Err(_)) => Err(conv_err), // Prioritize conversion error
                    }
                },
                Err(e) => Err(format!("Failed to initialize configuration: {}", e)),
            };

            // Re-enable convert button
            btn.activate();

            // Display result
            match result {
                Ok(()) => {
                    let success_msg = format!(
                        "✓ Success!\n\nSnapshot image successfully converted to:\n{}",
                        output_path
                    );
                    status_buffer.borrow_mut().set_text(&success_msg);
                }
                Err(e) => {
                    let error_msg = format!("✗ Conversion failed:\n\n{}", e);
                    status_buffer.borrow_mut().set_text(&error_msg);
                }
            }
        });
    }

    // Quit button callback
    quit_btn.set_callback(|_| {
        app::quit();
    });

    // Handle window close
    window.set_callback(|_| {
        if app::event() == enums::Event::Close {
            app::quit();
        }
    });

    app.run().unwrap();
}

/// Show help window with usage instructions
fn show_help_window() {
    let help_width = 640;
    let help_height = 540;

    let mut help_window = Window::default()
        .with_size(help_width, help_height)
        .with_label(&format!("Help - VICE Snapshot to PRG Converter v{}", VERSION));
    help_window.make_resizable(false);
    help_window.set_pos(
        (app::screen_size().0 as i32 - help_width) / 2,
        (app::screen_size().1 as i32 - help_height) / 2,
    );

    let help_text = format!(
        r#"VICE 3.9 x64sc Snapshot to PRG Converter v{}

This program is unlicensed and dedicated to the public domain.
Developed by Tommy Olsen.

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

OVERVIEW

Converts VICE 3.9 x64sc emulator snapshots (.vsf files) into
self-restoring PRG files that run on real Commodore 64 hardware.

The PRG file will restore the complete machine state including CPU
registers, memory, VIC-II graphics, SID audio, CIA timers, and
zero page exactly as it was when the snapshot was taken.

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

QUICK START

1. In VICE 3.9 x64sc monitor (Alt+H), run:
   f 0000 ffff 00
   reset
   x (exit monitor)

2. Load your program normally (avoid "Smart attach...")

3. Take snapshot: File → Save snapshot image (.vsf)

4. In this converter:
   - Select input .vsf file
   - Choose output .prg filename
   - Click Convert

5. Transfer .prg to C64 and run:
   LOAD "yourfile.prg",8,1
   RUN

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

IMPORTANT LIMITATIONS

• Only works with VICE 3.9 x64sc snapshots
• Memory MUST be initialized before snapshot (f 0000 ffff 00)
• Do NOT use "Smart attach..." feature in VICE
• Some edge cases with unusual stack configurations may fail

━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

For complete documentation, see README.md in the installation
directory or visit: https://github.com/tommyo123/Vice_Snapshot_to_PRG
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
