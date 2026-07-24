use anyhow::{bail, Result};
use nostr_sdk::{Keys, ToBech32};
use rayon::iter::{IntoParallelIterator, ParallelIterator};
use serde::Serialize;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

const BECH32_CHARS: &str = "qpzry9x8gf2tvdw0s3jn54khce6mua7l";

#[derive(Serialize)]
struct VanityResult {
    nsec: String,
    npub: String,
    pubkey: String,
}

/// `threads`/search only apply when `prefix` is non-empty; an empty prefix returns a
/// fresh random key immediately. Never reads or writes config/secret-key state.
pub fn run(prefix: &str, threads: Option<usize>, json_output: bool) -> Result<()> {
    for ch in prefix.chars() {
        if !BECH32_CHARS.contains(ch) {
            bail!(
                "Invalid bech32 character '{}'. Allowed: {}",
                ch,
                BECH32_CHARS
            );
        }
    }

    if prefix.is_empty() {
        return emit_result(&Keys::generate(), json_output);
    }

    let num_threads = threads.unwrap_or_else(num_cpus);
    print_status(json_output, format!("Searching for npub1{}...", prefix));
    print_status(json_output, format!("Using {} threads", num_threads));

    let counter = Arc::new(AtomicU64::new(0));
    let found = Arc::new(AtomicBool::new(false));
    let cancelled = Arc::new(AtomicBool::new(false));

    // Ctrl+C handler
    let cancelled_ctrlc = Arc::clone(&cancelled);
    ctrlc::set_handler(move || {
        cancelled_ctrlc.store(true, Ordering::SeqCst);
        eprintln!("\nCancelled.");
    })?;

    // Progress reporter thread
    let counter_progress = Arc::clone(&counter);
    let found_progress = Arc::clone(&found);
    let cancelled_progress = Arc::clone(&cancelled);
    let start = Instant::now();
    let progress_handle = std::thread::spawn(move || loop {
        std::thread::sleep(std::time::Duration::from_secs(1));
        if found_progress.load(Ordering::SeqCst) || cancelled_progress.load(Ordering::SeqCst) {
            break;
        }
        let count = counter_progress.load(Ordering::Relaxed);
        let elapsed = start.elapsed().as_secs();
        let rate = count.checked_div(elapsed).unwrap_or(count);
        eprintln!(
            "Tried: {} keys | Elapsed: {}s | Rate: {} keys/s",
            count, elapsed, rate
        );
    });

    // Build rayon thread pool and search
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(num_threads)
        .build()?;

    let target = format!("npub1{}", prefix);
    let result: Option<Keys> = pool.install(|| {
        let counter = Arc::clone(&counter);
        let found = Arc::clone(&found);
        let cancelled = Arc::clone(&cancelled);

        (0..usize::MAX).into_par_iter().find_map_any(|_| {
            if found.load(Ordering::Relaxed) || cancelled.load(Ordering::Relaxed) {
                return None;
            }

            let keys = Keys::generate();
            counter.fetch_add(1, Ordering::Relaxed);

            let npub = keys.public_key().to_bech32().ok()?;
            if npub.starts_with(&target) {
                found.store(true, Ordering::SeqCst);
                Some(keys)
            } else {
                None
            }
        })
    });

    let _ = progress_handle.join();

    let total = counter.load(Ordering::Relaxed);
    let elapsed = start.elapsed();

    match result {
        Some(keys) => {
            print_status(
                json_output,
                format!(
                    "\nFound after {} tries ({:.2}s)!",
                    total,
                    elapsed.as_secs_f64()
                ),
            );
            emit_result(&keys, json_output)
        }
        None => {
            print_status(
                json_output,
                format!(
                    "\nSearch stopped after {} tries ({:.2}s). No match found.",
                    total,
                    elapsed.as_secs_f64()
                ),
            );
            Ok(())
        }
    }
}

/// Route a human-readable progress/status line to stdout normally, or to stderr when
/// `--json` is active so stdout stays reserved for the single trailing JSON result line.
fn print_status(json_output: bool, msg: String) {
    if json_output {
        eprintln!("{}", msg);
    } else {
        println!("{}", msg);
    }
}

fn emit_result(keys: &Keys, json_output: bool) -> Result<()> {
    let nsec = keys.secret_key().to_bech32()?;
    let npub = keys.public_key().to_bech32()?;
    let pubkey = keys.public_key().to_hex();

    if json_output {
        let result = VanityResult { nsec, npub, pubkey };
        println!("{}", serde_json::to_string(&result)?);
    } else {
        println!("nsec: {}", nsec);
        println!("npub: {}", npub);
    }
    Ok(())
}

fn num_cpus() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
}
