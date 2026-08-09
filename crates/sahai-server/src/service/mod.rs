//! ドメイン層。オーケストレーションとビジネスルール。
//! sahai-coreのロジックをDB/Docker/Traefikと組み合わせる。

pub mod compose_sync;
pub mod deletion;
pub mod lifecycle;
pub mod port_check;
pub mod registration;
pub mod settings;
pub mod update;
pub mod upload;

use crate::domain::{ContainerWithChildren, ServiceDetail};
use crate::error::AppError;
use crate::repo::{containers, ports, services, volumes};
use crate::state::AppState;

/// `{id_or_name}`からServiceDetailを組み立てる。repo層の複数クエリを束ねるだけの
/// 薄いヘルパーで、ビジネスルールは持たない。
pub async fn load_detail(state: &AppState, id_or_name: &str) -> Result<ServiceDetail, AppError> {
    let row = services::find_by_id_or_name(state.db.pool(), id_or_name)
        .await?
        .ok_or_else(|| AppError::NotFound(id_or_name.to_string()))?;
    load_detail_by_id(state, row.id).await
}

pub async fn load_detail_by_id(
    state: &AppState,
    service_id: i64,
) -> Result<ServiceDetail, AppError> {
    let row = services::find_by_id(state.db.pool(), service_id)
        .await?
        .ok_or_else(|| AppError::NotFound(service_id.to_string()))?;
    let service = crate::domain::Service::try_from(row).map_err(AppError::Internal)?;

    let container_rows = containers::list_by_service(state.db.pool(), service_id).await?;
    let mut container_list = Vec::with_capacity(container_rows.len());
    for row in container_rows {
        let container_id = row.id;
        let container =
            crate::domain::ServiceContainer::try_from(row).map_err(AppError::Internal)?;

        let port_rows = ports::list_by_container(state.db.pool(), container_id).await?;
        let mut port_list = Vec::with_capacity(port_rows.len());
        for p in port_rows {
            port_list.push(crate::domain::ServicePort::try_from(p).map_err(AppError::Internal)?);
        }

        let volume_rows = volumes::list_by_container(state.db.pool(), container_id).await?;
        let volume_list = volume_rows.into_iter().map(Into::into).collect();

        container_list.push(ContainerWithChildren {
            container,
            ports: port_list,
            volumes: volume_list,
        });
    }

    Ok(ServiceDetail {
        service,
        containers: container_list,
        route_warning: None,
    })
}

/// service層のTDDで共有するテストヘルパー。DBは実SQLite(一時ファイル)に対して行う。
/// `DockerClients::connect_for_test`は実Dockerデーモンに一切到達できないクライアントを
/// 使う(実際に稼働中のコンテナをテストが誤って操作しないため。
/// `docker::mod::unreachable_docker_client_for_test`参照)。
#[cfg(test)]
pub mod test_support {
    use std::sync::Arc;

    use tokio::sync::RwLock;

    use crate::config::Config;
    use crate::docker::DockerClients;
    use crate::repo::Db;
    use crate::settings::{Settings, SharedSettings};
    use crate::state::{AppState, AppStateInner};
    use crate::traefik::RouteWriter;

    pub fn test_settings() -> SharedSettings {
        Arc::new(RwLock::new(Settings {
            domain: "example.com".to_string(),
            https_redirect: true,
            registry_url: "registry.example.test".to_string(),
            api_token: "test".to_string(),
            dns_provider: "cloudflare".to_string(),
            acme_email: "admin@example.test".to_string(),
            registry_username: None,
            registry_password: None,
        }))
    }

    pub async fn test_state() -> AppState {
        let dir = std::env::temp_dir().join(format!("sahai_svc_test_{}", unique_suffix()));
        std::fs::create_dir_all(&dir).unwrap();
        let config = Config {
            bind_addr: "127.0.0.1:0".to_string(),
            database_path: dir.join("test.sqlite3"),
            // 静的ファイル配信のTDDで実際にindex.html等を置けるよう、テスト専用の
            // 一時ディレクトリ配下を指す(api::router_testsのstatic serving系テスト参照)
            web_dist_dir: dir.join("web_dist"),
            // DNS設定(.sahai.env書き込み)のTDDで実際にファイルを置けるよう、
            // テスト専用の一時ディレクトリ配下を指す(service::settings::dns系テスト参照)
            env_file_path: dir.join(".sahai.env"),
            sahai_data_root: dir,
        };
        let settings = test_settings();

        // max_connections(1)で固定し、ImmediateTransactionのcommit/rollback漏れを
        // 同一コネクションの再利用で確実に検出できるようにする。
        // マイグレーションはDb::connect_for_test内で実行される
        let db = Db::connect_for_test(&config).await.unwrap();

        // settingsテーブルにもtest_settings()と同じ内容を投入しておく(本番のmain.rsの
        // 起動シーケンスを模す)。service::settings::update等がDBのUPDATEを前提とする
        // ため、行が存在しないと更新が0件ヒットのまま静かに失敗してしまう
        {
            let seeded = settings.read().await.clone();
            crate::repo::settings::seed(db.pool(), &(&seeded).into())
                .await
                .unwrap();
        }

        let docker =
            DockerClients::connect_for_test(settings.clone(), config.sahai_data_root.clone());

        let traefik = RouteWriter::new(
            config.traefik_dynamic_dir(),
            "host.docker.internal".to_string(),
            "http://sahai-server:8080".to_string(),
            "letsencrypt".to_string(),
            settings.clone(),
        );

        Arc::new(AppStateInner {
            config,
            settings,
            db,
            docker,
            traefik,
            registry_internal_url: "http://registry:5000".to_string(),
        })
    }

    /// 初回セットアップ未完了(DBに行が無い・api_tokenが空)を模したstate。
    /// service::settings::setup()のTDDで使う
    pub async fn test_state_unconfigured() -> AppState {
        let dir = std::env::temp_dir().join(format!("sahai_svc_test_{}", unique_suffix()));
        std::fs::create_dir_all(&dir).unwrap();
        let config = Config {
            bind_addr: "127.0.0.1:0".to_string(),
            database_path: dir.join("test.sqlite3"),
            // 静的ファイル配信のTDDで実際にindex.html等を置けるよう、テスト専用の
            // 一時ディレクトリ配下を指す(api::router_testsのstatic serving系テスト参照)
            web_dist_dir: dir.join("web_dist"),
            // DNS設定(.sahai.env書き込み)のTDDで実際にファイルを置けるよう、
            // テスト専用の一時ディレクトリ配下を指す(service::settings::dns系テスト参照)
            env_file_path: dir.join(".sahai.env"),
            sahai_data_root: dir,
        };
        let settings: SharedSettings = Arc::new(RwLock::new(Settings {
            domain: String::new(),
            https_redirect: true,
            registry_url: String::new(),
            api_token: String::new(),
            dns_provider: "cloudflare".to_string(),
            acme_email: String::new(),
            registry_username: None,
            registry_password: None,
        }));

        // DBには行を投入しない(未セットアップ状態そのもの)
        let db = Db::connect_for_test(&config).await.unwrap();

        let docker =
            DockerClients::connect_for_test(settings.clone(), config.sahai_data_root.clone());

        let traefik = RouteWriter::new(
            config.traefik_dynamic_dir(),
            "host.docker.internal".to_string(),
            "http://sahai-server:8080".to_string(),
            "letsencrypt".to_string(),
            settings.clone(),
        );

        Arc::new(AppStateInner {
            config,
            settings,
            db,
            docker,
            traefik,
            registry_internal_url: "http://registry:5000".to_string(),
        })
    }

    fn unique_suffix() -> String {
        format!(
            "{:x}_{:?}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            std::thread::current().id()
        )
    }
}
