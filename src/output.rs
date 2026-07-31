//! Optional file output for commands that can print a lot of data.
//!
//! Without `--out` every command behaves exactly as before: the body of the
//! output goes to stdout. With `--out <PATH>` the *body* is written to that
//! file instead and stdout only keeps the short status/summary lines, so a
//! caller that pipes nostaro into an LLM prompt (OpenCrab) never has to pay for
//! a 979-entry follow list.
//!
//! Commands emit their body through the [`outln!`](crate::outln) macro (text)
//! or [`write_json`] (structured). Everything printed with plain `println!`
//! stays on stdout in both modes.

use anyhow::{bail, Context, Result};
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard, OnceLock};

/// Format of the body written to `--out`.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, clap::ValueEnum)]
pub enum OutFormat {
    /// The same human-readable listing that would have gone to stdout.
    #[default]
    Text,
    /// A single machine-readable JSON document.
    Json,
}

#[derive(Default)]
struct Sink {
    path: Option<PathBuf>,
    format: OutFormat,
    /// Created lazily on the first body write, so a command that produces no
    /// body does not leave an empty file behind.
    writer: Option<BufWriter<File>>,
    lines: usize,
    json_written: bool,
}

impl Sink {
    fn writer(&mut self) -> Result<&mut BufWriter<File>> {
        if self.writer.is_none() {
            let path = self
                .path
                .clone()
                .expect("writer() is only reachable when --out was given");
            let file = File::create(&path)
                .with_context(|| format!("failed to create output file {}", path.display()))?;
            self.writer = Some(BufWriter::new(file));
        }
        Ok(self
            .writer
            .as_mut()
            .expect("writer was just initialized above"))
    }
}

static SINK: OnceLock<Mutex<Sink>> = OnceLock::new();

fn sink() -> MutexGuard<'static, Sink> {
    SINK.get_or_init(|| Mutex::new(Sink::default()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Point the body output at `path` (stdout when `None`), resetting any
/// previous target. Called once per run, before the command executes.
pub fn configure(path: Option<PathBuf>, format: OutFormat) {
    let mut sink = sink();
    *sink = Sink {
        path,
        format,
        ..Sink::default()
    };
}

/// True when the caller asked for the body as JSON. Commands that can produce a
/// structured body check this and call [`write_json`] instead of `outln!`.
pub fn is_json() -> bool {
    let sink = sink();
    sink.path.is_some() && sink.format == OutFormat::Json
}

/// Declare that this command owns the body, before writing any of it.
///
/// Creates the `--out` file even when the body turns out to be empty, so that
/// "no results" is an empty file rather than a missing one (which a caller
/// would otherwise not be able to tell from "this command ignores --out").
pub fn open_body() -> Result<()> {
    let mut sink = sink();
    if sink.path.is_some() && sink.format == OutFormat::Text {
        sink.writer()?;
    }
    Ok(())
}

/// Write one line of body output. Prefer the [`outln!`](crate::outln) macro.
pub fn write_line(args: std::fmt::Arguments<'_>) -> Result<()> {
    let mut sink = sink();
    if sink.path.is_none() {
        println!("{}", args);
        return Ok(());
    }
    if sink.format == OutFormat::Json {
        // In JSON mode the JSON document *is* the body; the text rendering is
        // dropped rather than mixed into the file.
        return Ok(());
    }
    let writer = sink.writer()?;
    writeln!(writer, "{}", args).context("failed to write to the output file")?;
    sink.lines += 1;
    Ok(())
}

/// Write the structured body. Only called when [`is_json`] is true.
pub fn write_json(value: &serde_json::Value) -> Result<()> {
    let text = serde_json::to_string_pretty(value)?;
    let mut sink = sink();
    if sink.path.is_none() {
        println!("{}", text);
        return Ok(());
    }
    let writer = sink.writer()?;
    writeln!(writer, "{}", text).context("failed to write to the output file")?;
    sink.json_written = true;
    Ok(())
}

/// Flush and close the output file, then print the one-line summary on stdout.
///
/// Called once, at the end of a successful run.
pub fn finish() -> Result<()> {
    let mut sink = sink();
    let Some(path) = sink.path.clone() else {
        return Ok(());
    };

    // Defensive: the CLI rejects `--out-format json` on commands without a JSON
    // body *before* the command runs, so getting here means a supported command
    // forgot to emit its document. All four are read-only, so failing this late
    // cannot leave a half-published event behind.
    if sink.format == OutFormat::Json && !sink.json_written {
        bail!("this command produced no JSON output; re-run without --out-format json");
    }

    if let Some(writer) = sink.writer.as_mut() {
        writer
            .flush()
            .with_context(|| format!("failed to write {}", path.display()))?;
    }

    match (sink.writer.is_some(), sink.format) {
        (false, _) => println!(
            "No file output for this command; {} was not written.",
            path.display()
        ),
        (true, OutFormat::Json) => println!("Wrote JSON output to {}", path.display()),
        (true, OutFormat::Text) => {
            println!("Wrote {} line(s) to {}", sink.lines, path.display())
        }
    }

    sink.writer = None;
    Ok(())
}

/// Best-effort flush used on the error path. The sink lives in a `static`, so
/// its `Drop` never runs and the buffer has to be pushed out by hand.
pub fn flush() {
    if let Some(writer) = sink().writer.as_mut() {
        let _ = writer.flush();
    }
}

/// `println!` for the *body* of a command's output: stdout normally, the
/// `--out` file when one was given.
#[macro_export]
macro_rules! outln {
    () => {
        $crate::output::write_line(format_args!(""))
    };
    ($($arg:tt)*) => {
        $crate::output::write_line(format_args!($($arg)*))
    };
}
