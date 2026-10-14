//! The one long-running job the app does at a time (a start, stop, build,
//! backup or restore) and what its steps share: the stack, the backups
//! folder, a way to report output, and the process to interrupt on cancel.

use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, Mutex};

use tauri::{AppHandle, Emitter};

use crate::stack;

pub struct Running {
    pub kind: String,
    /// The process the current step is waiting on, if any.
    pub pid: Option<u32>,
    pub cancelled: bool,
}

impl Running {
    /// Marks the task cancelled and interrupts its current process.
    pub fn cancel(&mut self) {
        self.cancelled = true;
        if let Some(pid) = self.pid {
            stack::interrupt(pid);
        }
    }
}

pub type Slot = Arc<Mutex<Option<Running>>>;

#[derive(Clone)]
pub struct Ctx {
    pub handle: AppHandle,
    pub dir: PathBuf,
    pub backups: PathBuf,
    pub slot: Slot,
}

impl Ctx {
    /// Adds a line to the Activity pane.
    pub fn line(&self, line: impl Into<String>) {
        let _ = self.handle.emit("task-line", line.into());
    }

    pub fn track(&self, pid: Option<u32>) {
        if let Some(running) = self.slot.lock().unwrap().as_mut() {
            running.pid = pid;
        }
    }

    pub fn check_cancelled(&self) -> Result<(), String> {
        match self.slot.lock().unwrap().as_ref() {
            Some(r) if r.cancelled => Err("Cancelled.".into()),
            _ => Ok(()),
        }
    }

    /// Runs a command to completion, streaming its output to the Activity
    /// pane. It runs even after a cancel, so clean-up steps still happen.
    pub fn run(&self, cmd: Command) -> Result<(), String> {
        let lines = self.clone();
        let (mut child, done) = stack::spawn_lines(cmd, move |line| lines.line(line)).map_err(|e| e.to_string())?;
        self.track(Some(child.id()));
        let _ = done.join();
        let status = child.wait().map_err(|e| e.to_string());
        self.track(None);
        let status = status?;
        if status.success() {
            return Ok(());
        }
        self.check_cancelled()?;
        Err(match status.code() {
            Some(code) => format!("The command stopped with exit code {code}."),
            None => "The command was stopped.".into(),
        })
    }

    pub fn compose(&self) -> Command {
        stack::compose(&self.dir)
    }
}
