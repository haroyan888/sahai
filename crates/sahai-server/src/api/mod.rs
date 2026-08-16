pub mod dto;
pub mod not_http_service;
pub mod services;
pub mod settings;
pub mod setup;

use axum::extract::{DefaultBodyLimit, Request, State};
use axum::handler::Handler;
use axum::http::header;
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::Router;
use tower::ServiceExt;
use tower_http::cors::CorsLayer;
use tower_http::services::{ServeDir, ServeFile};

use crate::state::AppState;

/// Not Serviceページ(Web UI側のクライアントサイドルート)のパス。
const NOT_SERVICE_PATH: &str = "/not-service";

pub fn router(state: AppState) -> Router {
    // Web UI(React SPAのビルド済み静的ファイル)をsahai-server自身が配信する。
    // クライアントサイドルーティングのため、静的ファイルとして見つからないパスは
    // すべてindex.htmlへフォールバックする(SPAの標準パターン)
    // `not_found_service`はステータスを常に404へ強制するため使えない(SPAの
    // クライアントサイドルーティングには200でindex.htmlを返す必要がある)。
    // ステータスを元のまま(200)にする`fallback`を使う。
    // フォールバック先はindex.htmlを返すだけの`ServeFile`ではなく`spa_fallback`
    // ハンドラにして、アクセス元のホストによってはNot Serviceページへ寄せる。
    // `append_index_html_on_directories(false)`が要る: 既定では`/`宛てを
    // `ServeDir`自身がindex.htmlとして直接返してしまい、Traefikのcatch-allが
    // 転送してくる典型的なパスであるにもかかわらずフォールバックに届かない
    let serve_web_ui = ServeDir::new(&state.config.web_dist_dir)
        .append_index_html_on_directories(false)
        .fallback(spa_fallback.with_state(state.clone()));

    // axum 0.8の動的パスセグメントは`{name}`構文(0.7時代の`:name`はここでは使えない)。
    let authed = Router::new()
        .route("/api/services", get(services::list).post(services::create))
        .route(
            "/api/services/upload",
            // axum既定のボディサイズ上限(2MB)だと通常サイズのtar.gzでも413で
            // 弾かれるため、このルートにだけ明示的に上限を上げる(500MB)。
            post(services::upload).layer(DefaultBodyLimit::max(500 * 1024 * 1024)),
        )
        .route(
            "/api/services/{id_or_name}",
            get(services::get)
                .put(services::update)
                .delete(services::delete),
        )
        .route(
            "/api/services/{id_or_name}/upload",
            // /api/services/uploadと同じ理由でボディサイズ上限を引き上げる(500MB)
            post(services::update_upload).layer(DefaultBodyLimit::max(500 * 1024 * 1024)),
        )
        .route("/api/services/{id_or_name}/start", post(services::start))
        .route("/api/services/{id_or_name}/stop", post(services::stop))
        .route(
            "/api/services/{id_or_name}/restart",
            post(services::restart),
        )
        .route("/api/services/{id_or_name}/stats", get(services::stats))
        .route("/api/services/{id_or_name}/logs", get(services::logs))
        .route(
            "/api/services/{id_or_name}/registry",
            get(services::registry_status),
        )
        .route("/api/settings", get(settings::get).put(settings::update))
        .route(
            "/api/settings/dns-provider",
            get(settings::get_dns_config).put(settings::update_dns_config),
        )
        .route(
            "/api/settings/registry",
            get(settings::get_registry_config).put(settings::update_registry_config),
        )
        .route_layer(axum::middleware::from_fn_with_state(
            state.clone(),
            crate::auth::require_bearer_token,
        ));

    // Not Serviceページ用の情報API。Web UIが未ログイン状態でも問い合わせできるよう、
    // 認証は不要(authedの外側に登録することでBearerトークン必須層を
    // 経由しない)。Traefikは非HTTPサービス・未登録サブドメイン宛てのアクセスをすべて
    // Web UIコンテナへ転送する設計に変更したため、
    // このAPIはTraefikからの転送ではなくWeb UI自身が`?host=`を付けて明示的に呼ぶ。
    //
    // Web UI(Vite開発サーバー等、別オリジンで動く管理画面)からAPIを直接叩けるようにする。
    // 認証はBearerトークン(JSが明示的に付与する)で行っており、ブラウザのCookie自動送信を
    // 前提にしていないため、オリジン制限をCORSのセキュリティ境界として使う設計ではない。
    // オリジンを絞っても得られる安全性が無いためpermissiveとする。
    Router::new()
        .merge(authed)
        .route("/api/not-service", get(not_http_service::handler))
        // 初回セットアップ用。未ログイン(=まだトークンを知らない)状態のWeb UIから
        // 叩く必要があるため認証層の外側に置く。作成自体は未設定時のみ許可する
        // (service::settings::setup参照)
        .route("/api/setup", get(setup::status).post(setup::create))
        .fallback_service(serve_web_ui)
        .layer(CorsLayer::permissive())
        .with_state(state)
}

/// SPAのエントリポイント(index.html)を返すフォールバック。
///
/// Traefikのcatch-allルートは`*.<domain>`宛でどのサービス用ルートにもマッチしなかった
/// リクエストを、パス`/`のままここへ転送してくる。そのままindex.htmlを返すとSPAは
/// 認証ゲートに落ち、エンドユーザーには身に覚えのないログイン画面が出てしまうため、
/// 管理画面(`sahai.<domain>`)以外のサブドメイン宛てならNot Serviceページへ寄せる。
///
/// この判定を`ServeDir`の前段ではなくフォールバック側で行うのは、実在するアセット
/// (`/assets/*`等)を巻き込まないため。前段だとNot Serviceページ自身のJS/CSSまで
/// リダイレクトされ、ページが描画できなくなる。
async fn spa_fallback(State(state): State<AppState>, req: Request) -> Response {
    let domain = state.settings.read().await.domain.clone();
    let host = req
        .headers()
        .get(header::HOST)
        .and_then(|value| value.to_str().ok());

    if needs_not_service_redirect(host, &domain, req.uri().path()) {
        // 303 See Other。案内ページは常にGETで取りに行かせる
        return Redirect::to(NOT_SERVICE_PATH).into_response();
    }

    ServeFile::new(state.config.web_dist_dir.join("index.html"))
        .oneshot(req)
        .await
        .into_response()
}

/// Not Serviceページへ寄せるべきアクセスかを判定する。
///
/// ベースドメイン配下のサブドメインだけを対象にすることで、`localhost`や生IPでの
/// 直接アクセス、およびベースドメインが未確定な初期設定前を巻き込まない
/// (Traefikのcatch-allルートも`*.<domain>`にしかマッチせず、判定範囲は一致する)。
/// リダイレクト先自身を除外しないと無限ループになる。
fn needs_not_service_redirect(host: Option<&str>, domain: &str, path: &str) -> bool {
    if domain.is_empty() || path == NOT_SERVICE_PATH {
        return false;
    }
    let Some(host) = host else {
        return false;
    };
    // Hostヘッダーは`example.com:8443`のようにポートを伴いうる。
    // ホスト名自体の大文字小文字は区別されない
    let hostname = host.split(':').next().unwrap_or(host).to_ascii_lowercase();
    let domain = domain.to_ascii_lowercase();

    hostname != format!("sahai.{domain}") && hostname.ends_with(&format!(".{domain}"))
}

#[cfg(test)]
mod router_tests {
    use axum::body::to_bytes;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    use crate::api::dto::{ContainerInput, CreateServiceRequest};
    use crate::service::{
        registration,
        test_support::{test_state, test_state_unconfigured},
    };
    use crate::state::AppState;

    async fn register_non_http_service(state: &AppState) {
        registration::create(
            state,
            CreateServiceRequest {
                name: "mysql".to_string(),
                source_type: "image".to_string(),
                image: Some("mysql:8".to_string()),
                compose_content: None,
                env_vars: None,
                containers: vec![ContainerInput {
                    name: "mysql".to_string(),
                    ports: vec![crate::api::dto::PortInput {
                        container_port: 3306,
                        host_port: Some(20050),
                        protocol: "tcp".to_string(),
                        is_http: false,
                    }],
                    volumes: vec![],
                }],
            },
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn not_service_api_returns_found_service_info_without_auth() {
        // Web UI(未ログインでも見られるNot Serviceページ)がwindow.location.hostnameを
        // ?host=として明示的に渡して問い合わせる設計。認証不要
        let state = test_state().await;
        register_non_http_service(&state).await;
        let app = super::router(state);

        let request = Request::builder()
            .uri("/api/not-service?host=mysql.example.com")
            .body(axum::body::Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(json["found"], true);
        assert_eq!(json["name"], "mysql");
        assert_eq!(json["ports"][0]["host_port"], 20050);
    }

    #[tokio::test]
    async fn not_service_api_returns_found_false_for_unknown_subdomain() {
        // 未登録サブドメインもエラーではなく`found: false`のJSONで表現する
        // (Docker操作の成否をstatusフィールドで表現するのと同じ設計方針)
        let state = test_state().await;
        let app = super::router(state);

        let request = Request::builder()
            .uri("/api/not-service?host=doesnotexist.example.com")
            .body(axum::body::Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(json["found"], false);
    }

    #[tokio::test]
    async fn api_routes_still_require_bearer_token() {
        let state = test_state().await;
        let app = super::router(state);

        let request = Request::builder()
            .uri("/api/services")
            .body(axum::body::Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    /// 初期設定前は`api_token`が空文字列のため、`Bearer `(空トークン)が
    /// 「空 == 空」で一致して認証を素通りしないことを保証する。
    #[tokio::test]
    async fn empty_bearer_token_is_rejected_when_unconfigured() {
        let state = test_state_unconfigured().await;
        let app = super::router(state);

        let request = Request::builder()
            .uri("/api/services")
            .header("Authorization", "Bearer ")
            .body(axum::body::Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(
            response.status(),
            StatusCode::UNAUTHORIZED,
            "未設定状態でも空トークンで認証を通してはいけない"
        );
    }

    #[tokio::test]
    async fn cors_preflight_is_allowed_for_api_routes() {
        // Web UI(Vite開発サーバー等)が別オリジンからAPIを叩けるようにするため、
        // プリフライト(OPTIONS)にAccess-Control-Allow-*ヘッダーが付与される必要がある。
        let state = test_state().await;
        let app = super::router(state);

        let request = Request::builder()
            .method("OPTIONS")
            .uri("/api/services")
            .header(axum::http::header::ORIGIN, "http://localhost:5173")
            .header(axum::http::header::ACCESS_CONTROL_REQUEST_METHOD, "GET")
            .body(axum::body::Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert!(response
            .headers()
            .contains_key(axum::http::header::ACCESS_CONTROL_ALLOW_ORIGIN));
    }

    #[tokio::test]
    async fn cors_header_is_present_on_actual_api_responses() {
        let state = test_state().await;
        let app = super::router(state);

        let request = Request::builder()
            .uri("/api/services")
            .header(axum::http::header::ORIGIN, "http://localhost:5173")
            .header(axum::http::header::AUTHORIZATION, "Bearer test")
            .body(axum::body::Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert!(response
            .headers()
            .contains_key(axum::http::header::ACCESS_CONTROL_ALLOW_ORIGIN));
    }

    #[tokio::test]
    async fn get_settings_returns_current_values() {
        let state = test_state().await;
        let app = super::router(state);

        let request = Request::builder()
            .uri("/api/settings")
            .header(axum::http::header::AUTHORIZATION, "Bearer test")
            .body(axum::body::Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(json["domain"], "example.com");
        assert_eq!(json["https_redirect"], true);
    }

    #[tokio::test]
    async fn put_settings_persists_and_is_reflected_immediately() {
        let state = test_state().await;
        let app = super::router(state.clone());

        let body = serde_json::json!({
            "domain": "example.com",
            "https_redirect": false,
            "api_token": "test",
        });
        let request = Request::builder()
            .method("PUT")
            .uri("/api/settings")
            .header(axum::http::header::AUTHORIZATION, "Bearer test")
            .header(axum::http::header::CONTENT_TYPE, "application/json")
            .body(axum::body::Body::from(body.to_string()))
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        // 保存後すぐに共有stateへ反映されていること
        let reflected = state.settings.read().await;
        assert_eq!(reflected.domain, "example.com");
        assert!(!reflected.https_redirect);
        // dns_provider/registry_urlはこの画面では変更できず、既存値(test_settings()の
        // 既定値)がそのまま維持されるはず(専用のDNS/証明書設定・レジストリ設定画面
        // でのみ変更できる)
        assert_eq!(reflected.dns_provider, "cloudflare");
        assert_eq!(reflected.registry_url, "registry.example.test");
    }

    #[tokio::test]
    async fn put_settings_rejects_empty_domain() {
        let state = test_state().await;
        let app = super::router(state);

        let body = serde_json::json!({
            "domain": "",
            "https_redirect": true,
            "api_token": "test",
        });
        let request = Request::builder()
            .method("PUT")
            .uri("/api/settings")
            .header(axum::http::header::AUTHORIZATION, "Bearer test")
            .header(axum::http::header::CONTENT_TYPE, "application/json")
            .body(axum::body::Body::from(body.to_string()))
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn dns_provider_routes_still_require_bearer_token() {
        let state = test_state().await;
        let app = super::router(state);

        let request = Request::builder()
            .uri("/api/settings/dns-provider")
            .body(axum::body::Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn get_dns_provider_returns_current_config() {
        let state = test_state().await;
        let app = super::router(state);

        let request = Request::builder()
            .uri("/api/settings/dns-provider")
            .header(axum::http::header::AUTHORIZATION, "Bearer test")
            .body(axum::body::Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(json["dns_provider"], "cloudflare");
        assert_eq!(json["credentials"], serde_json::json!([]));
    }

    #[tokio::test]
    async fn put_dns_provider_rejects_empty_provider() {
        let state = test_state().await;
        let app = super::router(state);

        let body = serde_json::json!({
            "dns_provider": "",
            "acme_email": "a@b.com",
            "credentials": [],
        });
        let request = Request::builder()
            .method("PUT")
            .uri("/api/settings/dns-provider")
            .header(axum::http::header::AUTHORIZATION, "Bearer test")
            .header(axum::http::header::CONTENT_TYPE, "application/json")
            .body(axum::body::Body::from(body.to_string()))
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn registry_config_routes_still_require_bearer_token() {
        let state = test_state().await;
        let app = super::router(state);

        let request = Request::builder()
            .uri("/api/settings/registry")
            .body(axum::body::Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn get_registry_config_returns_current_values() {
        let state = test_state().await;
        let app = super::router(state);

        let request = Request::builder()
            .uri("/api/settings/registry")
            .header(axum::http::header::AUTHORIZATION, "Bearer test")
            .body(axum::body::Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(json["registry_url"], "registry.example.test");
        assert!(
            json["login_warning"].is_null()
                || !json.as_object().unwrap().contains_key("login_warning")
        );
    }

    #[tokio::test]
    async fn put_registry_config_persists_and_reflects_immediately() {
        // registry.sahai.example.comには実際には到達できないため、docker loginは必ず失敗する。
        // それでも200・DB保存成功・login_warning付きで返ることを検証する
        // (service::settings::update_registry_configのテストと対応)
        let state = test_state().await;
        let app = super::router(state.clone());

        let body = serde_json::json!({
            "registry_url": "registry.sahai.example.com",
            "registry_username": "reguser",
            "registry_password": "regpass",
        });
        let request = Request::builder()
            .method("PUT")
            .uri("/api/settings/registry")
            .header(axum::http::header::AUTHORIZATION, "Bearer test")
            .header(axum::http::header::CONTENT_TYPE, "application/json")
            .body(axum::body::Body::from(body.to_string()))
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(json["registry_url"], "registry.sahai.example.com");
        assert!(
            json["login_warning"].is_string(),
            "docker login失敗の警告が載るはず"
        );

        let reflected = state.settings.read().await;
        assert_eq!(reflected.registry_username.as_deref(), Some("reguser"));
        assert_eq!(reflected.registry_password.as_deref(), Some("regpass"));
    }

    #[tokio::test]
    async fn put_registry_config_rejects_username_without_password() {
        let state = test_state().await;
        let app = super::router(state);

        let body = serde_json::json!({
            "registry_url": "registry.sahai.example.com",
            "registry_username": "reguser",
        });
        let request = Request::builder()
            .method("PUT")
            .uri("/api/settings/registry")
            .header(axum::http::header::AUTHORIZATION, "Bearer test")
            .header(axum::http::header::CONTENT_TYPE, "application/json")
            .body(axum::body::Body::from(body.to_string()))
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn put_dns_provider_persists_credentials_even_though_traefik_recreate_is_unreachable_in_tests(
    ) {
        // test_state()のcompose_file_pathは実在しないため、Traefik再作成は必ず失敗し
        // 500が返る。ただしその前段のDB/.env書き込みは実行される
        // (service::settings::update_dns_configのテスト参照)。ここではAPI層が
        // service層のエラーを正しく502/500系として伝播することだけを確認する
        let state = test_state().await;
        {
            let path = &state.config.env_file_path;
            tokio::fs::create_dir_all(path.parent().unwrap())
                .await
                .unwrap();
            tokio::fs::write(path, "SAHAI_DOMAIN=example.com\n")
                .await
                .unwrap();
        }
        let app = super::router(state.clone());

        let body = serde_json::json!({
            "dns_provider": "route53",
            "acme_email": "a@b.com",
            "credentials": [{"key": "AWS_ACCESS_KEY_ID", "value": "AKIA..."}],
        });
        let request = Request::builder()
            .method("PUT")
            .uri("/api/settings/dns-provider")
            .header(axum::http::header::AUTHORIZATION, "Bearer test")
            .header(axum::http::header::CONTENT_TYPE, "application/json")
            .body(axum::body::Body::from(body.to_string()))
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(state.settings.read().await.dns_provider, "route53");
    }

    #[tokio::test]
    async fn setup_status_reports_unconfigured_without_auth() {
        let state = test_state_unconfigured().await;
        let app = super::router(state);

        let request = Request::builder()
            .uri("/api/setup")
            .body(axum::body::Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(json["configured"], false);
    }

    #[tokio::test]
    async fn setup_status_reports_configured_without_auth() {
        let state = test_state().await;
        let app = super::router(state);

        let request = Request::builder()
            .uri("/api/setup")
            .body(axum::body::Body::empty())
            .unwrap();

        let response = app.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(json["configured"], true);
    }

    fn setup_request(
        body: serde_json::Value,
        setup_token: Option<&str>,
    ) -> Request<axum::body::Body> {
        let mut builder = Request::builder()
            .method("POST")
            .uri("/api/setup")
            .header(axum::http::header::CONTENT_TYPE, "application/json");
        if let Some(token) = setup_token {
            builder = builder.header(crate::setup_token::HEADER, token);
        }
        builder
            .body(axum::body::Body::from(body.to_string()))
            .unwrap()
    }

    fn setup_body() -> serde_json::Value {
        serde_json::json!({
            "domain": "example.com",
            "https_redirect": true,
            "api_token": "initial-token",
        })
    }

    /// 起動時に発行されたセットアップトークンを提示すれば、Bearer認証なしで初期設定できる。
    #[tokio::test]
    async fn setup_create_succeeds_with_setup_token_when_unconfigured() {
        let state = test_state_unconfigured().await;
        let token = crate::setup_token::issue(&state.config.sahai_data_root)
            .await
            .unwrap();
        let app = super::router(state.clone());

        let response = app
            .oneshot(setup_request(setup_body(), Some(&token)))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(state.settings.read().await.api_token, "initial-token");
    }

    /// トークン未提示の初期設定は拒否する(第三者による初期設定の先取りを防ぐ)。
    #[tokio::test]
    async fn setup_create_is_rejected_without_setup_token() {
        let state = test_state_unconfigured().await;
        crate::setup_token::issue(&state.config.sahai_data_root)
            .await
            .unwrap();
        let app = super::router(state.clone());

        let response = app
            .oneshot(setup_request(setup_body(), None))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(
            state.settings.read().await.api_token,
            "",
            "拒否された場合は設定が書き換わっていないこと"
        );
    }

    #[tokio::test]
    async fn setup_create_is_rejected_with_wrong_setup_token() {
        let state = test_state_unconfigured().await;
        crate::setup_token::issue(&state.config.sahai_data_root)
            .await
            .unwrap();
        let app = super::router(state.clone());

        let response = app
            .oneshot(setup_request(setup_body(), Some("wrong-token")))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    /// 初期設定に成功したらトークンは失効し、二度目は使えない。
    #[tokio::test]
    async fn setup_token_is_revoked_after_successful_setup() {
        let state = test_state_unconfigured().await;
        let token = crate::setup_token::issue(&state.config.sahai_data_root)
            .await
            .unwrap();

        let ok = super::router(state.clone())
            .oneshot(setup_request(setup_body(), Some(&token)))
            .await
            .unwrap();
        assert_eq!(ok.status(), StatusCode::OK);

        assert!(
            !crate::setup_token::verify(&state.config.sahai_data_root, &token).await,
            "設定完了後のトークンは失効しているはず"
        );
    }

    /// 設定済みなら、有効なトークンを提示しても初期設定はやり直せない
    /// (トークン検証とは独立した`service::settings::setup`側のガード)。
    #[tokio::test]
    async fn setup_create_rejects_when_already_configured() {
        let state = test_state().await;
        let token = crate::setup_token::issue(&state.config.sahai_data_root)
            .await
            .unwrap();
        let app = super::router(state.clone());

        let body = serde_json::json!({
            "domain": "example.com",
            "https_redirect": true,
            "api_token": "hijack-attempt",
        });

        let response = app
            .oneshot(setup_request(body, Some(&token)))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::CONFLICT);
    }

    // Web UI(React SPA)をsahai-server自身が配信するため、静的ファイル配信+
    // クライアントサイドルーティング用のSPAフォールバックをここで検証する。
    async fn write_fake_web_dist(state: &AppState) {
        let dir = &state.config.web_dist_dir;
        tokio::fs::create_dir_all(dir).await.unwrap();
        tokio::fs::write(dir.join("index.html"), "<html>spa-shell</html>")
            .await
            .unwrap();
        tokio::fs::create_dir_all(dir.join("assets")).await.unwrap();
        tokio::fs::write(dir.join("assets").join("app.js"), "console.log('app')")
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn serves_existing_static_asset() {
        let state = test_state().await;
        write_fake_web_dist(&state).await;
        let app = super::router(state);

        let request = Request::builder()
            .uri("/assets/app.js")
            .body(axum::body::Body::empty())
            .unwrap();
        let response = app.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert_eq!(bytes, "console.log('app')".as_bytes());
    }

    /// Traefikのcatch-allルートに拾われたリクエスト(登録されていないサブドメイン宛て)。
    /// そのままSPAを返すと認証ゲートに落ちてログイン画面が出てしまうため、
    /// 案内ページ(/not-service)へ寄せる
    #[tokio::test]
    async fn redirects_to_not_service_page_for_unregistered_subdomain() {
        let state = test_state().await;
        write_fake_web_dist(&state).await;
        let app = super::router(state);

        let request = Request::builder()
            .uri("/")
            .header(axum::http::header::HOST, "nosuch.example.com")
            .body(axum::body::Body::empty())
            .unwrap();
        let response = app.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        assert_eq!(
            response
                .headers()
                .get(axum::http::header::LOCATION)
                .and_then(|v| v.to_str().ok()),
            Some("/not-service")
        );
    }

    /// 管理画面のホストは従来どおりSPAを返す(ログイン画面へ到達できること)。
    #[tokio::test]
    async fn serves_spa_for_admin_host() {
        let state = test_state().await;
        write_fake_web_dist(&state).await;
        let app = super::router(state);

        let request = Request::builder()
            .uri("/")
            .header(axum::http::header::HOST, "sahai.example.com")
            .body(axum::body::Body::empty())
            .unwrap();
        let response = app.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert_eq!(bytes, "<html>spa-shell</html>".as_bytes());
    }

    /// 実在するアセットはリダイレクト対象外。リダイレクトしてしまうと、
    /// Not Serviceページ自身のJS/CSSが読めず白画面になる
    #[tokio::test]
    async fn does_not_redirect_existing_assets_on_unregistered_subdomain() {
        let state = test_state().await;
        write_fake_web_dist(&state).await;
        let app = super::router(state);

        let request = Request::builder()
            .uri("/assets/app.js")
            .header(axum::http::header::HOST, "nosuch.example.com")
            .body(axum::body::Body::empty())
            .unwrap();
        let response = app.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert_eq!(bytes, "console.log('app')".as_bytes());
    }

    /// リダイレクト先自身はSPAを返す(無限リダイレクトにしない)。
    #[tokio::test]
    async fn serves_spa_for_not_service_path_itself() {
        let state = test_state().await;
        write_fake_web_dist(&state).await;
        let app = super::router(state);

        let request = Request::builder()
            .uri("/not-service")
            .header(axum::http::header::HOST, "nosuch.example.com")
            .body(axum::body::Body::empty())
            .unwrap();
        let response = app.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert_eq!(bytes, "<html>spa-shell</html>".as_bytes());
    }

    /// 初期設定前(ベースドメイン未確定)は判定できないため、常にSPAを返す。
    /// ここでリダイレクトすると初期設定の案内画面に到達できなくなる
    #[tokio::test]
    async fn serves_spa_when_domain_is_not_configured_yet() {
        let state = test_state_unconfigured().await;
        write_fake_web_dist(&state).await;
        let app = super::router(state);

        let request = Request::builder()
            .uri("/")
            .header(axum::http::header::HOST, "nosuch.example.com")
            .body(axum::body::Body::empty())
            .unwrap();
        let response = app.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn falls_back_to_index_html_for_unknown_client_side_routes() {
        // React Routerが処理する/services等のパスはディスク上にファイルが無いため、
        // SPAのエントリポイント(index.html)へフォールバックする必要がある
        let state = test_state().await;
        write_fake_web_dist(&state).await;
        let app = super::router(state);

        let request = Request::builder()
            .uri("/services/myapp")
            .body(axum::body::Body::empty())
            .unwrap();
        let response = app.oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert_eq!(bytes, "<html>spa-shell</html>".as_bytes());
    }
}

#[cfg(test)]
mod not_service_redirect_tests {
    use super::needs_not_service_redirect;

    const DOMAIN: &str = "example.com";

    fn redirects(host: &str) -> bool {
        needs_not_service_redirect(Some(host), DOMAIN, "/")
    }

    #[test]
    fn unregistered_subdomain_is_redirected() {
        assert!(redirects("nosuch.example.com"));
    }

    /// 非HTTPサービスのサブドメインも同じ案内ページ(ポート一覧)へ寄せる。
    #[test]
    fn non_http_service_subdomain_is_redirected() {
        assert!(redirects("mysql.example.com"));
    }

    #[test]
    fn admin_host_is_not_redirected() {
        assert!(!redirects("sahai.example.com"));
    }

    /// `localhost`・生IPはTraefikのcatch-all(`*.<domain>`)の対象外。
    /// 開発時やポートフォワード経由の直接アクセスを巻き込まない
    #[test]
    fn hosts_outside_the_base_domain_are_not_redirected() {
        assert!(!redirects("localhost"));
        assert!(!redirects("127.0.0.1"));
        assert!(!redirects("sahai.other.test"));
        // ベースドメインそのもの(apex)もサブドメインではない
        assert!(!redirects("example.com"));
        // 部分一致でベースドメイン扱いしない(`.`区切りを要求する)
        assert!(!redirects("notexample.com"));
    }

    #[test]
    fn host_header_port_is_ignored() {
        assert!(redirects("nosuch.example.com:8443"));
        assert!(!redirects("sahai.example.com:8443"));
    }

    #[test]
    fn host_header_case_is_ignored() {
        assert!(!redirects("Sahai.Example.COM"));
        assert!(redirects("NoSuch.Example.COM"));
    }

    /// リダイレクト先自身は寄せない(無限リダイレクトの防止)。
    #[test]
    fn not_service_path_itself_is_not_redirected() {
        assert!(!needs_not_service_redirect(
            Some("nosuch.example.com"),
            DOMAIN,
            "/not-service"
        ));
    }

    /// 初期設定前はベースドメインが未確定で判定できない。
    #[test]
    fn empty_domain_never_redirects() {
        assert!(!needs_not_service_redirect(
            Some("nosuch.example.com"),
            "",
            "/"
        ));
    }

    #[test]
    fn missing_host_header_never_redirects() {
        assert!(!needs_not_service_redirect(None, DOMAIN, "/"));
    }
}
