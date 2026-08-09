//! `~/.config/sahai/config.toml` の読み書き。

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct CliConfig {
    #[serde(default)]
    pub control_plane: ControlPlaneConfig,
    #[serde(default)]
    pub registry: RegistryConfig,
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct ControlPlaneConfig {
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub token: String,
    /// TLS証明書検証をスキップするか(既定false)。ローカルなドメイン
    /// (例: SAHAI_DOMAIN=localhost)でのテストなど、DNS-01証明書発行ができず
    /// 自己署名証明書のままの環境向け。config.tomlに`insecure = true`を
    /// 手動で追記することで永続設定できる(実運用では使うべきではない)。
    #[serde(default)]
    pub insecure: bool,
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct RegistryConfig {
    #[serde(default)]
    pub url: String,
}

pub fn config_path() -> Result<PathBuf, String> {
    let home = dirs::home_dir().ok_or_else(|| "ホームディレクトリが見つかりません".to_string())?;
    Ok(home.join(".config").join("sahai").join("config.toml"))
}

impl CliConfig {
    pub fn load() -> Result<Self, String> {
        let path = config_path()?;
        if !path.exists() {
            return Err(format!(
                "設定ファイルが見つかりません: {}\n`sahai login` を先に実行してください",
                path.display()
            ));
        }
        let content = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
        toml::from_str(&content).map_err(|e| e.to_string())
    }

    pub fn save(&self) -> Result<(), String> {
        let path = config_path()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let content = toml::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(&path, content).map_err(|e| e.to_string())
    }
}
