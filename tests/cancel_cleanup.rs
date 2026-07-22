//! Cancelling a conversion must stop it and leave no temporary files behind.

use std::path::PathBuf;

use vice_snapshot_to_prg_converter::config::{finish_conversion, Config, CrtConfig, WorkDirGuard};
use vice_snapshot_to_prg_converter::convert_snapshot::ConvertSnapshot;
use vice_snapshot_to_prg_converter::convert_snapshot_crt::ConvertSnapshotCRT;
use vice_snapshot_to_prg_converter::convert_snapshot_magic_desk_crt::ConvertSnapshotMagicDeskCRT;
use vice_snapshot_to_prg_converter::progress::{is_cancelled_error, Progress, CANCELLED};

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "vsf_cancel_{}_{}_{:?}",
        tag,
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn progress_reports_the_current_step_and_cancels() {
    let p = Progress::new();
    assert!(!p.is_cancelled());
    assert!(p.check().is_ok());

    p.step("Compressing RAM...").unwrap();
    assert_eq!(p.current_step(), "Compressing RAM...");

    // A cloned handle shares state with the original. The GUI and the worker use this.
    let ui = p.clone();
    ui.cancel();
    assert!(p.is_cancelled());
    assert_eq!(p.check().unwrap_err(), CANCELLED);
    // Once cancelled, starting a step fails instead of proceeding.
    assert_eq!(p.step("Assembling PRG...").unwrap_err(), CANCELLED);
    assert!(is_cancelled_error(CANCELLED));
}

#[test]
fn work_dir_guard_removes_the_directory_on_drop() {
    let dir = scratch("guard");
    std::fs::write(dir.join("ram.lzsa"), b"intermediate").unwrap();
    assert!(dir.exists());

    {
        let _guard = WorkDirGuard::new(dir.clone());
    }
    assert!(!dir.exists(), "work directory survived the guard");
}

#[test]
fn work_dir_guard_removes_the_directory_on_panic() {
    let dir = scratch("panic");
    std::fs::write(dir.join("ram.lzsa"), b"intermediate").unwrap();

    let d = dir.clone();
    let caught = std::panic::catch_unwind(move || {
        let _guard = WorkDirGuard::new(d);
        panic!("conversion blew up");
    });

    assert!(caught.is_err(), "the panic should have propagated");
    assert!(!dir.exists(), "work directory survived a panic");
}

/// A conversion whose handle is already cancelled must report cancellation rather than doing the
/// work. The check sits ahead of reading the input, so a missing input still reports CANCELLED.
#[test]
fn a_cancelled_handle_stops_every_converter_before_it_reads_the_input() {
    let dir = scratch("stop");
    let _guard = WorkDirGuard::new(dir.clone());
    let missing = dir.join("no_such_snapshot.vsf");
    let missing = missing.to_str().unwrap();

    let progress = Progress::new();
    progress.cancel();

    let out_prg = dir.join("out.prg");
    let err = ConvertSnapshot::new(
        Config::new(&dir).with_progress(progress.clone()),
    )
    .convert(missing, out_prg.to_str().unwrap())
    .unwrap_err();
    assert!(is_cancelled_error(&err), "PRG: expected cancellation, got: {err}");
    assert!(!out_prg.exists(), "PRG: an output file was written");

    for (tag, is_magic_desk) in [("ef", false), ("md", true)] {
        let out = dir.join(format!("out_{tag}.crt"));
        let cfg = CrtConfig::new(Config::new(&dir).with_progress(progress.clone()));
        let err = if is_magic_desk {
            ConvertSnapshotMagicDeskCRT::new(cfg).convert(missing, out.to_str().unwrap())
        } else {
            ConvertSnapshotCRT::new(cfg).convert(missing, out.to_str().unwrap())
        }
        .unwrap_err();
        assert!(is_cancelled_error(&err), "{tag}: expected cancellation, got: {err}");
        assert!(!out.exists(), "{tag}: an output file was written");
    }
}

#[test]
fn a_cancelled_run_deletes_the_output_it_had_started() {
    let dir = scratch("outfile");
    let _guard = WorkDirGuard::new(dir.clone());

    let work = dir.join("work");
    std::fs::create_dir_all(&work).unwrap();
    let out = dir.join("half_written.prg");
    std::fs::write(&out, b"truncated output").unwrap();
    // The guard for the work directory has already run by the time finish_conversion is called.
    std::fs::remove_dir_all(&work).unwrap();

    let r = finish_conversion(Err(CANCELLED.to_string()), &work, out.to_str().unwrap());
    assert!(is_cancelled_error(&r.unwrap_err()));
    assert!(!out.exists(), "cancelled run left a partial output file behind");
}

#[test]
fn a_successful_run_keeps_its_output_and_reports_a_stuck_work_dir() {
    let dir = scratch("kept");
    let _guard = WorkDirGuard::new(dir.clone());

    let out = dir.join("good.prg");
    std::fs::write(&out, b"finished output").unwrap();
    let gone = dir.join("already_removed");

    // Normal case: work directory gone, output kept.
    assert_eq!(finish_conversion(Ok(7), &gone, out.to_str().unwrap()), Ok(7));
    assert!(out.exists(), "successful run lost its output");

    // A work directory that still exists is reported as an error.
    let stuck = dir.join("stuck");
    std::fs::create_dir_all(&stuck).unwrap();
    let err = finish_conversion(Ok(7), &stuck, out.to_str().unwrap()).unwrap_err();
    assert!(err.contains("temporary directory"), "unexpected message: {err}");
    assert!(out.exists(), "output removed on a non-cancelled path");
}

/// A handle that is not cancelled must not short-circuit. The converter gets far enough to fail on
/// the missing input.
#[test]
fn an_active_handle_does_not_short_circuit() {
    let dir = scratch("active");
    let _guard = WorkDirGuard::new(dir.clone());
    let missing = dir.join("no_such_snapshot.vsf");
    let missing = missing.to_str().unwrap();
    let out = dir.join("out.prg");

    let err = ConvertSnapshot::new(Config::new(&dir).with_progress(Progress::new()))
        .convert(missing, out.to_str().unwrap())
        .unwrap_err();

    assert!(!is_cancelled_error(&err), "should not report cancellation: {err}");
    assert!(err.contains("read"), "expected a read failure, got: {err}");
}
