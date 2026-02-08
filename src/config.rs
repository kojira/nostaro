use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NostaroConfig {
    pub secret_key: Option<String>,
    pub relays: Vec<String>,
    pub default_relays: Vec<String>,
}

impl Default for NostaroConfig {
    fn default() -> Self {
        let default_relays = vec![
            "wss://relay.damus.io".to_string(),
            "wss://nos.lol".to_string(),
            "wss://relay.nostr.band".to_string(),
            "wss://r.kojira.io".to_string(),
        ];
        Self {
            secret_key: None,
            relays: Vec::new(),
            default_relays,
        }
    }
}

impl NostaroConfig {
    pub fn config_dir() -> PathBuf {
        dirs::home_dir()
            .expect("Could not find home directory")
            .join(".nostaro")
    }

    pub fn config_path() -> PathBuf {
        Self::config_dir().join("config.toml")
    }

    pub fn load() -> Result<Self> {
        Self::load_from(&Self::config_path())
    }

    pub fn load_from(path: &std::path::Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let content = std::fs::read_to_string(path)?;
        let config: NostaroConfig = toml::from_str(&content)?;
        Ok(config)
    }

    pub fn save(&self) -> Result<()> {
        self.save_to(&Self::config_path())
    }

    pub fn save_to(&self, path: &std::path::Path) -> Result<()> {
        if let Some(dir) = path.parent() {
            if !dir.exists() {
                std::fs::create_dir_all(dir)?;
            }
        }
        let content = toml::to_string_pretty(self)?;
        std::fs::write(path, content)?;
        Ok(())
    }

    pub fn active_relays(&self) -> Vec<String> {
        if self.relays.is_empty() {
            self.default_relays.clone()
        } else {
            self.relays.clone()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = NostaroConfig::default();
        assert!(config.secret_key.is_none());
        assert!(config.relays.is_empty());
        assert_eq!(config.default_relays.len(), 4);
        assert!(config.default_relays.contains(&"wss://r.kojira.io".to_string()));
    }

    #[test]
    fn test_active_relays_uses_defaults_when_empty() {
        let config = NostaroConfig::default();
        let active = config.active_relays();
        assert_eq!(active, config.default_relays);
    }

    #[test]
    fn test_active_relays_uses_custom_when_set() {
        let mut config = NostaroConfig::default();
        config.relays = vec!["wss://custom.relay".to_string()];
        let active = config.active_relays();
        assert_eq!(active, vec!["wss://custom.relay"]);
    }

    #[test]
    fn test_config_serialization_roundtrip() {
        let config = NostaroConfig {
            secret_key: Some("nsec1test".to_string()),
            relays: vec!["wss://relay.example.com".to_string()],
            default_relays: vec!["wss://default.relay".to_string()],
        };
        let serialized = toml::to_string_pretty(&config).unwrap();
        let deserialized: NostaroConfig = toml::from_str(&serialized).unwrap();
        assert_eq!(deserialized.secret_key, config.secret_key);
        assert_eq!(deserialized.relays, config.relays);
        assert_eq!(deserialized.default_relays, config.default_relays);
    }

    #[test]
    fn test_save_and_load_from_file() {
        let dir = std::env::temp_dir().join("nostaro_test_config_v2");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("config.toml");

        let config = NostaroConfig {
            secret_key: Some("nsec1testkey".to_string()),
            relays: vec!["wss://relay.test.com".to_string()],
            default_relays: vec!["wss://default.test.com".to_string()],
        };
        config.save_to(&path).unwrap();

        let loaded = NostaroConfig::load_from(&path).unwrap();
        assert_eq!(loaded.secret_key, config.secret_key);
        assert_eq!(loaded.relays, config.relays);
        assert_eq!(loaded.default_relays, config.default_relays);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_load_from_nonexistent_returns_default() {
        let path = std::env::temp_dir().join("nostaro_nonexistent_config_v2.toml");
        let loaded = NostaroConfig::load_from(&path).unwrap();
        assert!(loaded.secret_key.is_none());
        assert!(loaded.relays.is_empty());
    }
}
