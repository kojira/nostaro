use anyhow::{Context, Result};
use nostr_sdk::prelude::*;

use crate::config::NostaroConfig;

/// 実行時に秘密鍵を渡す環境変数。opencrab（呼び出し側）は鍵を config へ平文で書かず、
/// spawn ごとにこの env で注入する。素の CLI 運用（`nostaro init` で config に鍵を書く）は
/// env 未設定なら従来どおり config へフォールバックするので不変。
const SECRET_KEY_ENV: &str = "NOSTARO_SECRET_KEY";

pub fn generate_keys() -> Keys {
    Keys::generate()
}

/// 秘密鍵を解決する。**env `NOSTARO_SECRET_KEY` を最優先**し、無ければ config へフォールバックする。
///
/// env は opencrab が実行時に注入する渡し方（鍵を config へ平文で置かないため）。
/// **空文字・空白のみは「未設定」扱い**にする（誤って空を優先すると全コマンドが
/// 「鍵が空」で停止するため、fail する側ではなく config へ委ねる側に倒す）。
pub fn keys_from_config(config: &NostaroConfig) -> Result<Keys> {
    // env 優先。空/空白のみは未設定として無視する。
    if let Some(env_key) = std::env::var(SECRET_KEY_ENV)
        .ok()
        .filter(|s| !s.trim().is_empty())
    {
        return Keys::parse(env_key.trim())
            .context("Failed to parse secret key from NOSTARO_SECRET_KEY env");
    }
    // env 未設定なら従来どおり config から。
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

    /// env は process 全体で共有される。env を触るテストを直列化し、他テストの
    /// `keys_from_config`（env を読む）と競合させない。
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

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
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var(SECRET_KEY_ENV);
        let config = NostaroConfig::default();
        let result = keys_from_config(&config);
        assert!(result.is_err());
    }

    #[test]
    fn test_keys_from_config_valid_key() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var(SECRET_KEY_ENV);
        let keys = generate_keys();
        let nsec = keys.secret_key().to_bech32().unwrap();
        let config = NostaroConfig {
            secret_key: Some(nsec),
            ..NostaroConfig::default()
        };
        let loaded_keys = keys_from_config(&config).unwrap();
        assert_eq!(loaded_keys.public_key(), keys.public_key());
    }

    /// env が最優先される（config に別の鍵があっても env の鍵を使う）。
    #[test]
    fn test_env_secret_key_takes_priority_over_config() {
        let _guard = ENV_LOCK.lock().unwrap();
        let env_keys = generate_keys();
        let config_keys = generate_keys();
        assert_ne!(env_keys.public_key(), config_keys.public_key());

        std::env::set_var(SECRET_KEY_ENV, env_keys.secret_key().to_bech32().unwrap());
        let config = NostaroConfig {
            secret_key: Some(config_keys.secret_key().to_bech32().unwrap()),
            ..NostaroConfig::default()
        };
        let loaded = keys_from_config(&config).unwrap();
        std::env::remove_var(SECRET_KEY_ENV);
        assert_eq!(
            loaded.public_key(),
            env_keys.public_key(),
            "env の鍵が config より優先されなかった"
        );
    }

    /// env が未設定なら config へフォールバックする。
    #[test]
    fn test_falls_back_to_config_when_env_unset() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var(SECRET_KEY_ENV);
        let keys = generate_keys();
        let config = NostaroConfig {
            secret_key: Some(keys.secret_key().to_bech32().unwrap()),
            ..NostaroConfig::default()
        };
        let loaded = keys_from_config(&config).unwrap();
        assert_eq!(loaded.public_key(), keys.public_key());
    }

    /// env も config も無ければ従来どおりエラー。空文字・空白のみの env は「未設定」扱い
    /// （＝ config へ委ね、config も無ければエラー）。
    #[test]
    fn test_error_when_neither_env_nor_config() {
        let _guard = ENV_LOCK.lock().unwrap();
        // 空白のみの env は無視され、config も無いのでエラー。
        std::env::set_var(SECRET_KEY_ENV, "   ");
        let config = NostaroConfig::default();
        let result = keys_from_config(&config);
        std::env::remove_var(SECRET_KEY_ENV);
        assert!(
            result.is_err(),
            "空白のみ env + config 無しはエラーであるべき"
        );
    }
}
