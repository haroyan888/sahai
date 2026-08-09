//! 実行時に変更可能な設定。
//!
//! `Config`(config.rs)はプロセス起動に必須のブートストラップ値(bind_addr・
//! DBファイルパス等)のみを持つのに対し、こちらはWeb UIの設定画面から
//! 保存後すぐに変更できる値をDBに永続化して保持する。`Arc<RwLock<Settings>>`として
//! AppStateInner・RouteWriter・ComposeRuntime等の間で共有し、各利用箇所は
//! 呼び出しのたびに`.read().await`で最新値を読む(構築時に固定値をコピーしない)。

use std::sync::Arc;

use tokio::sync::RwLock;

use crate::repo::settings::SettingsRow;

#[derive(Debug, Clone)]
pub struct Settings {
    pub domain: String,
    pub https_redirect: bool,
    pub registry_url: String,
    pub api_token: String,
    pub dns_provider: String,
    pub acme_email: String,
    /// sahai service create(サーバー側build+push)専用のレジストリ資格情報。
    /// Web UIの「レジストリ設定」カードから編集できる。利用者がローカルで
    /// `docker login`する`container push`とは別の資格情報ストア。
    pub registry_username: Option<String>,
    pub registry_password: Option<String>,
}

pub type SharedSettings = Arc<RwLock<Settings>>;

impl From<SettingsRow> for Settings {
    fn from(row: SettingsRow) -> Self {
        Settings {
            domain: row.domain,
            https_redirect: row.https_redirect,
            registry_url: row.registry_url,
            api_token: row.api_token,
            dns_provider: row.dns_provider,
            acme_email: row.acme_email,
            registry_username: row.registry_username,
            registry_password: row.registry_password,
        }
    }
}

impl From<&Settings> for SettingsRow {
    fn from(s: &Settings) -> Self {
        SettingsRow {
            domain: s.domain.clone(),
            https_redirect: s.https_redirect,
            registry_url: s.registry_url.clone(),
            api_token: s.api_token.clone(),
            dns_provider: s.dns_provider.clone(),
            acme_email: s.acme_email.clone(),
            registry_username: s.registry_username.clone(),
            registry_password: s.registry_password.clone(),
        }
    }
}

impl Settings {
    /// 環境変数から初期値を組み立てる。DBがまだ空の初回起動時のみ使う
    /// (移行元の`.env`ベースの既存デプロイをそのまま引き継ぐため)。
    /// `domain`・`dns_provider`はハードコードした既定値を持たない(
    /// 特定のDNSプロバイダに固定しない方針)。未設定時は空文字列のまま環境変数として
    /// 渡ってくる(`.env`未設定のキーはdockerが値なしのKEY=として渡すため、この関数の
    /// 中では「未設定」と「空文字列が設定されている」を区別できない。`std::env::var`は
    /// キー自体が存在しない場合のみErrを返す)。そのため`domain`の空チェックはこの関数の
    /// 呼び出し元(main.rs)で`api_token`と同様に行う。
    pub fn seed_from_env() -> Result<Self, String> {
        let api_token = std::env::var("SAHAI_API_TOKEN")
            .map_err(|_| "環境変数 SAHAI_API_TOKEN が設定されていません".to_string())?;
        let domain = std::env::var("SAHAI_DOMAIN").unwrap_or_default();
        let https_redirect = std::env::var("SAHAI_HTTPS_REDIRECT")
            .map(|v| v != "false")
            .unwrap_or(true);
        let registry_url = std::env::var("SAHAI_REGISTRY_URL")
            .unwrap_or_else(|_| format!("registry.sahai.{domain}"));
        let dns_provider = std::env::var("SAHAI_DNS_PROVIDER").unwrap_or_default();
        let acme_email = std::env::var("SAHAI_ACME_EMAIL").unwrap_or_default();
        // SAHAI_REGISTRY_USERNAME/PASSWORDはここでのみ読む(初回シード専用)。
        // 以後はWeb UIの「レジストリ設定」カードから編集した値がDBの正となり、
        // これらの環境変数は無視される。空文字列は「未設定」として扱う
        // (domainの空チェックと同じ理由。dockerが値なしのKEY=を渡すため)
        let registry_username = std::env::var("SAHAI_REGISTRY_USERNAME")
            .ok()
            .filter(|s| !s.is_empty());
        let registry_password = std::env::var("SAHAI_REGISTRY_PASSWORD")
            .ok()
            .filter(|s| !s.is_empty());

        Ok(Settings {
            domain,
            https_redirect,
            registry_url,
            api_token,
            dns_provider,
            acme_email,
            registry_username,
            registry_password,
        })
    }

    /// 環境変数からも引き継げず、DBにも行が無い初回起動時のプレースホルダー。
    /// api_tokenが空である状態を「初期セットアップ未完了」の判定に使う
    /// (service::settings::is_configured参照)。この状態のままではdomainが空のため
    /// 管理画面用Traefikルートは書き出さない(main.rs参照)。
    pub fn unconfigured() -> Self {
        Settings {
            domain: String::new(),
            https_redirect: true,
            registry_url: String::new(),
            api_token: String::new(),
            dns_provider: String::new(),
            acme_email: String::new(),
            registry_username: None,
            registry_password: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_row_round_trips_through_settings() {
        let row = SettingsRow {
            domain: "example.com".to_string(),
            https_redirect: false,
            registry_url: "registry.sahai.example.com".to_string(),
            api_token: "tok".to_string(),
            dns_provider: "route53".to_string(),
            acme_email: "admin@example.com".to_string(),
            registry_username: Some("reguser".to_string()),
            registry_password: Some("regpass".to_string()),
        };
        let settings: Settings = row.clone().into();
        let round_tripped: SettingsRow = (&settings).into();

        assert_eq!(round_tripped.domain, row.domain);
        assert_eq!(round_tripped.https_redirect, row.https_redirect);
        assert_eq!(round_tripped.registry_url, row.registry_url);
        assert_eq!(round_tripped.api_token, row.api_token);
        assert_eq!(round_tripped.dns_provider, row.dns_provider);
        assert_eq!(round_tripped.acme_email, row.acme_email);
        assert_eq!(round_tripped.registry_username, row.registry_username);
        assert_eq!(round_tripped.registry_password, row.registry_password);
    }
}
