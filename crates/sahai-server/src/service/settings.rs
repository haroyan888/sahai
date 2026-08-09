//! 設定画面(Web UI)のオーケストレーション。DBへの永続化+`SharedSettings`への
//! 反映+影響を受けるTraefikルートの再生成までを1つの操作として行う。

use crate::error::{AppError, FieldError};
use crate::repo::settings::SettingsRow;
use crate::repo::{services, settings as settings_repo};
use crate::settings::Settings;
use crate::state::AppState;

/// DNS/証明書設定(Web UIの「DNS/証明書設定」セクション)。domain等の基本設定とは
/// 別画面・別保存アクションにしている。こちらの保存はTraefikコンテナの再作成を伴い
/// (数秒の一時的な接続断が起きる)、影響範囲を利用者に明示しやすくするため。
/// `credentials`はキーが空文字列でないことをバリデーションで保証するのみで、
/// あえて`Vec<(String, String)>`のまま持つ(repo層のシグネチャとも揃えている)。
/// JSONへ`{key, value}`形式で返す変換はAPI層(api/settings.rs)の責務とする
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DnsConfig {
    pub dns_provider: String,
    pub acme_email: String,
    /// 選んだDNSプロバイダが要求する認証情報(例: cloudflareなら`CF_DNS_API_TOKEN`)。
    /// キーはそのプロバイダが指定する環境変数名をそのまま使う
    /// (一覧: https://go-acme.github.io/lego/dns/index.html)。
    pub credentials: Vec<(String, String)>,
}

pub async fn get_dns_config(state: &AppState) -> Result<DnsConfig, AppError> {
    let (dns_provider, acme_email) = {
        let settings = state.settings.read().await;
        (settings.dns_provider.clone(), settings.acme_email.clone())
    };
    let credentials = settings_repo::list_dns_provider_credentials(state.db.pool()).await?;
    Ok(DnsConfig {
        dns_provider,
        acme_email,
        credentials,
    })
}

/// バリデーション→`.sahai.env`書き込み→DB永続化→インメモリ反映→
/// Traefikコンテナの再作成、をこの順で行う。ブリッジファイル書き込みを最初に行うのは、
/// ここで失敗した場合にDBとファイルの内容が食い違った状態を作らないため(何も
/// 変更されていない状態でエラーを返せる)。ファイルが存在しなければ`env_file::upsert`が
/// ディレクトリごと自動作成する。Traefikの再作成を最後に置くのは、DB・ファイルは
/// 既に新しい内容になっているため、再作成に失敗しても手動で
/// `docker start <traefikコンテナ名>`をやり直せば復旧できるからである。
pub async fn update_dns_config(
    state: &AppState,
    new_config: DnsConfig,
) -> Result<DnsConfig, AppError> {
    validate_dns_config(&new_config)?;

    let mut env_updates: Vec<(&str, &str)> = vec![
        ("SAHAI_DNS_PROVIDER", new_config.dns_provider.as_str()),
        ("SAHAI_ACME_EMAIL", new_config.acme_email.as_str()),
    ];
    for (key, value) in &new_config.credentials {
        env_updates.push((key.as_str(), value.as_str()));
    }
    crate::env_file::upsert(&state.config.env_file_path, &env_updates)
        .await
        .map_err(|e| AppError::Internal(format!(".sahai.envの書き込みに失敗しました: {e}")))?;

    {
        let mut current = state.settings.write().await;
        current.dns_provider = new_config.dns_provider.clone();
        current.acme_email = new_config.acme_email.clone();
    }
    let row: SettingsRow = (&state.settings.read().await.clone()).into();
    settings_repo::update(state.db.pool(), &row).await?;
    settings_repo::replace_dns_provider_credentials(state.db.pool(), &new_config.credentials)
        .await?;

    // dns_provider/acme_emailはTraefikの静的設定(CLI引数)としてしか渡せないため、
    // 再作成時にコンテナの起動コマンドへ反映させる必要がある(container.rsの
    // override_acme_cmd_flags参照)。ここまでのバリデーションでdns_providerは
    // 非空であることが保証されている
    crate::traefik::recreate_traefik(
        &state.docker.docker,
        &state.config.env_file_path,
        &new_config.dns_provider,
        &new_config.acme_email,
    )
    .await
    .map_err(|e| AppError::Internal(format!("Traefikコンテナの再作成に失敗しました: {e}")))?;

    Ok(new_config)
}

fn validate_dns_config(config: &DnsConfig) -> Result<(), AppError> {
    let mut errors = Vec::new();
    if config.dns_provider.trim().is_empty() {
        errors.push(FieldError {
            field: "dns_provider".to_string(),
            message: "DNSプロバイダを入力してください".to_string(),
        });
    }
    if config.acme_email.trim().is_empty() {
        errors.push(FieldError {
            field: "acme_email".to_string(),
            message: "通知先メールアドレスを入力してください".to_string(),
        });
    }
    for (i, (key, _)) in config.credentials.iter().enumerate() {
        if key.trim().is_empty() {
            errors.push(FieldError {
                field: format!("credentials[{i}].key"),
                message: "キーを入力してください".to_string(),
            });
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(AppError::Validation(errors))
    }
}

pub async fn get(state: &AppState) -> Settings {
    state.settings.read().await.clone()
}

/// api_tokenが未設定(空文字列)かどうかで「初回セットアップ未完了」を判定する。
pub async fn is_configured(state: &AppState) -> bool {
    !state.settings.read().await.api_token.trim().is_empty()
}

/// 初回セットアップ(未設定時のみ許可)。バリデーション→DB新規作成→インメモリ反映→
/// 管理画面用Traefikルートの初回書き出し、をこの順で行う。起動時点ではdomainが
/// 空でルートを書き出せないため、ここで初めて書き出す(main.rs参照)。
pub async fn setup(state: &AppState, mut new_settings: Settings) -> Result<Settings, AppError> {
    if is_configured(state).await {
        return Err(AppError::Conflict("既に設定済みです".to_string()));
    }
    apply_registry_url_default(&mut new_settings);
    validate(&new_settings)?;

    let row: SettingsRow = (&new_settings).into();
    settings_repo::seed(state.db.pool(), &row).await?;

    {
        let mut current = state.settings.write().await;
        *current = new_settings.clone();
    }

    if let Err(e) = state
        .traefik
        .write_static_admin_routes(&state.registry_internal_url)
        .await
    {
        tracing::warn!("管理画面用の静的Traefikルートの書き出しに失敗しました: {e}");
    }

    Ok(new_settings)
}

/// バリデーション→DB永続化→インメモリ反映→影響を受けるTraefikルートの再生成、
/// をこの順で行う。domain/https_redirectの変更は既存の全登録サービスのルートに
/// 影響するため、保存のたびに管理画面ルート+全サービスのルートを再生成する
/// (「保存後すぐに反映」というユーザー確認済みの方針)。
pub async fn update(state: &AppState, mut new_settings: Settings) -> Result<Settings, AppError> {
    apply_registry_url_default(&mut new_settings);
    validate(&new_settings)?;

    let row: SettingsRow = (&new_settings).into();
    settings_repo::update(state.db.pool(), &row).await?;

    {
        let mut current = state.settings.write().await;
        *current = new_settings.clone();
    }

    if let Err(e) = state
        .traefik
        .write_static_admin_routes(&state.registry_internal_url)
        .await
    {
        tracing::warn!("管理画面用の静的Traefikルートの再生成に失敗しました: {e}");
    }

    let rows = services::list_all(state.db.pool()).await?;
    for row in rows {
        // 個々のサービスのルート再生成が1件失敗しても、他のサービスへは影響させず
        // 続行する(設定保存自体は成功として扱う。失敗はログに残す)
        match super::load_detail_by_id(state, row.id).await {
            Ok(detail) => {
                if let Err(e) = state.traefik.write_route(&detail).await {
                    tracing::warn!(
                        "サービス '{}' のTraefikルート再生成に失敗しました: {e}",
                        row.name
                    );
                }
            }
            Err(e) => {
                tracing::warn!("サービス '{}' の詳細取得に失敗しました: {e:?}", row.name);
            }
        }
    }

    Ok(new_settings)
}

/// registry_urlが空の場合、domainから`registry.sahai.<domain>`を自動生成する
/// (settings.rs::seed_from_envの既存の自動補完と同じ式をWeb UI経由の保存にも適用する)。
/// domainが空の場合は何もしない — その場合はdomain自体のバリデーションエラーで
/// 申し込み全体がブロックされるため、registry_urlにまで空の値を作る必要が無い。
fn apply_registry_url_default(settings: &mut Settings) {
    if settings.registry_url.trim().is_empty() && !settings.domain.trim().is_empty() {
        settings.registry_url = default_registry_url(&settings.domain);
    }
}

fn default_registry_url(domain: &str) -> String {
    format!("registry.sahai.{domain}")
}

/// レジストリ設定(Web UIの「レジストリ設定」カード)。sahai service createが
/// サーバー側でdocker build/pushする際に使う資格情報+レジストリURL。
/// username/passwordの保存はTraefik再作成のような重い副作用を伴わず、docker loginは
/// 同期的にすぐ終わるため、DNS/証明書設定のような再接続ポーリングの仕組みは持たない。
/// docker loginが失敗してもDB保存自体は成功として扱い、login_warningとして
/// 呼び出し元に伝える(domain.rs::ServiceDetail.route_warningと同じ
/// 「保存は成功、警告だけ伝える」パターン)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistryConfig {
    pub registry_url: String,
    pub registry_username: Option<String>,
    pub registry_password: Option<String>,
    /// docker loginに失敗した場合のみSome。get_registry_configでは常にNone
    pub login_warning: Option<String>,
}

pub async fn get_registry_config(state: &AppState) -> RegistryConfig {
    let settings = state.settings.read().await;
    RegistryConfig {
        registry_url: settings.registry_url.clone(),
        registry_username: settings.registry_username.clone(),
        registry_password: settings.registry_password.clone(),
        login_warning: None,
    }
}

/// バリデーション→(空ならdomainから自動生成)→インメモリ反映→DB永続化→
/// (資格情報が両方揃っていれば)docker loginの順で行う。DNS設定
/// (update_dns_config)と異なり、docker loginは同期的にすぐ終わる軽い処理で
/// 接続断も起きないため、失敗してもエラーにはせずlogin_warningに理由を積んで
/// 200で返す(ユーザー確定方針。DB保存自体は既に完了しているため、資格情報が
/// 間違っていてもレジストリURLだけ先に保存しておく、といった使い方ができる)
pub async fn update_registry_config(
    state: &AppState,
    mut new_config: RegistryConfig,
) -> Result<RegistryConfig, AppError> {
    new_config.registry_username = normalize_credential(new_config.registry_username);
    new_config.registry_password = normalize_credential(new_config.registry_password);
    validate_registry_config(&new_config)?;

    let domain = state.settings.read().await.domain.clone();
    if new_config.registry_url.trim().is_empty() && !domain.trim().is_empty() {
        new_config.registry_url = default_registry_url(&domain);
    }

    {
        let mut current = state.settings.write().await;
        current.registry_url = new_config.registry_url.clone();
        current.registry_username = new_config.registry_username.clone();
        current.registry_password = new_config.registry_password.clone();
    }
    let row: SettingsRow = (&state.settings.read().await.clone()).into();
    settings_repo::update(state.db.pool(), &row).await?;

    let login_warning = match (&new_config.registry_username, &new_config.registry_password) {
        (Some(username), Some(password)) => {
            match crate::docker::registry_login::login(&new_config.registry_url, username, password)
                .await
            {
                Ok(()) => None,
                Err(e) => Some(format!("レジストリへのログインに失敗しました: {e}")),
            }
        }
        _ => None,
    };

    Ok(RegistryConfig {
        login_warning,
        ..new_config
    })
}

/// 空文字列は「未入力」として扱い、Noneに正規化する(username/passwordどちらも
/// 空にすれば資格情報をクリアできる)
fn normalize_credential(value: Option<String>) -> Option<String> {
    value.filter(|s| !s.trim().is_empty())
}

/// username/passwordは「両方空(未設定のまま) or 両方入力」のみ許可する。
/// 片方だけの入力はVALIDATION_ERROR
fn validate_registry_config(config: &RegistryConfig) -> Result<(), AppError> {
    let username_present = config.registry_username.is_some();
    let password_present = config.registry_password.is_some();
    if username_present != password_present {
        let field = if username_present {
            "registry_password"
        } else {
            "registry_username"
        };
        return Err(AppError::validation_single(
            field,
            "ユーザー名とパスワードは両方入力するか、両方空にしてください",
        ));
    }
    Ok(())
}

/// domain/https_redirect/registry_url/api_tokenのみを検証する。
/// dns_provider/acme_emailはこの画面(基本設定)では変更できないため対象外
/// (専用の`validate_dns_config`を参照)。registry_urlは`apply_registry_url_default`が
/// domainから自動補完するため、ここでは空チェックしない(domainが空ならdomain自身の
/// エラーだけでブロックされ、registry_urlにも同じ原因で二重にエラーを出す必要がない)
fn validate(settings: &Settings) -> Result<(), AppError> {
    let mut errors = Vec::new();
    if settings.domain.trim().is_empty() {
        errors.push(crate::error::FieldError {
            field: "domain".to_string(),
            message: "ドメインを入力してください".to_string(),
        });
    }
    if settings.api_token.trim().is_empty() {
        errors.push(crate::error::FieldError {
            field: "api_token".to_string(),
            message: "APIトークンを入力してください".to_string(),
        });
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(AppError::Validation(errors))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::test_support::{test_state, test_state_unconfigured};

    fn valid_settings() -> Settings {
        Settings {
            domain: "example.com".to_string(),
            https_redirect: true,
            registry_url: "registry.sahai.example.com".to_string(),
            api_token: "tok".to_string(),
            dns_provider: "cloudflare".to_string(),
            acme_email: "admin@example.com".to_string(),
            registry_username: None,
            registry_password: None,
        }
    }

    #[test]
    fn validate_accepts_fully_populated_settings() {
        assert!(validate(&valid_settings()).is_ok());
    }

    #[test]
    fn validate_rejects_empty_domain() {
        let mut s = valid_settings();
        s.domain = "".to_string();
        let err = validate(&s).unwrap_err();
        match err {
            AppError::Validation(fields) => {
                assert!(fields.iter().any(|f| f.field == "domain"));
            }
            _ => panic!("Validationエラーを期待"),
        }
    }

    #[test]
    fn validate_accumulates_multiple_missing_fields() {
        let s = Settings {
            domain: "".to_string(),
            https_redirect: true,
            registry_url: "".to_string(),
            api_token: "".to_string(),
            dns_provider: "cloudflare".to_string(),
            acme_email: "".to_string(),
            registry_username: None,
            registry_password: None,
        };
        let err = validate(&s).unwrap_err();
        match err {
            AppError::Validation(fields) => {
                assert_eq!(fields.len(), 2, "domain・api_tokenの2件のはず: {fields:?}");
            }
            _ => panic!("Validationエラーを期待"),
        }
    }

    #[test]
    fn apply_registry_url_default_fills_from_domain_when_empty() {
        let mut s = valid_settings();
        s.registry_url = "".to_string();
        apply_registry_url_default(&mut s);
        assert_eq!(s.registry_url, "registry.sahai.example.com");
    }

    #[test]
    fn apply_registry_url_default_preserves_explicit_value() {
        let mut s = valid_settings();
        s.registry_url = "custom-registry.sahai.example.com".to_string();
        apply_registry_url_default(&mut s);
        assert_eq!(s.registry_url, "custom-registry.sahai.example.com");
    }

    #[test]
    fn apply_registry_url_default_leaves_empty_when_domain_also_empty() {
        let mut s = valid_settings();
        s.domain = "".to_string();
        s.registry_url = "".to_string();
        apply_registry_url_default(&mut s);
        assert_eq!(s.registry_url, "");
    }

    #[tokio::test]
    async fn is_configured_is_false_when_api_token_empty() {
        let state = test_state_unconfigured().await;
        assert!(!is_configured(&state).await);
    }

    #[tokio::test]
    async fn is_configured_is_true_when_api_token_set() {
        let state = test_state().await;
        assert!(is_configured(&state).await);
    }

    #[tokio::test]
    async fn setup_succeeds_when_unconfigured_and_reflects_immediately() {
        let state = test_state_unconfigured().await;
        let saved = setup(&state, valid_settings()).await.unwrap();

        assert_eq!(saved.domain, "example.com");
        assert!(is_configured(&state).await);
        assert_eq!(state.settings.read().await.domain, "example.com");
    }

    #[tokio::test]
    async fn setup_rejects_when_already_configured() {
        let state = test_state().await;
        let err = setup(&state, valid_settings()).await.unwrap_err();
        assert!(matches!(err, AppError::Conflict(_)));
    }

    #[tokio::test]
    async fn setup_validates_like_update() {
        let state = test_state_unconfigured().await;
        let mut s = valid_settings();
        s.domain = "".to_string();
        let err = setup(&state, s).await.unwrap_err();
        assert!(matches!(err, AppError::Validation(_)));
    }

    #[tokio::test]
    async fn setup_auto_fills_registry_url_when_empty() {
        let state = test_state_unconfigured().await;
        let mut s = valid_settings();
        s.registry_url = "".to_string();
        let saved = setup(&state, s).await.unwrap();

        assert_eq!(saved.registry_url, "registry.sahai.example.com");
        assert_eq!(
            state.settings.read().await.registry_url,
            "registry.sahai.example.com"
        );
    }

    #[tokio::test]
    async fn update_auto_fills_registry_url_when_empty() {
        let state = test_state().await;
        let mut s = valid_settings();
        s.registry_url = "".to_string();
        let saved = update(&state, s).await.unwrap();

        assert_eq!(saved.registry_url, "registry.sahai.example.com");
        assert_eq!(
            state.settings.read().await.registry_url,
            "registry.sahai.example.com"
        );
    }

    fn valid_dns_config() -> DnsConfig {
        DnsConfig {
            dns_provider: "cloudflare".to_string(),
            acme_email: "admin@example.com".to_string(),
            credentials: vec![("CF_DNS_API_TOKEN".to_string(), "secret".to_string())],
        }
    }

    async fn write_test_env_file(state: &AppState) {
        let path = &state.config.env_file_path;
        tokio::fs::create_dir_all(path.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(path, "SAHAI_DOMAIN=example.com\n")
            .await
            .unwrap();
    }

    #[test]
    fn validate_dns_config_accepts_fully_populated() {
        assert!(validate_dns_config(&valid_dns_config()).is_ok());
    }

    #[test]
    fn validate_dns_config_rejects_empty_provider() {
        let mut c = valid_dns_config();
        c.dns_provider = "".to_string();
        let err = validate_dns_config(&c).unwrap_err();
        match err {
            AppError::Validation(fields) => {
                assert!(fields.iter().any(|f| f.field == "dns_provider"))
            }
            _ => panic!("Validationエラーを期待"),
        }
    }

    #[test]
    fn validate_dns_config_rejects_empty_credential_key() {
        let mut c = valid_dns_config();
        c.credentials.push(("".to_string(), "value".to_string()));
        let err = validate_dns_config(&c).unwrap_err();
        match err {
            AppError::Validation(fields) => {
                assert!(fields.iter().any(|f| f.field == "credentials[1].key"));
            }
            _ => panic!("Validationエラーを期待"),
        }
    }

    #[tokio::test]
    async fn get_dns_config_reads_from_settings_and_credentials_table() {
        let state = test_state().await;
        settings_repo::replace_dns_provider_credentials(
            state.db.pool(),
            &[("CF_DNS_API_TOKEN".to_string(), "secret".to_string())],
        )
        .await
        .unwrap();

        let config = get_dns_config(&state).await.unwrap();

        assert_eq!(config.dns_provider, "cloudflare");
        assert_eq!(
            config.credentials,
            vec![("CF_DNS_API_TOKEN".to_string(), "secret".to_string())]
        );
    }

    #[tokio::test]
    async fn update_dns_config_persists_to_db_and_env_even_if_traefik_recreate_fails() {
        // compose_file_pathがtest_state()では実在しないため、Traefik再作成ステップは
        // 必ず失敗する。ただしDB・.envへの反映は既に完了しているべき、という設計意図
        // (update_dns_configのドキュメントコメント参照)を検証する
        let state = test_state().await;
        write_test_env_file(&state).await;

        let result = update_dns_config(&state, valid_dns_config()).await;

        assert!(result.is_err());
        assert_eq!(state.settings.read().await.dns_provider, "cloudflare");

        let env_content = tokio::fs::read_to_string(&state.config.env_file_path)
            .await
            .unwrap();
        assert!(env_content.contains("SAHAI_DNS_PROVIDER=cloudflare"));
        assert!(env_content.contains("CF_DNS_API_TOKEN=secret"));

        let credentials = settings_repo::list_dns_provider_credentials(state.db.pool())
            .await
            .unwrap();
        assert_eq!(
            credentials,
            vec![("CF_DNS_API_TOKEN".to_string(), "secret".to_string())]
        );
    }

    #[tokio::test]
    async fn update_dns_config_rejects_invalid_input_without_touching_env_file() {
        let state = test_state().await;
        write_test_env_file(&state).await;
        let mut c = valid_dns_config();
        c.dns_provider = "".to_string();

        let err = update_dns_config(&state, c).await.unwrap_err();
        assert!(matches!(err, AppError::Validation(_)));

        let env_content = tokio::fs::read_to_string(&state.config.env_file_path)
            .await
            .unwrap();
        assert!(
            !env_content.contains("SAHAI_DNS_PROVIDER"),
            "バリデーション失敗時は.envに触れないはず"
        );
    }

    #[tokio::test]
    async fn update_dns_config_creates_env_file_when_missing() {
        let state = test_state().await;
        // write_test_env_fileを呼ばない。sahai_data/サブディレクトリ自体も
        // 事前に作らない(初回チェックアウト直後を模す)。repoディレクトリだけは
        // test_state()側で用意されている前提。
        assert!(
            !state.config.env_file_path.exists(),
            "テスト前提: .sahai.envはまだ存在しないはず"
        );

        // compose_file_pathがtest_state()では実在しないため、Traefik再作成ステップは
        // 必ず失敗する。ただしファイルの自動作成自体はTraefik再作成より前に行われる
        // ため、そこは成功しているべき、という設計意図を検証する
        let result = update_dns_config(&state, valid_dns_config()).await;
        assert!(result.is_err());

        let env_content = tokio::fs::read_to_string(&state.config.env_file_path)
            .await
            .expect(".sahai.envがディレクトリごと自動作成されているはず");
        assert!(env_content.contains("SAHAI_DNS_PROVIDER=cloudflare"));
        assert!(env_content.contains("CF_DNS_API_TOKEN=secret"));
    }

    fn valid_registry_config() -> RegistryConfig {
        RegistryConfig {
            registry_url: "registry.sahai.example.com".to_string(),
            registry_username: Some("reguser".to_string()),
            registry_password: Some("regpass".to_string()),
            login_warning: None,
        }
    }

    #[tokio::test]
    async fn get_registry_config_reads_from_settings() {
        let state = test_state().await;
        {
            let mut current = state.settings.write().await;
            current.registry_url = "registry.example.test".to_string();
            current.registry_username = Some("u".to_string());
            current.registry_password = Some("p".to_string());
        }

        let config = get_registry_config(&state).await;

        assert_eq!(config.registry_url, "registry.example.test");
        assert_eq!(config.registry_username.as_deref(), Some("u"));
        assert_eq!(config.registry_password.as_deref(), Some("p"));
        assert_eq!(config.login_warning, None);
    }

    #[test]
    fn validate_registry_config_rejects_username_without_password() {
        let mut c = valid_registry_config();
        c.registry_password = None;
        let err = validate_registry_config(&c).unwrap_err();
        match err {
            AppError::Validation(fields) => {
                assert!(fields.iter().any(|f| f.field == "registry_password"));
            }
            _ => panic!("Validationエラーを期待"),
        }
    }

    #[test]
    fn validate_registry_config_rejects_password_without_username() {
        let mut c = valid_registry_config();
        c.registry_username = None;
        let err = validate_registry_config(&c).unwrap_err();
        match err {
            AppError::Validation(fields) => {
                assert!(fields.iter().any(|f| f.field == "registry_username"));
            }
            _ => panic!("Validationエラーを期待"),
        }
    }

    #[test]
    fn validate_registry_config_accepts_both_empty() {
        let mut c = valid_registry_config();
        c.registry_username = None;
        c.registry_password = None;
        assert!(validate_registry_config(&c).is_ok());
    }

    #[test]
    fn validate_registry_config_accepts_both_present() {
        assert!(validate_registry_config(&valid_registry_config()).is_ok());
    }

    #[tokio::test]
    async fn update_registry_config_persists_to_db_even_if_docker_login_fails() {
        // registry.sahai.example.comには実際には到達できないため、docker loginは必ず失敗する。
        // それでもDB保存自体は成功し、失敗理由がlogin_warningに載ることを検証する
        // (update_dns_configの「Traefik再作成失敗時は500」とはあえて異なる設計。
        // ユーザー確定方針)
        let state = test_state().await;

        let result = update_registry_config(&state, valid_registry_config()).await;

        let saved = result.expect("docker login失敗時もDB保存自体は成功するはず");
        assert_eq!(saved.registry_url, "registry.sahai.example.com");
        assert!(
            saved.login_warning.is_some(),
            "docker login失敗の警告が載るはず"
        );
        assert_eq!(
            state.settings.read().await.registry_username.as_deref(),
            Some("reguser")
        );

        let row = crate::repo::settings::load(state.db.pool())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.registry_username.as_deref(), Some("reguser"));
        assert_eq!(row.registry_password.as_deref(), Some("regpass"));
    }

    #[tokio::test]
    async fn update_registry_config_auto_fills_registry_url_when_empty() {
        let state = test_state().await;
        let mut c = valid_registry_config();
        c.registry_url = "".to_string();
        c.registry_username = None;
        c.registry_password = None;

        let saved = update_registry_config(&state, c).await.unwrap();

        // test_state()のdomainは"example.com"(service::test_support::test_settings参照)
        assert_eq!(saved.registry_url, "registry.sahai.example.com");
        assert_eq!(
            state.settings.read().await.registry_url,
            "registry.sahai.example.com"
        );
    }

    #[tokio::test]
    async fn update_registry_config_clears_credentials_when_both_empty() {
        let state = test_state().await;
        update_registry_config(&state, valid_registry_config())
            .await
            .unwrap();

        let mut cleared = valid_registry_config();
        cleared.registry_username = Some("".to_string());
        cleared.registry_password = Some("".to_string());
        let saved = update_registry_config(&state, cleared).await.unwrap();

        assert_eq!(saved.registry_username, None);
        assert_eq!(saved.registry_password, None);
        assert_eq!(
            saved.login_warning, None,
            "資格情報が空ならdocker loginは実行されないはず"
        );
        assert_eq!(state.settings.read().await.registry_username, None);
    }
}
