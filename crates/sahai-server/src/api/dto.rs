//! リクエストのserde構造体。
//! レスポンスはdomain.rsの型をそのままシリアライズして返す。

use serde::Deserialize;

fn default_protocol() -> String {
    "tcp".to_string()
}

#[derive(Debug, Deserialize)]
pub struct PortInput {
    pub container_port: i64,
    /// is_httpのポートはホストに公開しないため省略できる。
    #[serde(default)]
    pub host_port: Option<i64>,
    #[serde(default = "default_protocol")]
    pub protocol: String,
    #[serde(default)]
    pub is_http: bool,
}

#[derive(Debug, Deserialize)]
pub struct VolumeInput {
    pub container_path: String,
}

#[derive(Debug, Deserialize)]
pub struct ContainerInput {
    pub name: String,
    #[serde(default)]
    pub ports: Vec<PortInput>,
    #[serde(default)]
    pub volumes: Vec<VolumeInput>,
}

#[derive(Debug, Deserialize)]
pub struct CreateServiceRequest {
    pub name: String,
    pub source_type: String,
    #[serde(default)]
    pub image: Option<String>,
    #[serde(default)]
    pub compose_content: Option<String>,
    #[serde(default)]
    pub env_vars: Option<serde_json::Value>,
    pub containers: Vec<ContainerInput>,
}

#[derive(Debug, Deserialize, Default)]
pub struct UpdateServiceRequest {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub image: Option<String>,
    #[serde(default)]
    pub compose_content: Option<String>,
    #[serde(default)]
    pub env_vars: Option<serde_json::Value>,
    #[serde(default)]
    pub containers: Option<Vec<ContainerInput>>,
}

/// `POST /api/services/upload`のmetadataパート(JSON)。`sahai service create`が送る。
/// `source_type`は含めない。展開後のディレクトリ構成から`sahai_core::compose::find_compose_file`
/// で自動判定するため(CLI側の判定ロジックと同じ関数を共有する。14章)。
#[derive(Debug, Deserialize)]
pub struct UploadServiceMetadata {
    pub name: String,
    #[serde(default)]
    pub build_args: Vec<BuildArgInput>,
    #[serde(default)]
    pub platform: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct BuildArgInput {
    pub key: String,
    pub value: String,
}

/// `POST /api/services/{id_or_name}/upload`のmetadataパート(JSON)。`sahai service update`が
/// 送る。既存サービスの`name`・`source_type`は変更しないため、`UploadServiceMetadata`と異なり
/// `name`を持たない(パスの`id_or_name`で対象サービスを特定する)。
#[derive(Debug, Deserialize)]
pub struct UpdateUploadMetadata {
    #[serde(default)]
    pub build_args: Vec<BuildArgInput>,
    #[serde(default)]
    pub platform: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub struct DeleteQuery {
    #[serde(default)]
    pub purge_volumes: bool,
}

/// 設定画面(Web UI「基本設定」)からの更新リクエスト。全項目を毎回まとめて送る
/// (部分更新はしない)。dns_provider/acme_email/registry_url/registry_username/
/// registry_passwordはここには含めない(それぞれ専用の「DNS/証明書設定」画面
/// 〈UpdateDnsConfigRequest〉・「レジストリ設定」画面〈UpdateRegistryConfigRequest〉
/// でのみ変更できる。こちらの保存は重い副作用を伴わないため、混在させると
/// 「保存したのに反映されない」という誤解を招くため分離している)。
#[derive(Debug, Deserialize)]
pub struct UpdateSettingsRequest {
    pub domain: String,
    pub https_redirect: bool,
    pub api_token: String,
}

#[derive(Debug, Deserialize)]
pub struct DnsCredentialInput {
    pub key: String,
    pub value: String,
}

/// レジストリ設定画面(Web UI「レジストリ設定」カード)からの更新リクエスト。
/// username/passwordは両方空(未設定のまま)か両方入力するかのどちらかのみ許可する
/// (片方だけはVALIDATION_ERROR。service::settings::validate_registry_config参照)。
/// 保存すると同期的にdocker loginを試みるが、失敗してもDB保存自体は成功する。
#[derive(Debug, Deserialize)]
pub struct UpdateRegistryConfigRequest {
    /// 省略可(service::settings::update_registry_configがdomainから
    /// `registry.sahai.<domain>`を自動生成する)
    #[serde(default)]
    pub registry_url: String,
    #[serde(default)]
    pub registry_username: Option<String>,
    #[serde(default)]
    pub registry_password: Option<String>,
}

/// DNS/証明書設定画面(Web UI)からの更新リクエスト。保存するとTraefikコンテナの
/// 再作成が走る。
#[derive(Debug, Deserialize)]
pub struct UpdateDnsConfigRequest {
    pub dns_provider: String,
    #[serde(default)]
    pub acme_email: String,
    #[serde(default)]
    pub credentials: Vec<DnsCredentialInput>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_settings_request_parses_basic_fields() {
        let req: UpdateSettingsRequest = serde_json::from_str(
            r#"{"domain": "example.com", "https_redirect": true, "api_token": "tok"}"#,
        )
        .unwrap();
        assert_eq!(req.domain, "example.com");
        assert_eq!(req.api_token, "tok");
    }

    #[test]
    fn update_registry_config_request_defaults_optional_fields_when_omitted() {
        let req: UpdateRegistryConfigRequest = serde_json::from_str(r#"{}"#).unwrap();
        assert_eq!(req.registry_url, "");
        assert_eq!(req.registry_username, None);
        assert_eq!(req.registry_password, None);
    }

    #[test]
    fn update_registry_config_request_parses_all_fields() {
        let req: UpdateRegistryConfigRequest = serde_json::from_str(
            r#"{"registry_url": "registry.sahai.example.com", "registry_username": "u", "registry_password": "p"}"#,
        )
        .unwrap();
        assert_eq!(req.registry_url, "registry.sahai.example.com");
        assert_eq!(req.registry_username.as_deref(), Some("u"));
        assert_eq!(req.registry_password.as_deref(), Some("p"));
    }

    #[test]
    fn port_input_defaults_protocol_to_tcp_and_is_http_to_false() {
        let port: PortInput =
            serde_json::from_str(r#"{"container_port": 80, "host_port": 20001}"#).unwrap();
        assert_eq!(port.protocol, "tcp");
        assert!(!port.is_http);
    }

    #[test]
    fn port_input_respects_explicit_values() {
        let port: PortInput = serde_json::from_str(
            r#"{"container_port": 80, "host_port": 20001, "protocol": "udp", "is_http": true}"#,
        )
        .unwrap();
        assert_eq!(port.protocol, "udp");
        assert!(port.is_http);
    }

    #[test]
    fn container_input_defaults_ports_and_volumes_to_empty() {
        let container: ContainerInput = serde_json::from_str(r#"{"name": "app"}"#).unwrap();
        assert!(container.ports.is_empty());
        assert!(container.volumes.is_empty());
    }

    #[test]
    fn create_service_request_defaults_optional_fields_to_none() {
        let req: CreateServiceRequest =
            serde_json::from_str(r#"{"name": "myapp", "source_type": "image", "containers": []}"#)
                .unwrap();
        assert!(req.image.is_none());
        assert!(req.compose_content.is_none());
        assert!(req.env_vars.is_none());
    }

    #[test]
    fn create_service_request_requires_containers_field() {
        // containersに#[serde(default)]を付けていないため省略時はエラーになるべき
        let result: Result<CreateServiceRequest, _> =
            serde_json::from_str(r#"{"name": "myapp", "source_type": "image"}"#);
        assert!(result.is_err());
    }

    #[test]
    fn update_service_request_empty_body_leaves_everything_none() {
        let req: UpdateServiceRequest = serde_json::from_str("{}").unwrap();
        assert!(req.name.is_none());
        assert!(req.image.is_none());
        assert!(req.compose_content.is_none());
        assert!(req.env_vars.is_none());
        assert!(req.containers.is_none());
    }

    #[test]
    fn update_service_request_partial_body_only_sets_given_fields() {
        let req: UpdateServiceRequest =
            serde_json::from_str(r#"{"env_vars": {"FOO": "bar"}}"#).unwrap();
        assert!(req.name.is_none());
        assert_eq!(req.env_vars, Some(serde_json::json!({"FOO": "bar"})));
    }

    #[test]
    fn update_upload_metadata_defaults_build_args_and_platform() {
        let meta: UpdateUploadMetadata = serde_json::from_str(r#"{}"#).unwrap();
        assert!(meta.build_args.is_empty());
        assert!(meta.platform.is_none());
    }

    #[test]
    fn delete_query_defaults_purge_volumes_to_false() {
        let query: DeleteQuery = serde_json::from_str("{}").unwrap();
        assert!(!query.purge_volumes);
    }

    #[test]
    fn update_dns_config_request_defaults_acme_email_and_credentials_to_empty() {
        let req: UpdateDnsConfigRequest =
            serde_json::from_str(r#"{"dns_provider": "cloudflare"}"#).unwrap();
        assert_eq!(req.acme_email, "");
        assert!(req.credentials.is_empty());
    }

    #[test]
    fn update_dns_config_request_parses_credentials_list() {
        let req: UpdateDnsConfigRequest = serde_json::from_str(
            r#"{"dns_provider": "cloudflare", "acme_email": "a@b.com", "credentials": [{"key": "CF_DNS_API_TOKEN", "value": "secret"}]}"#,
        )
        .unwrap();
        assert_eq!(req.credentials.len(), 1);
        assert_eq!(req.credentials[0].key, "CF_DNS_API_TOKEN");
        assert_eq!(req.credentials[0].value, "secret");
    }
}
