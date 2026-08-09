//! 設定画面(Web UI)用エンドポイント。基本設定・DNS/証明書・レジストリで
//! 保存時の副作用が大きく異なるため、エンドポイントを分けている。

use axum::extract::State;
use axum::response::IntoResponse;
use axum::Json;
use serde::Serialize;

use crate::api::dto::{UpdateDnsConfigRequest, UpdateRegistryConfigRequest, UpdateSettingsRequest};
use crate::error::AppError;
use crate::service;
use crate::service::settings::{DnsConfig, RegistryConfig};
use crate::settings::Settings;
use crate::state::AppState;

#[derive(Serialize)]
struct DnsCredentialOutput {
    key: String,
    value: String,
}

#[derive(Serialize)]
struct DnsConfigResponse {
    dns_provider: String,
    acme_email: String,
    credentials: Vec<DnsCredentialOutput>,
}

impl From<DnsConfig> for DnsConfigResponse {
    fn from(c: DnsConfig) -> Self {
        DnsConfigResponse {
            dns_provider: c.dns_provider,
            acme_email: c.acme_email,
            credentials: c
                .credentials
                .into_iter()
                .map(|(key, value)| DnsCredentialOutput { key, value })
                .collect(),
        }
    }
}

/// 「基本設定」カードのレスポンス。`Settings`をそのまま返すと`dns_provider`/
/// `registry_url`/`registry_username`/`registry_password`(パスワードを含む)まで
/// 漏れてしまうため、DNS設定・レジストリ設定と同様に専用のレスポンス型で絞り込む。
#[derive(Serialize)]
pub(crate) struct BasicSettingsResponse {
    domain: String,
    https_redirect: bool,
    api_token: String,
}

impl From<Settings> for BasicSettingsResponse {
    fn from(s: Settings) -> Self {
        BasicSettingsResponse {
            domain: s.domain,
            https_redirect: s.https_redirect,
            api_token: s.api_token,
        }
    }
}

#[derive(Serialize)]
struct RegistryConfigResponse {
    registry_url: String,
    registry_username: Option<String>,
    registry_password: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    login_warning: Option<String>,
}

impl From<RegistryConfig> for RegistryConfigResponse {
    fn from(c: RegistryConfig) -> Self {
        RegistryConfigResponse {
            registry_url: c.registry_url,
            registry_username: c.registry_username,
            registry_password: c.registry_password,
            login_warning: c.login_warning,
        }
    }
}

pub async fn get(State(state): State<AppState>) -> impl IntoResponse {
    Json(BasicSettingsResponse::from(
        service::settings::get(&state).await,
    ))
}

pub async fn update(
    State(state): State<AppState>,
    Json(req): Json<UpdateSettingsRequest>,
) -> Result<impl IntoResponse, AppError> {
    // dns_provider/acme_email/registry_url/registry_username/registry_passwordは
    // この画面では変更できない(DNS/証明書設定・レジストリ設定の専用画面経由でのみ
    // 変更する。dto.rs::UpdateSettingsRequest参照)。既存の値をそのまま引き継ぐ
    let (dns_provider, acme_email, registry_url, registry_username, registry_password) = {
        let current = state.settings.read().await;
        (
            current.dns_provider.clone(),
            current.acme_email.clone(),
            current.registry_url.clone(),
            current.registry_username.clone(),
            current.registry_password.clone(),
        )
    };
    let new_settings = Settings {
        domain: req.domain,
        https_redirect: req.https_redirect,
        registry_url,
        api_token: req.api_token,
        dns_provider,
        acme_email,
        registry_username,
        registry_password,
    };
    let saved = service::settings::update(&state, new_settings).await?;
    Ok(Json(BasicSettingsResponse::from(saved)))
}

pub async fn get_dns_config(State(state): State<AppState>) -> Result<impl IntoResponse, AppError> {
    let config = service::settings::get_dns_config(&state).await?;
    Ok(Json(DnsConfigResponse::from(config)))
}

pub async fn update_dns_config(
    State(state): State<AppState>,
    Json(req): Json<UpdateDnsConfigRequest>,
) -> Result<impl IntoResponse, AppError> {
    let new_config = DnsConfig {
        dns_provider: req.dns_provider,
        acme_email: req.acme_email,
        credentials: req
            .credentials
            .into_iter()
            .map(|c| (c.key, c.value))
            .collect(),
    };
    let saved = service::settings::update_dns_config(&state, new_config).await?;
    Ok(Json(DnsConfigResponse::from(saved)))
}

pub async fn get_registry_config(State(state): State<AppState>) -> impl IntoResponse {
    Json(RegistryConfigResponse::from(
        service::settings::get_registry_config(&state).await,
    ))
}

pub async fn update_registry_config(
    State(state): State<AppState>,
    Json(req): Json<UpdateRegistryConfigRequest>,
) -> Result<impl IntoResponse, AppError> {
    let new_config = RegistryConfig {
        registry_url: req.registry_url,
        registry_username: req.registry_username,
        registry_password: req.registry_password,
        login_warning: None,
    };
    let saved = service::settings::update_registry_config(&state, new_config).await?;
    Ok(Json(RegistryConfigResponse::from(saved)))
}
