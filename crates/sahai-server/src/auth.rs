//! Bearerトークン検証ミドルウェア。

use axum::{
    extract::{Request, State},
    http::header::AUTHORIZATION,
    middleware::Next,
    response::Response,
};
use subtle::ConstantTimeEq;

use crate::error::AppError;
use crate::state::AppState;

pub async fn require_bearer_token(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Result<Response, AppError> {
    let header = request
        .headers()
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok());

    let token = header.and_then(|h| h.strip_prefix("Bearer "));
    let expected = state.settings.read().await.api_token.clone();

    match token {
        Some(t) if token_matches(t, &expected) => Ok(next.run(request).await),
        _ => Err(AppError::Unauthorized),
    }
}

/// 提示されたトークンが期待値と一致するか。
///
/// 初期設定前は`api_token`が空文字列(`Settings::unconfigured`)であり、素直に比較すると
/// `Bearer `(空トークン)が「空 == 空」で一致して全APIを素通りしてしまうため、
/// 期待値が空の場合は常に不一致として扱う。
/// 比較自体は定数時間で行い、先頭何文字まで一致したかが応答時間から漏れないようにする。
fn token_matches(presented: &str, expected: &str) -> bool {
    if expected.is_empty() || presented.is_empty() {
        return false;
    }
    presented.as_bytes().ct_eq(expected.as_bytes()).into()
}

#[cfg(test)]
mod tests {
    use super::token_matches;

    #[test]
    fn matches_identical_token() {
        assert!(token_matches("secret-token", "secret-token"));
    }

    #[test]
    fn rejects_different_token() {
        assert!(!token_matches("wrong", "secret-token"));
    }

    /// 未設定(空の期待値)に対して空トークンが一致してはいけない。
    #[test]
    fn rejects_empty_token_against_unconfigured_server() {
        assert!(!token_matches("", ""));
    }

    #[test]
    fn rejects_any_token_when_server_is_unconfigured() {
        assert!(!token_matches("anything", ""));
    }

    #[test]
    fn rejects_empty_token_against_configured_server() {
        assert!(!token_matches("", "secret-token"));
    }

    /// 前方一致するだけの短いトークンを受け付けない。
    #[test]
    fn rejects_prefix_of_expected_token() {
        assert!(!token_matches("secret", "secret-token"));
    }
}
