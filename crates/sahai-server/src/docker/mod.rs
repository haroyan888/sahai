//! Docker操作層。ライフサイクル操作(start/stop)はimage型/compose型で実装を分けるが、
//! 参照系操作(inspect/stats)は実コンテナ名が`svc-{container_id}`に統一されているため
//! type分岐が不要という非対称性がある。

pub mod build_runtime;
pub mod compose_runtime;
pub mod image_runtime;
pub mod inspector;
pub mod log_stream;
pub mod override_gen;
pub mod registry_login;

use async_trait::async_trait;

use crate::domain::{ServiceDetail, SourceType};
use crate::settings::SharedSettings;

pub use inspector::{ContainerRuntimeState, HealthObservation, Inspector};

#[derive(Debug, thiserror::Error)]
pub enum DockerError {
    #[error("Docker操作に失敗しました: {0}")]
    Bollard(#[from] bollard::errors::Error),
    #[error("docker composeの実行に失敗しました: {0}")]
    ComposeExec(String),
    #[error("{0}")]
    BuildExec(String),
    #[error("{0}")]
    Other(String),
}

/// image型・compose型それぞれのライフサイクル(起動・停止)を抽象化するトレイト。
/// 選択ロジックは本モジュールの`runtime_for`が持ち、呼び出し側(service::lifecycle)は
/// source_typeによるif/elseを持たない。
#[async_trait]
pub trait ContainerLifecycle: Send + Sync {
    async fn start(&self, service: &ServiceDetail) -> Result<(), DockerError>;
    async fn stop(&self, service: &ServiceDetail) -> Result<(), DockerError>;
}

pub struct DockerClients {
    /// Traefikコンテナの再作成(bollard直接操作)にも使う共有クライアント
    /// (`traefik::container::recreate_traefik`参照)。
    pub docker: bollard::Docker,
    pub image_runtime: image_runtime::ImageRuntime,
    pub compose_runtime: compose_runtime::ComposeRuntime,
    pub inspector: Inspector,
}

impl DockerClients {
    /// `sahai_data_root`はsahai-server自身のファイルI/O用、`host_data_root`は
    /// dockerdへbindマウント元として渡すためのもの。開発時のみ食い違う(config.rs参照)。
    pub fn connect(
        settings: SharedSettings,
        sahai_data_root: std::path::PathBuf,
        host_data_root: std::path::PathBuf,
    ) -> Result<Self, DockerError> {
        let docker = bollard::Docker::connect_with_local_defaults()?;
        Ok(Self::from_docker(
            docker,
            settings,
            sahai_data_root,
            host_data_root,
        ))
    }

    /// テスト専用: 実Dockerデーモンに一切到達できないクライアントで構築する。
    /// `service::mod::test_state()`等の単体テストヘルパーはこちらを使う。
    /// `traefik::recreate_traefik`はラベルでコンテナを検索するbollard直接操作のため、
    /// 実デーモンに接続すると開発機で稼働中のTraefikコンテナを誤って再作成しかねない。
    /// 単体テストは「Traefik再作成に失敗すること」だけを前提にしており実際の再作成
    /// 成功を検証する意図はないため、実デーモンに触れる必要はない(実際のTraefik
    /// 再作成を検証するテストは`traefik::container`の`#[ignore]`付きe2eテストが担う)。
    #[cfg(test)]
    pub fn connect_for_test(settings: SharedSettings, sahai_data_root: std::path::PathBuf) -> Self {
        Self::from_docker(
            unreachable_docker_client_for_test(),
            settings,
            sahai_data_root.clone(),
            sahai_data_root,
        )
    }

    fn from_docker(
        docker: bollard::Docker,
        settings: SharedSettings,
        sahai_data_root: std::path::PathBuf,
        host_data_root: std::path::PathBuf,
    ) -> Self {
        DockerClients {
            docker: docker.clone(),
            image_runtime: image_runtime::ImageRuntime::new(
                docker.clone(),
                host_data_root.clone(),
                settings.clone(),
            ),
            compose_runtime: compose_runtime::ComposeRuntime::new(
                settings,
                sahai_data_root,
                host_data_root,
            ),
            inspector: Inspector::new(docker),
        }
    }

    /// `source_type`に応じたライフサイクル実装を返すファクトリ。
    pub fn runtime_for(&self, source_type: SourceType) -> &dyn ContainerLifecycle {
        match source_type {
            SourceType::Image => &self.image_runtime,
            SourceType::Compose => &self.compose_runtime,
        }
    }
}

/// 実Dockerデーモンに一切到達できない`bollard::Docker`クライアントを作る。
/// 接続オブジェクトの構築自体はbollardの遅延評価特性により失敗しないが、
/// 以降のAPI呼び出しは全て接続エラーになる(`DockerClients::connect_for_test`、
/// `traefik::container`のテスト参照)。
#[cfg(test)]
pub fn unreachable_docker_client_for_test() -> bollard::Docker {
    #[cfg(windows)]
    {
        bollard::Docker::connect_with_named_pipe(
            "npipe://./pipe/sahai_test_unreachable",
            5,
            bollard::API_DEFAULT_VERSION,
        )
        .expect("到達不能クライアントの構築自体は失敗しないはず(bollardの遅延評価)")
    }
    #[cfg(not(windows))]
    {
        bollard::Docker::connect_with_unix(
            "unix:///nonexistent/sahai_test_unreachable.sock",
            5,
            bollard::API_DEFAULT_VERSION,
        )
        .expect("到達不能クライアントの構築自体は失敗しないはず(bollardの遅延評価)")
    }
}
