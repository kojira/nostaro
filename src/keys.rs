use anyhow::{Context, Result};
use nostr_sdk::prelude::*;

use crate::config::NostaroConfig;

pub fn generate_keys() -> Keys {
    Keys::generate()
}

pub fn keys_from_config(config: &NostaroConfig) -> Result<Keys> {
    let secret_key = config
        .secret_key
        .as_ref()
        .context("No secret key found in config. Run `nostaro init` first.")?;
    let keys = Keys::parse(secret_key).context("Failed to parse secret key from config")?;
    Ok(keys)
}

pub fn display_key_info(keys: &Keys) -> Result<()> {
    let npub = keys.public_key().to_bech32()?;
    let nsec = keys.secret_key().to_bech32()?;
    let hex_pubkey = keys.public_key().to_hex();

    println!("Public key (npub): {}", npub);
    println!("Secret key (nsec): {}", nsec);
    println!("Public key (hex):  {}", hex_pubkey);

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_keys() {
        let keys = generate_keys();
        let npub = keys.public_key().to_bech32().unwrap();
        let nsec = keys.secret_key().to_bech32().unwrap();
        assert!(npub.starts_with("npub1"));
        assert!(nsec.starts_with("nsec1"));
    }

    #[test]
    fn test_keys_from_config_missing_key() {
        let config = NostaroConfig::default();
        let result = keys_from_config(&config);
        assert!(result.is_err());
    }

    #[test]
    fn test_keys_from_config_valid_key() {
        let keys = generate_keys();
        let nsec = keys.secret_key().to_bech32().unwrap();
        let mut config = NostaroConfig::default();
        config.secret_key = Some(nsec);
        let loaded_keys = keys_from_config(&config).unwrap();
        assert_eq!(loaded_keys.public_key(), keys.public_key());
    }
}
