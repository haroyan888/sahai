use crate::config::{config_path, CliConfig};

/// 設定ファイルのパスと、設定可能な全項目・現在値を表示する。
/// 値の編集機能は持たない(変更は`sahai login`か、エディタで直接編集する)。
pub fn run() -> Result<(), String> {
    let path = config_path()?;
    let config = CliConfig::load()?;

    println!("設定ファイル: {}", path.display());
    println!();
    println!("[control_plane]");
    println!(
        "  url      = {}",
        display_or_unset(&config.control_plane.url)
    );
    // トークンは秘匿値のため、設定済みかどうかだけが分かればよい
    println!("  token    = {}", mask_token(&config.control_plane.token));
    println!("  insecure = {}", config.control_plane.insecure);
    println!();
    println!("[registry]");
    println!("  url      = {}", display_or_unset(&config.registry.url));

    Ok(())
}

fn display_or_unset(value: &str) -> String {
    if value.is_empty() {
        "(未設定)".to_string()
    } else {
        value.to_string()
    }
}

fn mask_token(token: &str) -> String {
    if token.is_empty() {
        "(未設定)".to_string()
    } else {
        format!("(設定済み、{}文字)", token.chars().count())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unset_values_are_shown_as_placeholder() {
        assert_eq!(display_or_unset(""), "(未設定)");
        assert_eq!(
            display_or_unset("https://sahai.example.com"),
            "https://sahai.example.com"
        );
    }

    /// トークンは値そのものを画面・ログに出さない。
    #[test]
    fn token_value_is_never_printed() {
        let masked = mask_token("super-secret-token");
        assert!(!masked.contains("super-secret-token"));
        assert!(masked.contains("18文字"));
    }

    #[test]
    fn empty_token_is_reported_as_unset() {
        assert_eq!(mask_token(""), "(未設定)");
    }
}
