use std::io::Write;

use crate::config::{CliConfig, ControlPlaneConfig, RegistryConfig};

/// Control Plane APIのBearerトークンを対話的に入力させ、設定ファイルに保存する。
pub fn run() -> Result<(), String> {
    let url = prompt("Control Plane URL (例: https://admin.example.com): ")?;
    let token = prompt("Bearer token: ")?;
    let registry_url = prompt("Registry URL (例: registry.sahai.example.com): ")?;

    let config = CliConfig {
        control_plane: ControlPlaneConfig {
            url,
            token,
            insecure: false,
        },
        registry: RegistryConfig { url: registry_url },
    };
    config.save()?;
    println!("設定を保存しました。");
    Ok(())
}

fn prompt(label: &str) -> Result<String, String> {
    print!("{label}");
    std::io::stdout().flush().map_err(|e| e.to_string())?;
    let mut input = String::new();
    std::io::stdin()
        .read_line(&mut input)
        .map_err(|e| e.to_string())?;
    Ok(input.trim().to_string())
}
