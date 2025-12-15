use anyhow::{Context, Result};
use serde_json::{Map as JsonMap, Value as JsonValue};
use std::fs;
use std::path::{Path, PathBuf};
use toml::value::Table as TomlTable;

/// Configure Claude Code to use the proxy
pub fn configure_claude(proxy_addr: &str) -> Result<()> {
    let home = std::env::var("HOME").context("HOME environment variable not set")?;
    let settings_path = PathBuf::from(home).join(".claude").join("settings.json");

    // Create directory if needed
    if let Some(parent) = settings_path.parent() {
        fs::create_dir_all(parent).context("Failed to create .claude directory")?;
    }

    let mut settings = load_json_object(&settings_path, "Claude settings")?;
    let mut env_map = match settings.remove("env") {
        Some(JsonValue::Object(map)) => map,
        Some(_) => {
            tracing::warn!(
                "Existing Claude settings 'env' field is not an object; overwriting managed keys"
            );
            JsonMap::new()
        }
        None => JsonMap::new(),
    };

    env_map.insert(
        "ANTHROPIC_AUTH_TOKEN".into(),
        JsonValue::String("cc-proxy".into()),
    );
    env_map.insert(
        "ANTHROPIC_BASE_URL".into(),
        JsonValue::String(format!("http://{}", proxy_addr)),
    );
    settings.insert("env".into(), JsonValue::Object(env_map));

    // Write settings
    fs::write(
        &settings_path,
        serde_json::to_string_pretty(&JsonValue::Object(settings))?,
    )
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

    let config_path = codex_dir.join("config.toml");
    let mut config_table = load_toml_table(&config_path, "Codex config.toml")?;
    config_table.insert(
        "preferred_auth_method".into(),
        toml::Value::String("apikey".into()),
    );
    config_table.insert("model".into(), toml::Value::String("gpt-5-codex".into()));
    config_table.insert(
        "model_provider".into(),
        toml::Value::String("cc-proxy".into()),
    );

    let providers_table = config_table
        .entry(String::from("model_providers"))
        .or_insert_with(|| toml::Value::Table(TomlTable::new()));
    let providers_table = providers_table
        .as_table_mut()
        .ok_or_else(|| anyhow::anyhow!("model_providers is not a table"))?;

    let cc_provider = providers_table
        .entry(String::from("cc-proxy"))
        .or_insert_with(|| toml::Value::Table(TomlTable::new()));
    let cc_provider = cc_provider
        .as_table_mut()
        .ok_or_else(|| anyhow::anyhow!("model_providers.cc-proxy is not a table"))?;
    cc_provider.insert("name".into(), toml::Value::String("cc-proxy".into()));
    cc_provider.insert(
        "base_url".into(),
        toml::Value::String(format!("http://{}", proxy_addr)),
    );
    cc_provider.insert(
        "env_key".into(),
        toml::Value::String("OPENAI_API_KEY".into()),
    );
    cc_provider.insert("wire_api".into(), toml::Value::String("responses".into()));
    cc_provider.insert("requires_openai_auth".into(), toml::Value::Boolean(false));

    let config_toml = toml::to_string_pretty(&config_table)
        .context("Failed to serialize Codex config to TOML")?;
    fs::write(&config_path, config_toml).context("Failed to write Codex config.toml")?;

    // Write auth.json
    let auth_path = codex_dir.join("auth.json");
    let mut auth = load_json_object(&auth_path, "Codex auth.json")?;
    auth.insert(
        "OPENAI_API_KEY".into(),
        JsonValue::String("cc-proxy".into()),
    );
    fs::write(
        &auth_path,
        serde_json::to_string_pretty(&JsonValue::Object(auth))?,
    )
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

fn load_json_object(path: &Path, description: &str) -> Result<JsonMap<String, JsonValue>> {
    if !path.exists() {
        return Ok(JsonMap::new());
    }

    let raw =
        fs::read_to_string(path).with_context(|| format!("Failed to read {}", description))?;
    if raw.trim().is_empty() {
        return Ok(JsonMap::new());
    }

    match serde_json::from_str::<JsonValue>(&raw) {
        Ok(JsonValue::Object(map)) => Ok(map),
        Ok(_) => {
            tracing::warn!(
                "{} is not a JSON object; managed fields will be reinitialized",
                description
            );
            Ok(JsonMap::new())
        }
        Err(err) => {
            tracing::warn!("Failed to parse {}: {}", description, err);
            Ok(JsonMap::new())
        }
    }
}

fn load_toml_table(path: &Path, description: &str) -> Result<TomlTable> {
    if !path.exists() {
        return Ok(TomlTable::new());
    }

    let raw =
        fs::read_to_string(path).with_context(|| format!("Failed to read {}", description))?;
    if raw.trim().is_empty() {
        return Ok(TomlTable::new());
    }

    match raw.parse::<toml::Value>() {
        Ok(toml::Value::Table(table)) => Ok(table),
        Ok(_) => {
            tracing::warn!(
                "{} is not a TOML table; managed fields will be reinitialized",
                description
            );
            Ok(TomlTable::new())
        }
        Err(err) => {
            tracing::warn!("Failed to parse {}: {}", description, err);
            Ok(TomlTable::new())
        }
    }
}
