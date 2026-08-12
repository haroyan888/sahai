//! ImageRuntime: bollardでのrun/stop/pull(image型のライフサイクル)。

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use async_trait::async_trait;
use bollard::auth::DockerCredentials;
use bollard::container::{
    Config, CreateContainerOptions, RemoveContainerOptions, StartContainerOptions,
    StopContainerOptions,
};
use bollard::image::CreateImageOptions;
use bollard::models::{HostConfig, PortBinding};
use bollard::Docker;

use crate::settings::SharedSettings;
use futures_util::StreamExt;
use sahai_core::naming::SAHAI_NETWORK;

use crate::domain::ServiceDetail;

use super::{ContainerLifecycle, DockerError};

pub struct ImageRuntime {
    docker: Docker,
    /// bindマウント元の組み立て専用。dockerdがホスト側パスとして解決するため、
    /// コンテナ内から見えるパス(Config::sahai_data_root)とは別物(config.rs参照)
    host_data_root: PathBuf,
    /// pullに添えるレジストリ資格情報の取得元。Web UIから変更されるため
    /// 構築時にコピーせず、呼び出しのたびに最新値を読む
    settings: SharedSettings,
}

impl ImageRuntime {
    pub fn new(docker: Docker, host_data_root: PathBuf, settings: SharedSettings) -> Self {
        ImageRuntime {
            docker,
            settings,
            host_data_root,
        }
    }

    async fn pull(&self, image: &str) -> Result<(), DockerError> {
        let options = CreateImageOptions {
            from_image: image,
            ..Default::default()
        };
        // bollardはDocker Engine APIを直接叩くため、docker loginが書く
        // ~/.docker/config.jsonを参照しない。資格情報を添えないと匿名でのpullになり、
        // htpasswd認証を要求する差配のレジストリからは取得できない
        // (compose型はdocker compose pull=CLI経由なので設定ファイルが効く)
        let credentials = self.registry_credentials_for(image).await;
        let mut stream = self.docker.create_image(Some(options), None, credentials);
        while let Some(progress) = stream.next().await {
            progress?;
        }
        Ok(())
    }

    /// 差配のレジストリ宛のときだけ資格情報を返す。
    /// Docker Hub等の外部レジストリへ差配の資格情報を送らないため、
    /// イメージ名がレジストリのホストで始まる場合に限る。
    async fn registry_credentials_for(&self, image: &str) -> Option<DockerCredentials> {
        let settings = self.settings.read().await;
        let registry_url = settings.registry_url.trim();
        if registry_url.is_empty() || !image.starts_with(&format!("{registry_url}/")) {
            return None;
        }
        let username = settings.registry_username.clone()?;
        let password = settings.registry_password.clone()?;
        Some(DockerCredentials {
            username: Some(username),
            password: Some(password),
            serveraddress: Some(registry_url.to_string()),
            ..Default::default()
        })
    }

    /// 実行前提: `service.containers`は要素1件のみ(image型は暗黙的に1コンテナ)。
    pub fn build_container_config(
        &self,
        service: &ServiceDetail,
        host_data_root: &Path,
    ) -> Result<(String, Config<String>), DockerError> {
        let container = service.containers.first().ok_or_else(|| {
            DockerError::Other("image型サービスにコンテナが存在しません".to_string())
        })?;
        let name = sahai_core::naming::container_docker_name(container.container.id);
        let image = service
            .service
            .image
            .clone()
            .ok_or_else(|| DockerError::Other("imageが未設定です".to_string()))?;

        let mut port_bindings: HashMap<String, Option<Vec<PortBinding>>> = HashMap::new();
        let mut exposed_ports: HashMap<String, HashMap<(), ()>> = HashMap::new();
        for port in &container.ports {
            let key = format!("{}/{}", port.container_port, port.protocol.as_str());
            exposed_ports.insert(key.clone(), HashMap::new());
            // is_httpのポートはhost_portを持たない。Traefikがsahaiネットワーク越しに
            // コンテナ名で直接到達するため、ホストへ公開する必要がない
            if let Some(host_port) = port.host_port {
                port_bindings.insert(
                    key,
                    Some(vec![PortBinding {
                        host_ip: None,
                        host_port: Some(host_port.to_string()),
                    }]),
                );
            }
        }

        let binds: Vec<String> = container
            .volumes
            .iter()
            .map(|v| {
                let host_path = sahai_core::naming::volume_host_path(
                    host_data_root,
                    service.service.id,
                    &v.container_path,
                );
                format!("{host_path}:{}", v.container_path)
            })
            .collect();

        let env: Vec<String> = match service.service.env_vars.as_object() {
            Some(map) => map
                .iter()
                .map(|(k, v)| format!("{k}={}", v.as_str().unwrap_or_default()))
                .collect(),
            None => Vec::new(),
        };

        let host_config = HostConfig {
            port_bindings: Some(port_bindings),
            binds: Some(binds),
            restart_policy: Some(bollard::models::RestartPolicy {
                name: Some(bollard::models::RestartPolicyNameEnum::UNLESS_STOPPED),
                maximum_retry_count: None,
            }),
            ..Default::default()
        };

        // 土台と同じネットワークへ参加させる。Traefikはここでコンテナ名を解決する
        let mut endpoints = HashMap::new();
        endpoints.insert(
            SAHAI_NETWORK.to_string(),
            bollard::models::EndpointSettings::default(),
        );

        let config = Config {
            image: Some(image),
            env: Some(env),
            exposed_ports: Some(exposed_ports),
            host_config: Some(host_config),
            networking_config: Some(bollard::container::NetworkingConfig {
                endpoints_config: endpoints,
            }),
            ..Default::default()
        };

        Ok((name, config))
    }
}

#[async_trait]
impl ContainerLifecycle for ImageRuntime {
    async fn start(&self, service: &ServiceDetail) -> Result<(), DockerError> {
        let image = service
            .service
            .image
            .as_deref()
            .ok_or_else(|| DockerError::Other("imageが未設定です".to_string()))?;
        self.pull(image).await?;

        let (name, config) = self.build_container_config(service, &self.host_data_root)?;

        // 既存コンテナが残っていれば掃除してから作り直す(冪等性の担保)
        let _ = self
            .docker
            .remove_container(
                &name,
                Some(RemoveContainerOptions {
                    force: true,
                    ..Default::default()
                }),
            )
            .await;

        self.docker
            .create_container(
                Some(CreateContainerOptions {
                    name: name.clone(),
                    platform: None,
                }),
                config,
            )
            .await?;
        self.docker
            .start_container(&name, None::<StartContainerOptions<String>>)
            .await?;
        Ok(())
    }

    async fn stop(&self, service: &ServiceDetail) -> Result<(), DockerError> {
        let Some(container) = service.containers.first() else {
            return Ok(());
        };
        let name = sahai_core::naming::container_docker_name(container.container.id);

        // 一度もstartしていないサービスに対してもservice::deletion::deleteは
        // 無条件にstop()を呼ぶ。コンテナが元々存在しない(404)場合は
        // 「既に止まっている」と同義なので成功として扱う(実機のWeb UIで
        // 未起動サービスの削除が永久に失敗するバグとして発覚)
        if let Err(e) = self
            .docker
            .stop_container(&name, None::<StopContainerOptions>)
            .await
        {
            if !is_not_found(&e) {
                return Err(e.into());
            }
        }
        // ComposeRuntime::stop(`docker compose down`)は停止+削除を行うため、
        // ImageRuntime側も同じ意味論に揃える。揃えないとservice::deletion::deleteが
        // 「stop→DBレコード削除」だけではコンテナ実体がリークする
        // (実Dockerに対するE2Eテストで実際に発覚したバグ)
        if let Err(e) = self
            .docker
            .remove_container(
                &name,
                Some(RemoveContainerOptions {
                    force: true,
                    ..Default::default()
                }),
            )
            .await
        {
            if !is_not_found(&e) {
                return Err(e.into());
            }
        }
        Ok(())
    }
}

fn is_not_found(e: &bollard::errors::Error) -> bool {
    matches!(e, bollard::errors::Error::DockerResponseServerError { status_code, .. } if *status_code == 404)
}

#[cfg(test)]
mod tests {
    use crate::domain::{
        ContainerWithChildren, HealthStatus, Protocol, Service, ServiceContainer, ServiceDetail,
        ServicePort, ServiceStatus, ServiceVolume, SourceType,
    };

    use super::*;

    fn runtime() -> ImageRuntime {
        runtime_with_settings(settings(None))
    }

    fn runtime_with_settings_and_root(settings: SharedSettings, root: PathBuf) -> ImageRuntime {
        ImageRuntime::new(
            Docker::connect_with_local_defaults().unwrap(),
            root,
            settings,
        )
    }

    fn runtime_with_settings(settings: SharedSettings) -> ImageRuntime {
        // bollardの接続はDocker daemon不在でも遅延評価のため失敗しない(起動テストで確認済み)
        ImageRuntime::new(
            Docker::connect_with_local_defaults().unwrap(),
            PathBuf::from("/var/sahai"),
            settings,
        )
    }

    /// registry_urlは固定。資格情報の有無だけを差し替える
    fn settings(credentials: Option<(&str, &str)>) -> SharedSettings {
        let (username, password) = match credentials {
            Some((u, p)) => (Some(u.to_string()), Some(p.to_string())),
            None => (None, None),
        };
        std::sync::Arc::new(tokio::sync::RwLock::new(crate::settings::Settings {
            domain: "example.com".to_string(),
            https_redirect: true,
            registry_url: "registry.sahai.example.com".to_string(),
            api_token: "t".to_string(),
            dns_provider: String::new(),
            acme_email: String::new(),
            registry_username: username,
            registry_password: password,
        }))
    }

    /// bollardは~/.docker/config.jsonを見ないため、資格情報を添えないと匿名pullになり
    /// htpasswd認証のレジストリから取得できない。
    #[tokio::test]
    async fn adds_credentials_for_own_registry() {
        let rt = runtime_with_settings(settings(Some(("u", "p"))));
        let creds = rt
            .registry_credentials_for("registry.sahai.example.com/myapp:latest")
            .await
            .expect("自分のレジストリには資格情報を添えるべき");
        assert_eq!(creds.username.as_deref(), Some("u"));
        assert_eq!(creds.password.as_deref(), Some("p"));
        assert_eq!(
            creds.serveraddress.as_deref(),
            Some("registry.sahai.example.com")
        );
    }

    /// 外部レジストリ宛のリクエストに差配の資格情報を送らない。
    #[tokio::test]
    async fn omits_credentials_for_external_registries() {
        let rt = runtime_with_settings(settings(Some(("u", "p"))));
        for image in [
            "nginx:alpine",
            "docker.io/library/postgres:16",
            "ghcr.io/someone/app:latest",
            // 前方一致だけで判定すると通ってしまう紛らわしい名前
            "registry.sahai.example.com.evil.test/app:latest",
        ] {
            assert!(
                rt.registry_credentials_for(image).await.is_none(),
                "{image}に資格情報を添えてはいけない"
            );
        }
    }

    #[tokio::test]
    async fn omits_credentials_when_not_configured() {
        let rt = runtime_with_settings(settings(None));
        assert!(rt
            .registry_credentials_for("registry.sahai.example.com/myapp:latest")
            .await
            .is_none());
    }

    fn image_service(
        image: Option<&str>,
        ports: Vec<ServicePort>,
        volumes: Vec<ServiceVolume>,
    ) -> ServiceDetail {
        ServiceDetail {
            service: Service {
                id: 5,
                name: "myapp".to_string(),
                subdomain: "myapp.example.com".to_string(),
                source_type: SourceType::Image,
                image: image.map(str::to_string),
                compose_content: None,
                env_vars: serde_json::json!({"FOO": "bar"}),
                status: ServiceStatus::Stopped,
                last_error: None,
                health_status: HealthStatus::Unknown,
                last_health_check_at: None,
                created_at: "2026-01-01T00:00:00.000Z".to_string(),
                updated_at: "2026-01-01T00:00:00.000Z".to_string(),
            },
            containers: vec![ContainerWithChildren {
                container: ServiceContainer {
                    id: 42,
                    service_id: 5,
                    name: "myapp".to_string(),
                    health_status: HealthStatus::Unknown,
                    last_health_check_at: None,
                },
                ports,
                volumes,
            }],
            route_warning: None,
        }
    }

    #[test]
    fn container_name_uses_container_id_not_service_name() {
        let service = image_service(Some("x:latest"), vec![], vec![]);
        let (name, _) = runtime()
            .build_container_config(&service, Path::new("/var/sahai"))
            .unwrap();
        assert_eq!(name, "svc-42");
    }

    #[test]
    fn missing_image_is_an_error() {
        let service = image_service(None, vec![], vec![]);
        let result = runtime().build_container_config(&service, Path::new("/var/sahai"));
        assert!(result.is_err());
    }

    #[test]
    fn no_containers_is_an_error() {
        let mut service = image_service(Some("x:latest"), vec![], vec![]);
        service.containers.clear();
        let result = runtime().build_container_config(&service, Path::new("/var/sahai"));
        assert!(result.is_err());
    }

    #[test]
    fn port_bindings_and_exposed_ports_are_keyed_by_port_and_protocol() {
        let ports = vec![ServicePort {
            id: 1,
            container_id: 42,
            container_port: 8080,
            host_port: Some(20001),
            protocol: Protocol::Tcp,
            is_http: true,
        }];
        let service = image_service(Some("x:latest"), ports, vec![]);
        let (_, config) = runtime()
            .build_container_config(&service, Path::new("/var/sahai"))
            .unwrap();

        let exposed = config.exposed_ports.unwrap();
        assert!(exposed.contains_key("8080/tcp"));

        let bindings = config
            .host_config
            .as_ref()
            .unwrap()
            .port_bindings
            .as_ref()
            .unwrap();
        let binding = bindings.get("8080/tcp").unwrap().as_ref().unwrap();
        assert_eq!(binding[0].host_port.as_deref(), Some("20001"));
    }

    #[test]
    fn volume_binds_use_service_id_based_host_path() {
        let volumes = vec![ServiceVolume {
            id: 1,
            container_id: 42,
            container_path: "/data".to_string(),
        }];
        let service = image_service(Some("x:latest"), vec![], volumes);
        let (_, config) = runtime()
            .build_container_config(&service, Path::new("/var/sahai"))
            .unwrap();

        let binds = config.host_config.as_ref().unwrap().binds.as_ref().unwrap();
        // service_id(5)基準であり、container_id(42)は含まれない
        assert_eq!(binds, &vec!["/var/sahai/services/5/data:/data".to_string()]);
    }

    #[test]
    fn env_vars_are_formatted_as_key_equals_value() {
        let service = image_service(Some("x:latest"), vec![], vec![]);
        let (_, config) = runtime()
            .build_container_config(&service, Path::new("/var/sahai"))
            .unwrap();
        assert_eq!(config.env, Some(vec!["FOO=bar".to_string()]));
    }

    #[test]
    fn restart_policy_is_unless_stopped() {
        let service = image_service(Some("x:latest"), vec![], vec![]);
        let (_, config) = runtime()
            .build_container_config(&service, Path::new("/var/sahai"))
            .unwrap();
        let policy = config.host_config.unwrap().restart_policy.unwrap();
        assert_eq!(
            policy.name,
            Some(bollard::models::RestartPolicyNameEnum::UNLESS_STOPPED)
        );
    }

    /// 実Dockerデーモンに対する結合テスト。通常の`cargo test`では走らず、
    /// `cargo test -- --ignored`で明示的に実行する(Docker Desktop等が必要)。
    #[tokio::test]
    #[ignore = "requires a running Docker daemon"]
    async fn e2e_start_creates_a_running_container_and_stop_removes_it() {
        let rt = runtime();
        let service = image_service(
            Some("nginx:alpine"),
            vec![ServicePort {
                id: 1,
                container_id: 42,
                container_port: 80,
                host_port: Some(21100),
                protocol: Protocol::Tcp,
                is_http: true,
            }],
            vec![],
        );

        let start_result = rt.start(&service).await;

        // 検証用に別のDockerクライアントでinspectする(ImageRuntime内部のdockerフィールドは非公開)
        let verify_docker = Docker::connect_with_local_defaults().unwrap();
        let inspect_result = verify_docker
            .inspect_container(
                "svc-42",
                None::<bollard::container::InspectContainerOptions>,
            )
            .await;

        let stop_result = rt.stop(&service).await;
        // stop後にコンテナ自体が残っていないかを確認する(ComposeRuntimeの`down`は
        // 停止+削除を行うため、ImageRuntimeの`stop`も同じ意味論に揃えるべき。
        // 揃っていないとservice::deletion::deleteが「stopしてからDBレコード削除」
        // だけではDockerコンテナがリークする)
        let inspect_after_stop = verify_docker
            .inspect_container(
                "svc-42",
                None::<bollard::container::InspectContainerOptions>,
            )
            .await;

        // 万一残っていた場合に備えた最終的な後片付け(assert失敗でリークしないため)
        let _ = verify_docker
            .remove_container(
                "svc-42",
                Some(RemoveContainerOptions {
                    force: true,
                    ..Default::default()
                }),
            )
            .await;

        start_result.expect("docker pull/run が成功するはず(nginx:alpineが取得できる前提)");
        let inspect = inspect_result.expect("起動直後にinspectできるはず");
        assert_eq!(
            inspect.state.and_then(|s| s.running),
            Some(true),
            "起動後はRunning状態のはず"
        );
        stop_result.expect("stopが成功するはず");
        assert!(
            inspect_after_stop.is_err(),
            "stop後はコンテナ自体が削除されている(=inspectできない)べき。\
             ComposeRuntimeのdownと同じ意味論に揃える(deletion.rsでのリーク防止)"
        );
    }

    /// naming::volume_host_pathが生成するホストパスが、実際にこの開発機(Windows)上の
    /// Docker Desktopでbollard経由のbindマウントとして機能するかを検証する。
    /// ホスト側からマーカーファイルを書き込み、nginxのdocroot(/usr/share/nginx/html)に
    /// マウントしてHTTP経由で読めることを確認する(コンテナ内にコマンドを注入できない
    /// build_container_configの制約上、nginxの標準動作を使って観測する)。
    #[tokio::test]
    #[ignore = "requires a running Docker daemon"]
    async fn e2e_volume_bind_mount_is_actually_reachable_from_the_container() {
        let data_root = std::env::temp_dir().join(format!(
            "sahai_volume_e2e_{:x}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let service_id = 777;
        let container_id = 778;
        let host_port = 21104;

        let host_path =
            sahai_core::naming::volume_host_path(&data_root, service_id, "/usr/share/nginx/html");
        tokio::fs::create_dir_all(&host_path).await.unwrap();
        tokio::fs::write(
            std::path::Path::new(&host_path).join("index.html"),
            "sahai-volume-e2e-marker",
        )
        .await
        .unwrap();

        let service = ServiceDetail {
            service: Service {
                id: service_id,
                name: "volumetest".to_string(),
                subdomain: "volumetest.example.com".to_string(),
                source_type: SourceType::Image,
                image: Some("nginx:alpine".to_string()),
                compose_content: None,
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
                    name: "volumetest".to_string(),
                    health_status: HealthStatus::Unknown,
                    last_health_check_at: None,
                },
                ports: vec![ServicePort {
                    id: 1,
                    container_id,
                    container_port: 80,
                    // このe2eはホスト経由でHTTP到達を確かめるため、あえて公開する
                    // (is_httpのポートは通常host_portを持たない)
                    host_port: Some(host_port),
                    protocol: Protocol::Tcp,
                    is_http: true,
                }],
                volumes: vec![ServiceVolume {
                    id: 1,
                    container_id,
                    container_path: "/usr/share/nginx/html".to_string(),
                }],
            }],
            route_warning: None,
        };

        let rt = runtime_with_settings_and_root(settings(None), data_root.clone());
        let start_result = rt.start(&service).await;

        // nginxの起動を待つ
        tokio::time::sleep(std::time::Duration::from_millis(800)).await;

        let curl_output = tokio::process::Command::new("curl")
            .arg("-s")
            .arg(format!("http://127.0.0.1:{host_port}/"))
            .output()
            .await;

        let stop_result = rt.stop(&service).await;
        let _ = tokio::fs::remove_dir_all(&data_root).await;

        start_result.expect("nginxコンテナの起動が成功するはず");
        let body = String::from_utf8(curl_output.unwrap().stdout).unwrap();
        assert_eq!(
            body, "sahai-volume-e2e-marker",
            "ホスト側で書き込んだファイルの内容がコンテナ経由のHTTPレスポンスに\
             現れるはず(=naming::volume_host_pathのパスが実際にbindマウントされている)"
        );
        stop_result.expect("stopが成功するはず");
    }

    /// 実機のWeb UIから「一度もstartしていないサービスをDELETEする」操作をした際に
    /// 発覚したバグの再現テスト。service::deletion::deleteはstatusを問わず常にstop()を
    /// 呼ぶため、実体の無いコンテナに対するstop()が失敗するとサービスが二度と
    /// 削除できなくなってしまう。stop()はDocker側の「既に無い」を成功として扱うべき。
    #[tokio::test]
    #[ignore = "requires a running Docker daemon"]
    async fn e2e_stop_on_a_never_started_container_is_a_no_op() {
        let rt = runtime();
        let service = image_service(Some("nginx:alpine"), vec![], vec![]);

        // svc-42という名前のコンテナは一度も作られていない状態でstopを呼ぶ
        let stop_result = rt.stop(&service).await;

        stop_result.expect("一度も起動していないコンテナに対するstopはエラーにならず成功するべき");
    }
}
