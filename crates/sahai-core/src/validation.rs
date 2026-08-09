//! サービス名・composeサービス名・ホストポートのバリデーション。
//! sahai-serverとsahai-cliで同じ規則を使うため、このクレートに集約する。

use crate::error::CoreError;

/// サービス名の長さ制約。
pub const SERVICE_NAME_MIN_LEN: usize = 2;
pub const SERVICE_NAME_MAX_LEN: usize = 63;

/// サービス名として使えない予約語。管理画面の`sahai.<domain>`とサブドメインが
/// 衝突するため。レジストリは`registry.sahai.<domain>`にあり、サービスの
/// `<name>.<domain>`とは階層が違うので予約しない。
/// 完全一致のみ拒否する(`sahai-app`のように含むだけの名前は許可)。
pub const RESERVED_SERVICE_NAMES: &[&str] = &["sahai"];

/// レジストリタグ(`<service-name>[-composeサービス名]`)の長さ上限。
pub const MAX_REGISTRY_TAG_LEN: usize = 128;

/// 差配自身がホストに公開しているポート。ここを奪われると差配全体が停止するため、
/// サービスのhost_portには使わせない(compose.yamlのtraefikのports指定と対応)。
pub const RESERVED_HOST_PORTS: [u16; 2] = [80, 443];

/// サービス名を検証する: `^[a-z][a-z0-9-]{0,61}[a-z0-9]$` 相当、2〜63文字。
pub fn validate_service_name(name: &str) -> Result<(), CoreError> {
    let chars: Vec<char> = name.chars().collect();
    let len = chars.len();

    if !(SERVICE_NAME_MIN_LEN..=SERVICE_NAME_MAX_LEN).contains(&len) {
        return Err(CoreError::Validation(format!(
            "サービス名は{}〜{}文字で指定してください(現在{}文字)",
            SERVICE_NAME_MIN_LEN, SERVICE_NAME_MAX_LEN, len
        )));
    }

    let first = chars[0];
    if !first.is_ascii_lowercase() {
        return Err(CoreError::Validation(
            "サービス名は英小文字で始まる必要があります".to_string(),
        ));
    }

    let last = chars[len - 1];
    if !(last.is_ascii_lowercase() || last.is_ascii_digit()) {
        return Err(CoreError::Validation(
            "サービス名は英小文字または数字で終わる必要があります".to_string(),
        ));
    }

    for c in &chars[1..len - 1] {
        if !(c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '-') {
            return Err(CoreError::Validation(format!(
                "サービス名に使用できない文字が含まれています: '{}'",
                c
            )));
        }
    }

    if RESERVED_SERVICE_NAMES.contains(&name) {
        return Err(CoreError::Validation(format!(
            "'{name}'は予約されているため使用できません"
        )));
    }

    Ok(())
}

/// composeサービス名を検証する: Dockerリポジトリ名として有効な文字([a-z0-9._-])のみ。
pub fn validate_compose_service_name(name: &str) -> Result<(), CoreError> {
    if name.is_empty() {
        return Err(CoreError::Validation(
            "composeサービス名が空です".to_string(),
        ));
    }
    for c in name.chars() {
        if !(c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-')) {
            return Err(CoreError::Validation(format!(
                "composeサービス名 '{name}' に使用できない文字が含まれています: '{c}'"
            )));
        }
    }
    Ok(())
}

/// 合成タグ(`<service-name>-<composeサービス名>`)の長さが128文字以内かを検証する。
pub fn validate_registry_tag_length(tag: &str) -> Result<(), CoreError> {
    let len = tag.chars().count();
    if len > MAX_REGISTRY_TAG_LEN {
        return Err(CoreError::Validation(format!(
            "合成タグ '{tag}' が{MAX_REGISTRY_TAG_LEN}文字を超えています({len}文字)"
        )));
    }
    Ok(())
}

/// host_portが単体で成立する値かを検証する。範囲の制限は設けず、
/// ポート番号として無効な値と差配自身の予約ポートだけを弾く。
/// 他サービスとの重複はDBを見ないと判断できないため、ここでは扱わない。
pub fn validate_host_port(port: i64) -> Result<(), CoreError> {
    // 0はDockerでは「空きポートを自動選択」の意味になり、手動指定の前提から外れる
    if !(1..=65535).contains(&port) {
        return Err(CoreError::Validation(format!(
            "ホストポートは1〜65535で指定してください(現在{port})"
        )));
    }
    if RESERVED_HOST_PORTS.contains(&(port as u16)) {
        return Err(CoreError::Validation(format!(
            "ポート{port}は差配自身が使用しているため指定できません"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn service_name_accepts_minimal_valid_names() {
        assert!(validate_service_name("ab").is_ok());
        assert!(validate_service_name("a1").is_ok());
        assert!(validate_service_name("my-app-2").is_ok());
    }

    #[test]
    fn service_name_rejects_too_short_or_too_long() {
        assert!(validate_service_name("a").is_err());
        let too_long = "a".repeat(64);
        assert!(validate_service_name(&too_long).is_err());
        let max_len = format!("a{}a", "b".repeat(61));
        assert_eq!(max_len.len(), 63);
        assert!(validate_service_name(&max_len).is_ok());
    }

    #[test]
    fn service_name_rejects_bad_first_or_last_char() {
        assert!(validate_service_name("1abc").is_err());
        assert!(validate_service_name("abc-").is_err());
        assert!(validate_service_name("-abc").is_err());
        assert!(validate_service_name("ABC").is_err());
    }

    #[test]
    fn service_name_rejects_reserved_names() {
        assert!(validate_service_name("sahai").is_err());
    }

    /// レジストリはsahai配下へ移したため、registryはサービス名に使える。
    #[test]
    fn service_name_allows_registry() {
        assert!(validate_service_name("registry").is_ok());
    }

    #[test]
    fn service_name_allows_names_only_containing_a_reserved_word() {
        // 予約語は完全一致のみ拒否する。"sahai-app"のサブドメインは
        // "sahai-app.<domain>"となり、管理画面用の"sahai.<domain>"とは衝突しない
        assert!(validate_service_name("sahai-app").is_ok());
        assert!(validate_service_name("my-registry").is_ok());
    }

    #[test]
    fn compose_service_name_allows_dot_underscore_hyphen() {
        assert!(validate_compose_service_name("my.app_1-x").is_ok());
        assert!(validate_compose_service_name("").is_err());
        assert!(validate_compose_service_name("My-App").is_err());
        assert!(validate_compose_service_name("app/name").is_err());
    }

    #[test]
    fn registry_tag_length_boundary() {
        let ok = "a".repeat(128);
        assert!(validate_registry_tag_length(&ok).is_ok());
        let too_long = "a".repeat(129);
        assert!(validate_registry_tag_length(&too_long).is_err());
    }

    #[test]
    fn host_port_boundary() {
        assert!(validate_host_port(1).is_ok());
        assert!(validate_host_port(65535).is_ok());
        assert!(validate_host_port(0).is_err());
        assert!(validate_host_port(65536).is_err());
        assert!(validate_host_port(-1).is_err());
    }

    #[test]
    fn host_port_rejects_ports_used_by_sahai_itself() {
        for port in RESERVED_HOST_PORTS {
            let err = validate_host_port(i64::from(port)).unwrap_err().to_string();
            assert!(err.contains("差配自身"), "{port}: {err}");
        }
    }

    /// 範囲による制限は設けていない。特定の帯に限定する変更が入れば落ちる。
    #[test]
    fn host_port_is_not_limited_to_a_particular_band() {
        for port in [22, 3000, 8080, 19999, 30000, 50000] {
            assert!(validate_host_port(port).is_ok(), "{port}");
        }
    }
}
