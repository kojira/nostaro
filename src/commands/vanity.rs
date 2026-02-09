use anyhow::{bail, Result};
use nostr_sdk::{Keys, ToBech32};
use rayon::iter::{IntoParallelIterator, ParallelIterator};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

const BECH32_CHARS: &str = "qpzry9x8gf2tvdw0s3jn54khce6mua7l";

pub fn run(prefix: &str, threads: Option<usize>) -> Result<()> {
    // Validate prefix
    if prefix.is_empty() {
        bail!("Prefix must not be empty");
    }
    for ch in prefix.chars() {
        if !BECH32_CHARS.contains(ch) {
            bail!(
                "Invalid bech32 character '{}'. Allowed: {}",
                ch,
                BECH32_CHARS
            );
        }
    }

    let num_threads = threads.unwrap_or_else(num_cpus);
    println!("Searching for npub1{}...", prefix);
    println!("Using {} threads", num_threads);

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
    let progress_handle = std::thread::spawn(move || {
        loop {
            std::thread::sleep(std::time::Duration::from_secs(1));
            if found_progress.load(Ordering::SeqCst) || cancelled_progress.load(Ordering::SeqCst) {
                break;
            }
            let count = counter_progress.load(Ordering::Relaxed);
            let elapsed = start.elapsed().as_secs();
            let rate = if elapsed > 0 { count / elapsed } else { count };
            eprintln!(
                "Tried: {} keys | Elapsed: {}s | Rate: {} keys/s",
                count, elapsed, rate
            );
        }
    });

    // Build rayon thread pool and search
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(num_threads)
        .build()?;

    let target = format!("npub1{}", prefix);
    let result: Option<(String, String)> = pool.install(|| {
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
                let nsec = keys.secret_key().to_bech32().ok()?;
                Some((nsec, npub))
            } else {
                None
            }
        })
    });

    let _ = progress_handle.join();

    let total = counter.load(Ordering::Relaxed);
    let elapsed = start.elapsed();

    match result {
        Some((nsec, npub)) => {
            println!("\nFound after {} tries ({:.2}s)!", total, elapsed.as_secs_f64());
            println!("nsec: {}", nsec);
            println!("npub: {}", npub);
        }
        None => {
            println!(
                "\nSearch stopped after {} tries ({:.2}s). No match found.",
                total,
                elapsed.as_secs_f64()
            );
        }
    }

    Ok(())
}

fn num_cpus() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
}
