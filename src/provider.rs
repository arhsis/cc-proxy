use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

/// Platform-specific configuration (apiUrl + apiKey)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlatformConfig {
    #[serde(rename = "apiUrl")]
    pub api_url: String,
    #[serde(rename = "apiKey")]
    pub api_key: String,
}

/// Provider with platform-specific configs
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Provider {
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub level: i32,
    pub name: Option<String>,
    #[serde(rename = "apiUrl")]
    pub api_url: Option<String>,
    #[serde(rename = "apiKey")]
    pub api_key: Option<String>,
    pub codex: Option<PlatformConfig>,
    pub claude: Option<PlatformConfig>,
}

impl Provider {
    /// Get platform-specific config for this provider
    pub fn get_platform_config(&self, kind: &str) -> Option<PlatformConfig> {
        let platform_config = match kind {
            "codex" => self.codex.clone(),
            "claude" => self.claude.clone(),
            _ => None,
        };

        if platform_config.is_some() {
            return platform_config;
        }

        // Backward-compatible fallback to a single shared config
        match (&self.api_url, &self.api_key) {
            (Some(url), Some(key)) if !url.is_empty() && !key.is_empty() => Some(PlatformConfig {
                api_url: url.clone(),
                api_key: key.clone(),
            }),
            _ => None,
        }
    }
}

fn default_enabled() -> bool {
    true
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(untagged)]
pub enum PlatformConfigList {
    Single(PlatformConfig),
    List(Vec<PlatformConfig>),
}

impl PlatformConfigList {
    fn into_vec(self) -> Vec<PlatformConfig> {
        match self {
            PlatformConfigList::Single(cfg) => vec![cfg],
            PlatformConfigList::List(list) => list,
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ProviderMapConfig {
    pub codex: Option<PlatformConfigList>,
    pub claude: Option<PlatformConfigList>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(untagged)]
pub enum ProviderConfig {
    List { providers: Vec<Provider> },
    Map { providers: ProviderMapConfig },
}

/// Load providers from configuration file
pub fn load_providers() -> Result<Vec<Provider>> {
    let config_path = get_config_path()?;

    if !config_path.exists() {
        tracing::warn!("Provider config not found: {:?}", config_path);
        return Ok(Vec::new());
    }

    let content = fs::read_to_string(&config_path)
        .with_context(|| format!("Failed to read provider config: {:?}", config_path))?;

    let config: ProviderConfig = serde_json::from_str(&content)
        .with_context(|| format!("Failed to parse provider config: {:?}", config_path))?;

    let providers = match config {
        ProviderConfig::List { providers } => providers,
        ProviderConfig::Map { providers } => {
            let mut flattened = Vec::new();

            if let Some(codex_list) = providers.codex {
                for cfg in codex_list.into_vec() {
                    flattened.push(Provider {
                        enabled: default_enabled(),
                        level: 0,
                        name: None,
                        api_url: None,
                        api_key: None,
                        codex: Some(cfg),
                        claude: None,
                    });
                }
            }

            if let Some(claude_list) = providers.claude {
                for cfg in claude_list.into_vec() {
                    flattened.push(Provider {
                        enabled: default_enabled(),
                        level: 0,
                        name: None,
                        api_url: None,
                        api_key: None,
                        codex: None,
                        claude: Some(cfg),
                    });
                }
            }

            if flattened.is_empty() {
                anyhow::bail!("No providers defined in provider.json");
            }

            flattened
        }
    };

    Ok(providers)
}

/// Get configuration file path
pub fn get_config_path() -> Result<PathBuf> {
    let home = std::env::var("HOME").context("HOME environment variable not set")?;
    let dir = PathBuf::from(home).join(".cc-proxy");

    let new_path = dir.join("provider.json");
    let legacy_path = dir.join("providers.json");

    if new_path.exists() || !legacy_path.exists() {
        Ok(new_path)
    } else {
        Ok(legacy_path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_defaults_to_enabled() {
        let provider = Provider {
            enabled: default_enabled(),
            level: 0,
            name: Some("test".to_string()),
            api_url: None,
            api_key: None,
            codex: None,
            claude: None,
        };

        assert!(provider.enabled);
    }

    #[test]
    fn get_platform_config_returns_correct_platform() {
        let provider = Provider {
            enabled: true,
            level: 1,
            name: Some("test".to_string()),
            api_url: None,
            api_key: None,
            codex: Some(PlatformConfig {
                api_url: "https://codex.api.com".to_string(),
                api_key: "codex-key".to_string(),
            }),
            claude: Some(PlatformConfig {
                api_url: "https://claude.api.com".to_string(),
                api_key: "claude-key".to_string(),
            }),
        };

        let codex_config = provider.get_platform_config("codex").unwrap();
        assert_eq!(codex_config.api_url, "https://codex.api.com");

        let claude_config = provider.get_platform_config("claude").unwrap();
        assert_eq!(claude_config.api_url, "https://claude.api.com");
    }

    #[test]
    fn map_config_parses_minimal_platforms() {
        let json = r#"
        {
            "providers": {
                "codex": { "apiUrl": "https://codex.api", "apiKey": "ckey" },
                "claude": [
                    { "apiUrl": "https://claude.api", "apiKey": "akey" },
                    { "apiUrl": "https://claude2.api", "apiKey": "akey2" }
                ]
            }
        }
        "#;

        let config: ProviderConfig = serde_json::from_str(json).unwrap();
        let providers = match config {
            ProviderConfig::List { providers } => providers,
            ProviderConfig::Map { providers } => {
                let mut flattened = Vec::new();

                if let Some(codex) = providers.codex {
                    for cfg in codex.into_vec() {
                        flattened.push(Provider {
                            enabled: default_enabled(),
                            level: 0,
                            name: None,
                            api_url: None,
                            api_key: None,
                            codex: Some(cfg),
                            claude: None,
                        });
                    }
                }

                if let Some(claude) = providers.claude {
                    for cfg in claude.into_vec() {
                        flattened.push(Provider {
                            enabled: default_enabled(),
                            level: 0,
                            name: None,
                            api_url: None,
                            api_key: None,
                            codex: None,
                            claude: Some(cfg),
                        });
                    }
                }

                flattened
            }
        };

        assert_eq!(providers.len(), 3);
        let provider = &providers[0];
        assert!(provider.codex.is_some());
        assert!(provider.claude.is_none());
        assert_eq!(
            providers[1].claude.as_ref().unwrap().api_url,
            "https://claude.api"
        );
        assert_eq!(
            providers[2].claude.as_ref().unwrap().api_url,
            "https://claude2.api"
        );
    }

    #[test]
    fn get_platform_config_falls_back_to_shared_keys() {
        let provider = Provider {
            enabled: true,
            level: 0,
            name: None,
            api_url: Some("https://shared.api.com".to_string()),
            api_key: Some("shared-key".to_string()),
            codex: None,
            claude: None,
        };

        let codex_config = provider.get_platform_config("codex").unwrap();
        assert_eq!(codex_config.api_url, "https://shared.api.com");

        let claude_config = provider.get_platform_config("claude").unwrap();
        assert_eq!(claude_config.api_url, "https://shared.api.com");
    }
}
