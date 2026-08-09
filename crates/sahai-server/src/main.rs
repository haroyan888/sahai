mod api;
mod auth;
mod config;
mod docker;
mod domain;
mod env_file;
mod error;
mod fs_perms;
mod health;
mod repo;
mod service;
mod settings;
mod setup_token;
mod state;
mod traefik;

use std::sync::Arc;

use config::Config;
use docker::DockerClients;
use health::HealthTask;
use repo::Db;
use settings::Settings;
use state::AppStateInner;
use tokio::sync::RwLock;
use traefik::RouteWriter;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let config = Config::from_env()?;
    tracing::info!("設定を読み込みました: bind={}", config.bind_addr);

    // データルート配下には平文の秘匿値を含むファイルが並ぶため、ディレクトリ自体も
    // 所有者限定にする(ファイルが読めなくてもサービス名・IDが列挙されるのを防ぐ)。
    // Db::connectがDBファイルを作るより前に適用する
    if let Err(e) = tokio::fs::create_dir_all(&config.sahai_data_root).await {
        tracing::warn!("データルートの作成に失敗しました: {e}");
    } else if let Err(e) = fs_perms::secure_dir(&config.sahai_data_root).await {
        tracing::warn!("データルートのパーミッション設定に失敗しました: {e}");
    }

    // マイグレーションはDb::connect内で(外部キー制約を無効にした専用コネクションで)
    // 実行される(repo/mod.rs::Db::run_migrations参照)
    let db = Db::connect(&config).await?;
    tracing::info!("マイグレーションを適用しました");

    // domain・https_redirect・registry_url・api_token・DNSプロバイダ関連はDBに永続化し、
    // Web UIの設定画面から保存後すぐ変更できるようにする(settings.rs参照)。
    // 初回起動でDBがまだ空の場合、環境変数に有効な値があればそこからシードし、
    // 無ければクラッシュさせず「未設定」のまま起動する(セットアップスクリプトが
    // POST /api/setupで投入するまで待つ)。api_token・domainのどちらかが空なら
    // シードしない(中途半端に設定済みと判定されるのを避けるため。settings.rs参照)
    let initial_settings = match repo::settings::load(db.pool()).await? {
        Some(row) => {
            tracing::info!("設定をDBから読み込みました");
            Settings::from(row)
        }
        None => match Settings::seed_from_env() {
            Ok(seed) if !seed.api_token.trim().is_empty() && !seed.domain.trim().is_empty() => {
                repo::settings::seed(db.pool(), &(&seed).into()).await?;
                tracing::info!("設定を環境変数からDBへ初期投入しました");
                seed
            }
            _ => {
                tracing::info!("設定が未投入です。セットアップスクリプトからの初期設定を待ちます");
                Settings::unconfigured()
            }
        },
    };
    let is_configured = !initial_settings.domain.trim().is_empty();

    // 初期設定を第三者に先取りされないよう、未設定の間だけワンタイムトークンを発行し
    // POST /api/setupで提示を要求する。値はログに出さない
    if is_configured {
        if let Err(e) = setup_token::revoke(&config.sahai_data_root).await {
            tracing::warn!("セットアップトークンの削除に失敗しました: {e}");
        }
    } else if let Err(e) = setup_token::issue(&config.sahai_data_root).await {
        // 発行できないと初期設定が一切通らなくなるため、原因が分かるようエラーで残す
        tracing::error!("セットアップトークンの発行に失敗しました。初期設定を実行できません: {e}");
    } else {
        tracing::info!(
            "セットアップトークンを発行しました: {}",
            config.sahai_data_root.join("setup-token").display()
        );
    }

    let settings: settings::SharedSettings = Arc::new(RwLock::new(initial_settings));

    let docker = DockerClients::connect(settings.clone(), config.sahai_data_root.clone())
        .map_err(|e| e.to_string())?;

    // `sahai service create`(サーバー側build+push)用の資格情報がDBに設定されていれば、
    // 起動時に一度だけdocker loginしておく(以降のpushはコンテナ内の
    // ~/.docker/config.jsonに保存された資格情報を使い回す)。Web UIの「レジストリ設定」
    // カードから保存時にも同じログインを試みる(service::settings::update_registry_config
    // 参照)。未設定運用(このコマンドを使わない場合)を壊さないよう、失敗は警告ログのみに留める
    let (registry_username, registry_password, registry_url_for_login) = {
        let s = settings.read().await;
        (
            s.registry_username.clone(),
            s.registry_password.clone(),
            s.registry_url.clone(),
        )
    };
    if let (Some(username), Some(password)) = (registry_username, registry_password) {
        match docker::registry_login::login(&registry_url_for_login, &username, &password).await {
            Ok(()) => {
                tracing::info!("レジストリへのログインに成功しました: {registry_url_for_login}")
            }
            Err(e) => tracing::warn!("レジストリへのログインに失敗しました: {e}"),
        }
    }

    // `sahai service create`のアップロード一時展開先を起動時にクリーンアップする。
    // 同期処理でジョブキューを持たないため、プロセスが生きている間しか「進行中の
    // アップロード」は存在しえない=再起動直後は必ず空にしてよい
    // (ビルド途中のクラッシュ・再起動時に残留したディレクトリの後始末)
    let uploads_dir = config.sahai_data_root.join("uploads");
    if let Err(e) = tokio::fs::remove_dir_all(&uploads_dir).await {
        if e.kind() != std::io::ErrorKind::NotFound {
            tracing::warn!("uploads一時ディレクトリの初期化に失敗しました: {e}");
        }
    }
    if let Err(e) = tokio::fs::create_dir_all(&uploads_dir).await {
        tracing::warn!("uploads一時ディレクトリの作成に失敗しました: {e}");
    }

    // sahai-server自身のdocker-compose上のアドレス。Web UI(静的ファイル)とAPIを
    // 同一コンテナが配信するため、管理画面ルートの転送先・is_httpポートを持たない
    // サービスの転送先の両方に使う。sahai-serverは常に
    // 自分自身を指すため環境変数での上書きは不要な固定値にしている
    let app_internal_url = "http://sahai-server:8080".to_string();

    let traefik = RouteWriter::new(
        config.traefik_dynamic_dir(),
        app_internal_url,
        // traefik.yml参照: certificatesResolversの名前はDNSプロバイダによらず固定
        // (プロバイダの切り替えはcompose.yamlのcommand引数/環境変数で行う)
        "letsencrypt".to_string(),
        settings.clone(),
    );

    // 管理画面(sahai.<domain>)のルート+未登録サブドメイン用のワイルドカード
    // catch-allルートを起動時に一度だけ書き出す。
    // 以前はリポジトリ管理の静的YAMLをbind-mountしていたが、Dockerの
    // 「read-onlyマウント済みディレクトリの中に単一ファイルを重ねてマウントできない」
    // という制約を実機で踏んだため、他の動的ルートと同じ書き込み方式に統一した。

    // registryコンテナのcompose上のアドレス。registry.sahai.<domain>宛のルートの転送先になる
    let registry_internal_url = std::env::var("SAHAI_REGISTRY_INTERNAL_URL")
        .unwrap_or_else(|_| "http://registry:5000".to_string());
    // 未設定状態(domainが空)ではHost()ルールを組み立てられないため、domainに依存しない
    // 暫定ルートを書き出す(Web UI/APIへ到達させ、セットアップ未完了の案内を出すため)。
    // 初期設定完了時にservice::settings::setup()が通常のdomainベースのルートで上書きする
    let admin_routes_result = if is_configured {
        traefik
            .write_static_admin_routes(&registry_internal_url)
            .await
    } else {
        traefik.write_bootstrap_routes().await
    };
    if let Err(e) = admin_routes_result {
        tracing::warn!("管理画面用の静的Traefikルートの書き出しに失敗しました: {e}");
    }

    let inspector_for_health = docker::Inspector::new(
        bollard::Docker::connect_with_local_defaults().map_err(|e| e.to_string())?,
    );
    let health_db = db.clone();

    let state: state::AppState = Arc::new(AppStateInner {
        config,
        settings,
        db,
        docker,
        traefik,
        registry_internal_url,
    });

    // ヘルスチェックはAPIハンドラとは独立したバックグラウンドタスクとして起動する
    tokio::spawn(HealthTask::new(health_db, inspector_for_health).run_forever());

    let app = api::router(state.clone());
    let listener = tokio::net::TcpListener::bind(&state.config.bind_addr).await?;
    tracing::info!("起動しました: {}", state.config.bind_addr);
    axum::serve(listener, app).await?;

    Ok(())
}
