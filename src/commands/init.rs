use anyhow::{bail, Result};
use nostr_sdk::{Keys, SecretKey, ToBech32};
use std::io::{self, Write};

use crate::config::NostaroConfig;
use crate::keys;

pub async fn run() -> Result<()> {
    println!("Welcome to nostaro setup!\n");

    print!("Do you want to (1) generate a new key or (2) import an existing key? [1/2]: ");
    io::stdout().flush()?;

    let mut choice = String::new();
    io::stdin().read_line(&mut choice)?;
    let choice = choice.trim();

    let generated_keys = if choice == "2" {
        print!("Enter your secret key (nsec1... or hex): ");
        io::stdout().flush()?;

        let mut secret_input = String::new();
        io::stdin().read_line(&mut secret_input)?;
        let secret_input = secret_input.trim().to_string();

        if secret_input.is_empty() {
            bail!("No secret key provided");
        }

        let secret_key = SecretKey::parse(&secret_input)?;
        Keys::new(secret_key)
    } else {
        println!("Generating new keypair...");
        keys::generate_keys()
    };

    keys::display_key_info(&generated_keys)?;

    let mut config = NostaroConfig::load()?;
    config.secret_key = Some(generated_keys.secret_key().to_bech32()?);

    if config.relays.is_empty() {
        config.relays = config.default_relays.clone();
    }

    config.save()?;

    println!("\nConfiguration saved to ~/.nostaro/config.toml");
    println!("Default relays have been configured.");
    println!("\nYou're all set! Try posting with: nostaro post \"Hello Nostr!\"");

    Ok(())
}
