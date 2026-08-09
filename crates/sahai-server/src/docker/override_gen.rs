//! compose型のoverride.yml生成。

use std::collections::BTreeMap;
use std::path::Path;

use serde::Serialize;

use crate::domain::ServiceDetail;

use super::DockerError;

#[derive(Debug, Serialize)]
struct OverrideFile {
    services: BTreeMap<String, OverrideService>,
}

#[derive(Debug, Default, Serialize)]
struct OverrideService {
    container_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    image: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    ports: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    volumes: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    env_file: Vec<String>,
}

/// build:を持つサービス名一覧をもとに、全コンテナ(build:の有無に関わらず)へ
/// container_name/ports/volumes/env_fileを注入し、build:を持つものだけimageも注入する。
pub fn generate_override_yaml(
    service: &ServiceDetail,
    registry_url: &str,
    build_service_names: &[String],
    sahai_data_root: &Path,
    env_file_path: &Path,
) -> Result<String, DockerError> {
    let mut services = BTreeMap::new();

    for container in &service.containers {
        let container_name = sahai_core::naming::container_docker_name(container.container.id);

        let image = if build_service_names
            .iter()
            .any(|n| n == &container.container.name)
        {
            let tag = sahai_core::naming::registry_tag_name(
                &service.service.name,
                Some(&container.container.name),
            );
            Some(format!("{registry_url}/{tag}:latest"))
        } else {
            None
        };

        let ports = container
            .ports
            .iter()
            .map(|p| {
                format!(
                    "{}:{}/{}",
                    p.host_port,
                    p.container_port,
                    p.protocol.as_str()
                )
            })
            .collect();

        let volumes = container
            .volumes
            .iter()
            .map(|v| {
                let host_path = sahai_core::naming::volume_host_path(
                    sahai_data_root,
                    service.service.id,
                    &v.container_path,
                );
                format!("{host_path}:{}", v.container_path)
            })
            .collect();

        services.insert(
            container.container.name.clone(),
            OverrideService {
                container_name,
                image,
                ports,
                volumes,
                env_file: vec![env_file_path.display().to_string()],
            },
        );
    }

    let file = OverrideFile { services };
    serde_yaml::to_string(&file).map_err(|e| DockerError::Other(e.to_string()))
}

/// 登録されたenv varsから`.env`ファイルの内容を生成する。
pub fn generate_env_file_content(env_vars: &serde_json::Value) -> String {
    match env_vars.as_object() {
        Some(map) => map
            .iter()
            .map(|(k, v)| format!("{k}={}", v.as_str().unwrap_or_default()))
            .collect::<Vec<_>>()
            .join("\n"),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::domain::{
        ContainerWithChildren, HealthStatus, Protocol, Service, ServiceContainer, ServiceDetail,
        ServicePort, ServiceStatus, ServiceVolume, SourceType,
    };

    use super::*;

    fn container(
        id: i64,
        name: &str,
        ports: Vec<ServicePort>,
        volumes: Vec<ServiceVolume>,
    ) -> ContainerWithChildren {
        ContainerWithChildren {
            container: ServiceContainer {
                id,
                service_id: 1,
                name: name.to_string(),
                health_status: HealthStatus::Unknown,
                last_health_check_at: None,
            },
            ports,
            volumes,
        }
    }

    fn compose_service_detail() -> ServiceDetail {
        ServiceDetail {
            service: Service {
                id: 1,
                name: "webstack".to_string(),
                subdomain: "webstack.example.com".to_string(),
                source_type: SourceType::Compose,
                image: None,
                compose_content: Some(
                    "services:\n  app:\n    build: .\n  db:\n    image: mysql:8\n".to_string(),
                ),
                env_vars: serde_json::json!({}),
                status: ServiceStatus::Stopped,
                last_error: None,
                health_status: HealthStatus::Unknown,
                last_health_check_at: None,
                created_at: "2026-01-01T00:00:00.000Z".to_string(),
                updated_at: "2026-01-01T00:00:00.000Z".to_string(),
            },
            containers: vec![
                container(
                    10,
                    "app",
                    vec![ServicePort {
                        id: 100,
                        container_id: 10,
                        container_port: 80,
                        host_port: 20010,
                        protocol: Protocol::Tcp,
                        is_http: true,
                    }],
                    vec![],
                ),
                container(
                    11,
                    "db",
                    vec![],
                    vec![ServiceVolume {
                        id: 200,
                        container_id: 11,
                        container_path: "/var/lib/mysql".to_string(),
                    }],
                ),
            ],
            route_warning: None,
        }
    }

    #[test]
    fn injects_image_only_for_build_services() {
        let detail = compose_service_detail();
        let yaml = generate_override_yaml(
            &detail,
            "registry.sahai.example.com",
            &["app".to_string()],
            Path::new("/var/sahai"),
            Path::new("/var/sahai/compose-projects/1/.env"),
        )
        .unwrap();

        let parsed: serde_yaml::Value = serde_yaml::from_str(&yaml).unwrap();
        let services = &parsed["services"];

        assert_eq!(
            services["app"]["image"].as_str(),
            Some("registry.sahai.example.com/webstack-app:latest")
        );
        // dbはbuild:を持たないためimageキー自体が存在しない(skip_serializing_if)
        assert!(services["db"]["image"].is_null());
    }

    #[test]
    fn injects_container_name_for_all_containers_regardless_of_build() {
        let detail = compose_service_detail();
        let yaml = generate_override_yaml(
            &detail,
            "registry.sahai.example.com",
            &["app".to_string()],
            Path::new("/var/sahai"),
            Path::new("/var/sahai/compose-projects/1/.env"),
        )
        .unwrap();

        let parsed: serde_yaml::Value = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(
            parsed["services"]["app"]["container_name"].as_str(),
            Some("svc-10")
        );
        assert_eq!(
            parsed["services"]["db"]["container_name"].as_str(),
            Some("svc-11")
        );
    }

    #[test]
    fn formats_ports_and_volumes_and_env_file() {
        let detail = compose_service_detail();
        let yaml = generate_override_yaml(
            &detail,
            "registry.sahai.example.com",
            &["app".to_string()],
            Path::new("/var/sahai"),
            Path::new("/var/sahai/compose-projects/1/.env"),
        )
        .unwrap();

        let parsed: serde_yaml::Value = serde_yaml::from_str(&yaml).unwrap();

        assert_eq!(
            parsed["services"]["app"]["ports"][0].as_str(),
            Some("20010:80/tcp")
        );
        // ボリュームパスはcontainer_idを含まずservice_id基準
        assert_eq!(
            parsed["services"]["db"]["volumes"][0].as_str(),
            Some("/var/sahai/services/1/var-lib-mysql:/var/lib/mysql")
        );
        assert_eq!(
            parsed["services"]["app"]["env_file"][0].as_str(),
            Some("/var/sahai/compose-projects/1/.env")
        );
        assert_eq!(
            parsed["services"]["db"]["env_file"][0].as_str(),
            Some("/var/sahai/compose-projects/1/.env")
        );
    }

    #[test]
    fn omits_empty_ports_and_volumes_keys() {
        let detail = compose_service_detail();
        let yaml = generate_override_yaml(
            &detail,
            "registry.sahai.example.com",
            &["app".to_string()],
            Path::new("/var/sahai"),
            Path::new("/var/sahai/compose-projects/1/.env"),
        )
        .unwrap();

        let parsed: serde_yaml::Value = serde_yaml::from_str(&yaml).unwrap();
        // dbはportsを持たないためキー自体が省略される
        assert!(parsed["services"]["db"]["ports"].is_null());
        // appはvolumesを持たないためキー自体が省略される
        assert!(parsed["services"]["app"]["volumes"].is_null());
    }

    #[test]
    fn env_file_content_formats_key_value_pairs() {
        let content = generate_env_file_content(&serde_json::json!({"FOO": "bar", "BAZ": "qux"}));
        let mut lines: Vec<&str> = content.lines().collect();
        lines.sort();
        assert_eq!(lines, vec!["BAZ=qux", "FOO=bar"]);
    }

    #[test]
    fn env_file_content_empty_for_empty_object() {
        assert_eq!(generate_env_file_content(&serde_json::json!({})), "");
    }
}
