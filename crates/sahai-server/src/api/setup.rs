//! 初回セットアップ用エンドポイント。
//! トークンがまだ存在しない段階で叩く必要があるため認証層の外側に登録する(api/mod.rs参照)。
//! 代わりに`create`は、起動時に発行されるセットアップトークン(`setup_token`)の提示を
//! 必須とし、初期設定を第三者に先取りされないようにする。
//! 設定済みの場合は`service::settings::setup`側が重ねて拒否する。

use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::IntoResponse;
use axum::Json;
use serde::Serialize;

use crate::api::dto::UpdateSettingsRequest;
use crate::api::settings::BasicSettingsResponse;
use crate::error::AppError;
use crate::service;
use crate::settings::Settings;
use crate::state::AppState;

#[derive(Serialize)]
pub struct SetupStatus {
    configured: bool,
}

pub async fn status(State(state): State<AppState>) -> impl IntoResponse {
    Json(SetupStatus {
        configured: service::settings::is_configured(&state).await,
    })
}

pub async fn create(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<UpdateSettingsRequest>,
) -> Result<impl IntoResponse, AppError> {
    let presented = headers
        .get(crate::setup_token::HEADER)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !crate::setup_token::verify(&state.config.sahai_data_root, presented).await {
        return Err(AppError::Unauthorized);
    }

    // dns_provider/acme_email/registry_url/registry_username/registry_passwordは
    // ここでは受け取らない(専用の「DNS/証明書設定」「レジストリ設定」画面で
    // 後から設定する。dto.rs::UpdateSettingsRequest参照)。空のまま仮置きしておく
    // (dns_providerに"cloudflare"等の特定プロバイダをハードコードしない、domainと
    // 同じ「既定値なし」方針)。registry_urlは空のまま渡し、
    // service::settings::setup内のapply_registry_url_defaultがdomainから自動生成する
    let new_settings = Settings {
        domain: req.domain,
        https_redirect: req.https_redirect,
        registry_url: String::new(),
        api_token: req.api_token,
        dns_provider: String::new(),
        acme_email: String::new(),
        registry_username: None,
        registry_password: None,
    };
    let saved = service::settings::setup(&state, new_settings).await?;

    // 役目を終えたトークンを失効させる。削除に失敗しても設定自体は成功しており、
    // 以後は`service::settings::setup`が設定済みとして拒否するため警告に留める
    if let Err(e) = crate::setup_token::revoke(&state.config.sahai_data_root).await {
        tracing::warn!("セットアップトークンの削除に失敗しました: {e}");
    }

    Ok(Json(BasicSettingsResponse::from(saved)))
}
