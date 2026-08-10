//! ComposeRuntime: `docker compose`サブプロセスでcompose型サービスをup/downする。
//! image型はbollard(Docker Engine API)を使うが、bollardはdocker-compose操作を
//! サポートしないため、ここだけCLIをサブプロセスとして呼ぶ。

use std::path::PathBuf;

use async_trait::async_trait;
use tokio::process::Command;

use crate::domain::ServiceDetail;
use crate::settings::SharedSettings;

use super::{override_gen, ContainerLifecycle, DockerError};

pub struct ComposeRuntime {
    settings: SharedSettings,
    sahai_data_root: PathBuf,
}

/// `docker compose down`が使えないときの後始末。コンテナ名は`svc-{id}`で決まるため、
/// compose定義を読まずに削除できる。composeが作ったネットワークも消す。
///
/// 存在しないものへの削除は失敗するが、結果として消えていればよいので無視する。
async fn force_remove_containers(
    service: &ServiceDetail,
    project: &str,
) -> Result<(), DockerError> {
    for container in &service.containers {
        let name = sahai_core::naming::container_docker_name(container.container.id);
        let output = Command::new("docker")
            .args(["rm", "-f", &name])
            .output()
            .await
            .map_err(|e| DockerError::ComposeExec(e.to_string()))?;
        if !output.status.success() {
            tracing::debug!(
                "コンテナ{name}の削除をスキップしました: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
    }

    // composeが作った既定ネットワーク。コンテナを消した後でなければ削除できない
    let _ = Command::new("docker")
        .args(["network", "rm", &format!("{project}_default")])
        .output()
        .await;
    Ok(())
}

impl ComposeRuntime {
    pub fn new(settings: SharedSettings, sahai_data_root: PathBuf) -> Self {
        ComposeRuntime {
            settings,
            sahai_data_root,
        }
    }

    /// base.yml/override.yml/.env の書き出し先(composeプロジェクトのルート)。
    fn project_dir(&self, service_id: i64) -> PathBuf {
        self.sahai_data_root
            .join("compose-projects")
            .join(service_id.to_string())
    }

    async fn write_project_files(
        &self,
        service: &ServiceDetail,
    ) -> Result<(PathBuf, PathBuf), DockerError> {
        let dir = self.project_dir(service.service.id);
        tokio::fs::create_dir_all(&dir)
            .await
            .map_err(|e| DockerError::Other(e.to_string()))?;

        let base_path = dir.join("base.yml");
        let override_path = dir.join("override.yml");
        let env_path = dir.join(".env");

        let compose_content = service
            .service
            .compose_content
            .as_deref()
            .ok_or_else(|| DockerError::Other("compose_contentが未設定です".to_string()))?;
        // 利用者が書いたports:とenv_file:は落としてから書き出す。どちらも差配が
        // 一元管理する。overrideでは打ち消せない(composeがこれらを合算するため)
        let base_yaml = sahai_core::compose::strip_managed_keys(compose_content)
            .map_err(DockerError::from_core)?;
        tokio::fs::write(&base_path, &base_yaml)
            .await
            .map_err(|e| DockerError::Other(e.to_string()))?;

        tokio::fs::write(
            &env_path,
            override_gen::generate_env_file_content(&service.service.env_vars),
        )
        .await
        .map_err(|e| DockerError::Other(e.to_string()))?;
        // env varsは平文のため、DBファイルと同じくパーミッションで防御する
        crate::fs_perms::secure_file(&env_path)
            .await
            .map_err(|e| DockerError::Other(e.to_string()))?;

        let build_service_names = sahai_core::compose::parse_build_service_names(compose_content)
            .map_err(DockerError::from_core)?;
        let registry_url = self.settings.read().await.registry_url.clone();
        let override_yaml = override_gen::generate_override_yaml(
            service,
            &registry_url,
            &build_service_names,
            &self.sahai_data_root,
            &env_path,
        )?;
        tokio::fs::write(&override_path, override_yaml)
            .await
            .map_err(|e| DockerError::Other(e.to_string()))?;

        Ok((base_path, override_path))
    }

    async fn run_compose(&self, args: &[&str]) -> Result<(), DockerError> {
        let output = Command::new("docker")
            .arg("compose")
            .args(args)
            .output()
            .await
            .map_err(|e| DockerError::ComposeExec(e.to_string()))?;
        if !output.status.success() {
            return Err(DockerError::ComposeExec(
                String::from_utf8_lossy(&output.stderr).to_string(),
            ));
        }
        Ok(())
    }
}

impl DockerError {
    fn from_core(e: sahai_core::CoreError) -> Self {
        DockerError::Other(e.to_string())
    }
}

#[async_trait]
impl ContainerLifecycle for ComposeRuntime {
    async fn start(&self, service: &ServiceDetail) -> Result<(), DockerError> {
        let (base_path, override_path) = self.write_project_files(service).await?;
        let project = sahai_core::naming::compose_project_name(service.service.id);
        let base = base_path.display().to_string();
        let overr = override_path.display().to_string();
        let file_args = [
            "-f",
            base.as_str(),
            "-f",
            overr.as_str(),
            "-p",
            project.as_str(),
        ];

        // up前に必ずイメージを取得し直す。`docker compose up`の既定
        // pull_policyは`missing`でローカルに在れば再取得しないため、これが無いと
        // `container push`で更新したイメージがあってもキャッシュの古い方で起動してしまう。
        // build:を持つサービスもoverrideでimage:を注入済みのためpull対象になる。
        // 取得に失敗しても起動自体は続行する(既製イメージのpullがレート制限等で
        // 失敗しても、キャッシュがあれば起動できるほうが可用性の面で望ましい)
        let pull_args: Vec<&str> = file_args.iter().copied().chain(["pull"]).collect();
        if let Err(e) = self.run_compose(&pull_args).await {
            tracing::warn!(
                "サービス{}のイメージ取得に失敗しました。ローカルキャッシュのイメージで起動します: {e}",
                service.service.name
            );
        }

        // build:を持つサービスもoverrideでimage:を注入済みのため、
        // --no-buildでローカル再ビルドを避ける(イメージはCLI/サーバー側で
        // ビルド・pushされたものを使う方針のため)
        let up_args: Vec<&str> = file_args
            .iter()
            .copied()
            .chain(["up", "-d", "--no-build", "--remove-orphans"])
            .collect();
        self.run_compose(&up_args).await
    }

    async fn stop(&self, service: &ServiceDetail) -> Result<(), DockerError> {
        let project = sahai_core::naming::compose_project_name(service.service.id);
        let dir = self.project_dir(service.service.id);
        let base_path = dir.join("base.yml");
        let override_path = dir.join("override.yml");

        // 一度もstartしていないサービスに対してもservice::deletion::deleteは
        // 無条件にstop()を呼ぶ。base.yml/override.ymlはstart()時に
        // しか書き出されないため、無い場合は「元々何も起動していない」に等しく
        // 成功として扱う(実機のWeb UIで未起動サービスの削除が永久に失敗する
        // バグとして発覚)
        if tokio::fs::metadata(&base_path).await.is_err() {
            return Ok(());
        }

        let down = self
            .run_compose(&[
                "-f",
                &base_path.display().to_string(),
                "-f",
                &override_path.display().to_string(),
                "-p",
                &project,
                "down",
            ])
            .await;

        if let Err(e) = down {
            // composeは設定エラーがあると何もせず失敗する。compose_contentの書き方を
            // 誤っているだけでサービスを消す手段が無くなってしまうため、DBが把握して
            // いるコンテナ名で直接削除へ切り替える
            tracing::warn!(
                "サービス{}のdocker compose downに失敗しました。コンテナを直接削除します: {e}",
                service.service.name
            );
            force_remove_containers(service, &project).await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::domain::{
        ContainerWithChildren, HealthStatus, Protocol, Service, ServiceContainer, ServiceDetail,
        ServicePort, ServiceStatus, SourceType,
    };

    use super::*;

    fn test_settings() -> SharedSettings {
        std::sync::Arc::new(tokio::sync::RwLock::new(crate::settings::Settings {
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

    fn compose_service_detail(
        service_id: i64,
        container_id: i64,
        host_port: Option<i64>,
    ) -> ServiceDetail {
        ServiceDetail {
            service: Service {
                id: service_id,
                name: "e2ecompose".to_string(),
                subdomain: "e2ecompose.example.com".to_string(),
                source_type: SourceType::Compose,
                image: None,
                // build:を持たない既製イメージのみ。
                // ローカルビルドが不要なのでE2Eテストが高速・確実になる
                compose_content: Some("services:\n  web:\n    image: nginx:alpine\n".to_string()),
                env_vars: serde_json::json!({}),
                status: ServiceStatus::Stopped,
                last_error: None,
                health_status: HealthStatus::Unknown,
                last_health_check_at: None,
                created_at: "2026-01-01T00:00:00.000Z".to_string(),
                updated_at: "2026-01-01T00:00:00.000Z".to_string(),
            },
            containers: vec![ContainerWithChildren {
                container: ServiceContainer {
                    id: container_id,
                    service_id,
                    name: "web".to_string(),
                    health_status: HealthStatus::Unknown,
                    last_health_check_at: None,
                },
                ports: vec![ServicePort {
                    id: 1,
                    container_id,
                    container_port: 80,
                    host_port,
                    protocol: Protocol::Tcp,
                    is_http: true,
                }],
                volumes: vec![],
            }],
            route_warning: None,
        }
    }

    /// 一度もstartしていないサービスに対してもservice::deletion::deleteは
    /// 無条件にstop()を呼ぶ。project_dirにbase.yml/override.ymlが
    /// 存在しない(=一度もstartしていない)場合にstop()が失敗すると、
    /// そのサービスが二度と削除できなくなってしまう
    /// (実機のWeb UIで発覚したバグ。image_runtime.rsの同種の修正と対になる)。
    #[tokio::test]
    #[ignore = "requires a running Docker daemon and the docker compose CLI plugin"]
    async fn e2e_stop_without_ever_starting_is_a_no_op() {
        let data_root = std::env::temp_dir().join(format!(
            "sahai_compose_e2e_neverstarted_{:x}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let runtime = ComposeRuntime::new(test_settings(), data_root.clone());
        let service = compose_service_detail(9101, 9102, Some(21103));

        let stop_result = runtime.stop(&service).await;
        let _ = tokio::fs::remove_dir_all(&data_root).await;

        stop_result
            .expect("一度もstartしていないcomposeサービスに対するstopはエラーにならず成功するべき");
    }

    /// compose定義が壊れているとdocker compose downは何もせず失敗する。
    /// フォールバックが無いと、書き方を誤った時点でサービスを消す手段が無くなる。
    #[tokio::test]
    #[ignore = "requires a running Docker daemon and the docker compose CLI plugin"]
    async fn e2e_stop_falls_back_to_direct_removal_when_compose_is_invalid() {
        let data_root = std::env::temp_dir().join(format!(
            "sahai_compose_e2e_invalid_{:x}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let runtime = ComposeRuntime::new(test_settings(), data_root.clone());
        let service = compose_service_detail(9200, 9201, None);

        runtime.start(&service).await.unwrap();

        // 起動後にcomposeを壊れた状態へ差し替える。network_modeとnetworksの同時指定は
        // 設定エラーとなり、downまで拒否される
        let base_path = data_root
            .join("compose-projects")
            .join("9200")
            .join("base.yml");
        tokio::fs::write(
            &base_path,
            "services:
  app:
    image: alpine:3.20
    network_mode: host
",
        )
        .await
        .unwrap();

        let stop_result = runtime.stop(&service).await;

        let remaining = tokio::process::Command::new("docker")
            .args([
                "ps",
                "-a",
                "--filter",
                "name=^svc-9201$",
                "--format",
                "{{.Names}}",
            ])
            .output()
            .await
            .unwrap();
        let _ = tokio::fs::remove_dir_all(&data_root).await;

        stop_result.expect("compose定義が壊れていてもstopは成功するべき");
        assert!(
            String::from_utf8_lossy(&remaining.stdout).trim().is_empty(),
            "フォールバックでコンテナが削除されるべき"
        );
    }

    /// 実Dockerデーモン+`docker compose`に対する結合テスト。
    /// `cargo test -- --ignored`で明示的に実行する。
    #[tokio::test]
    #[ignore = "requires a running Docker daemon and the docker compose CLI plugin"]
    async fn e2e_compose_up_creates_container_named_by_container_id_and_down_removes_it() {
        let data_root = std::env::temp_dir().join(format!(
            "sahai_compose_e2e_{:x}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let runtime = ComposeRuntime::new(test_settings(), data_root.clone());
        let service = compose_service_detail(9001, 9002, Some(21102));

        let up_result = runtime.start(&service).await;

        let verify_docker = bollard::Docker::connect_with_local_defaults().unwrap();
        let inspect_result = verify_docker
            .inspect_container(
                "svc-9002",
                None::<bollard::container::InspectContainerOptions>,
            )
            .await;

        // 後片付け: composeプロジェクトを確実に落としてから作業ディレクトリを消す
        let down_result = runtime.stop(&service).await;
        let _ = tokio::fs::remove_dir_all(&data_root).await;

        up_result.expect("docker compose up が成功するはず(nginx:alpineが取得できる前提)");
        let inspect =
            inspect_result.expect("container_nameで注入したsvc-{container_id}でinspectできるはず");
        assert_eq!(inspect.state.and_then(|s| s.running), Some(true));
        down_result.expect("docker compose down が成功するはず");
    }
}
