use anyhow::{Context, Result};
use serde_json::json;
use std::fs;
use std::path::PathBuf;

/// Configure Claude Code to use the proxy
pub fn configure_claude(proxy_addr: &str) -> Result<()> {
    let home = std::env::var("HOME").context("HOME environment variable not set")?;
    let settings_path = PathBuf::from(home).join(".claude").join("settings.json");

    // Create directory if needed
    if let Some(parent) = settings_path.parent() {
        fs::create_dir_all(parent).context("Failed to create .claude directory")?;
    }

    // Create settings JSON
    let settings = json!({
        "env": {
            "ANTHROPIC_AUTH_TOKEN": "cc-proxy",
            "ANTHROPIC_BASE_URL": format!("http://{}", proxy_addr)
        }
    });

    // Write settings
    fs::write(&settings_path, serde_json::to_string_pretty(&settings)?)
        .context("Failed to write Claude settings")?;

    tracing::info!("✓ Claude Code configured: {:?}", settings_path);
    Ok(())
}

/// Configure Codex to use the proxy
pub fn configure_codex(proxy_addr: &str) -> Result<()> {
    let home = std::env::var("HOME").context("HOME environment variable not set")?;
    let codex_dir = PathBuf::from(home).join(".codex");

    // Create directory if needed
    fs::create_dir_all(&codex_dir).context("Failed to create .codex directory")?;

    // Write config.toml
    let config_path = codex_dir.join("config.toml");
    let config_toml = format!(
        r#"preferred_auth_method = "apikey"
model = "gpt-5-codex"
model_provider = "cc-proxy"

[model_providers.cc-proxy]
name = "cc-proxy"
base_url = "http://{}"
env_key = "OPENAI_API_KEY"
wire_api = "responses"
requires_openai_auth = false
"#,
        proxy_addr
    );

    fs::write(&config_path, config_toml).context("Failed to write Codex config.toml")?;

    // Write auth.json
    let auth_path = codex_dir.join("auth.json");
    let auth_json = json!({
        "OPENAI_API_KEY": "cc-proxy"
    });

    fs::write(&auth_path, serde_json::to_string_pretty(&auth_json)?)
        .context("Failed to write Codex auth.json")?;

    tracing::info!("✓ Codex configured: {:?}", codex_dir);
    Ok(())
}

/// Configure both Claude Code and Codex
pub fn configure_all(proxy_addr: &str) -> Result<()> {
    configure_claude(proxy_addr)?;
    configure_codex(proxy_addr)?;
    tracing::info!("✓ All CLI tools configured to use proxy at {}", proxy_addr);
    Ok(())
}
