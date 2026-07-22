use anyhow::Result;

use crate::config::NostaroConfig;
use crate::keys;

/// Print the configured account's public key (hex) to stdout.
///
/// Used by callers (e.g. OpenCrab) that spawn nostaro per-agent with `--config` to learn
/// their own pubkey so they can skip self-authored events in a watch loop.
pub async fn run() -> Result<()> {
    let config = NostaroConfig::load()?;
    let own_keys = keys::keys_from_config(&config)?;
    println!("{}", own_keys.public_key().to_hex());
    Ok(())
}
