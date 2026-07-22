//! Cooperative cancellation and step reporting for a running conversion.
//!
//! A conversion runs on a worker thread while the GUI stays responsive. The GUI holds one
//! [`Progress`] handle and the worker another, both sharing the same state. The worker publishes
//! the step it is on and checks for cancellation between steps. The GUI displays the step and
//! sets the cancel flag.
//!
// Copyright (c) 2025-2026 Tommy Olsen
// Licensed under the MIT License.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// Error text a cancelled conversion returns, so callers can tell it apart from a failure.
pub const CANCELLED: &str = "Conversion cancelled by user.";

/// Shared cancel flag and current-step text. Cloning yields another handle to the same state.
#[derive(Clone, Default)]
pub struct Progress {
    cancelled: Arc<AtomicBool>,
    step: Arc<Mutex<String>>,
}

impl Progress {
    pub fn new() -> Self {
        Self::default()
    }

    /// Ask the conversion to stop. It finishes the step it is on, then unwinds.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed)
    }

    /// `Err(CANCELLED)` once cancellation has been requested. Called between conversion steps.
    pub fn check(&self) -> Result<(), String> {
        if self.is_cancelled() {
            Err(CANCELLED.to_string())
        } else {
            Ok(())
        }
    }

    /// Publish the step now starting, and return `Err(CANCELLED)` if a cancel is pending.
    pub fn step(&self, what: &str) -> Result<(), String> {
        if let Ok(mut s) = self.step.lock() {
            s.clear();
            s.push_str(what);
        }
        self.check()
    }

    /// The step most recently published, for display.
    pub fn current_step(&self) -> String {
        self.step.lock().map(|s| s.clone()).unwrap_or_default()
    }
}

/// True if `err` is the cancellation marker rather than a real failure.
pub fn is_cancelled_error(err: &str) -> bool {
    err == CANCELLED
}
