//! Control Plane APIを叩く薄いreqwestクライアント。

use serde::de::DeserializeOwned;
use serde_json::Value;

pub struct ApiClient {
    base_url: String,
    token: String,
    client: reqwest::Client,
}

impl ApiClient {
    /// `insecure=true`の場合、TLS証明書検証をスキップする。
    /// `SAHAI_DOMAIN`をローカルなドメイン(例: `localhost`)にしたテスト環境では、
    /// DNS-01でのLet's Encrypt証明書発行が行えず自己署名証明書のままになるため、
    /// このオプションが必要になる(本番のような実ドメイン運用では使うべきではない)。
    pub fn new(base_url: String, token: String, insecure: bool) -> Self {
        let client = reqwest::Client::builder()
            .danger_accept_invalid_certs(insecure)
            .build()
            .expect("reqwestクライアントの構築に失敗しました");
        ApiClient {
            base_url,
            token,
            client,
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url.trim_end_matches('/'), path)
    }

    pub async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T, String> {
        let resp = self
            .client
            .get(self.url(path))
            .bearer_auth(&self.token)
            .send()
            .await
            .map_err(|e| describe_error_chain(&e))?;
        Self::handle_response(resp).await
    }

    pub async fn post_empty<T: DeserializeOwned>(&self, path: &str) -> Result<T, String> {
        let resp = self
            .client
            .post(self.url(path))
            .bearer_auth(&self.token)
            .send()
            .await
            .map_err(|e| describe_error_chain(&e))?;
        Self::handle_response(resp).await
    }

    /// `metadata`パート(JSON文字列)+`archive`パート(tar.gzバイト列)のmultipart/form-data
    /// POST。サーバー側でのビルド(数分かかりうる)を待つため、この呼び出しにだけ
    /// 明示的に長めのタイムアウトを設定する(他の`get`/`post_empty`には影響させない)。
    pub async fn post_multipart<T: DeserializeOwned>(
        &self,
        path: &str,
        metadata_json: String,
        archive_bytes: Vec<u8>,
    ) -> Result<T, String> {
        let archive_part = reqwest::multipart::Part::bytes(archive_bytes)
            .file_name("archive.tar.gz")
            .mime_str("application/gzip")
            .map_err(|e| e.to_string())?;
        let form = reqwest::multipart::Form::new()
            .text("metadata", metadata_json)
            .part("archive", archive_part);

        let resp = self
            .client
            .post(self.url(path))
            .bearer_auth(&self.token)
            .timeout(std::time::Duration::from_secs(900))
            .multipart(form)
            .send()
            .await
            .map_err(|e| describe_error_chain(&e))?;
        Self::handle_response(resp).await
    }

    async fn handle_response<T: DeserializeOwned>(resp: reqwest::Response) -> Result<T, String> {
        if resp.status().is_success() {
            resp.json::<T>().await.map_err(|e| e.to_string())
        } else {
            let status = resp.status();
            let body: Value = resp
                .json()
                .await
                .unwrap_or_else(|_| serde_json::json!({"error": {"message": "不明なエラー"}}));
            Err(format!("HTTP {status}: {body}"))
        }
    }
}

/// エラーの`source()`チェーンを`: `区切りで連結する。reqwestの`Error::to_string()`は
/// トップレベルのメッセージ(例: "error sending request for url (...)")のみを返し、
/// TLS証明書検証エラーのような実際の原因はsourceチェーンの奥にしか現れないため、
/// これを展開しないとユーザーには何が起きているか分からない
/// (実機で自己署名証明書のドメインに対して`sahai container push`した際に発覚)。
fn describe_error_chain(err: &(dyn std::error::Error + 'static)) -> String {
    let mut parts = vec![err.to_string()];
    let mut source = err.source();
    while let Some(e) = source {
        parts.push(e.to_string());
        source = e.source();
    }
    parts.join(": ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug)]
    struct RootCause;
    impl std::fmt::Display for RootCause {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "invalid peer certificate")
        }
    }
    impl std::error::Error for RootCause {}

    #[derive(Debug)]
    struct MidLayer(RootCause);
    impl std::fmt::Display for MidLayer {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "error trying to connect")
        }
    }
    impl std::error::Error for MidLayer {
        fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
            Some(&self.0)
        }
    }

    #[derive(Debug)]
    struct TopLayer(MidLayer);
    impl std::fmt::Display for TopLayer {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "error sending request for url (https://example.test/)")
        }
    }
    impl std::error::Error for TopLayer {
        fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
            Some(&self.0)
        }
    }

    #[test]
    fn describe_error_chain_joins_all_source_messages() {
        let err = TopLayer(MidLayer(RootCause));
        assert_eq!(
            describe_error_chain(&err),
            "error sending request for url (https://example.test/): error trying to connect: invalid peer certificate"
        );
    }

    #[test]
    fn describe_error_chain_returns_single_message_when_no_source() {
        let err = RootCause;
        assert_eq!(describe_error_chain(&err), "invalid peer certificate");
    }

    #[test]
    fn url_joins_base_without_trailing_slash() {
        let client = ApiClient::new(
            "https://admin.example.com".to_string(),
            "t".to_string(),
            false,
        );
        assert_eq!(
            client.url("/api/services"),
            "https://admin.example.com/api/services"
        );
    }

    #[test]
    fn url_joins_base_with_trailing_slash_without_doubling() {
        let client = ApiClient::new(
            "https://admin.example.com/".to_string(),
            "t".to_string(),
            false,
        );
        assert_eq!(
            client.url("/api/services"),
            "https://admin.example.com/api/services"
        );
    }
}
