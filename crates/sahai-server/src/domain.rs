//! ドメインモデル。DBの行(repo層)とAPIのDTO(api層)の間に立つ、アプリケーション内部の型。
//! APIのJSON表現をそのままserdeで導出できる形にしてある。

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SourceType {
    Image,
    Compose,
}

impl SourceType {
    pub fn as_str(&self) -> &'static str {
        match self {
            SourceType::Image => "image",
            SourceType::Compose => "compose",
        }
    }
}

impl TryFrom<&str> for SourceType {
    type Error = String;
    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "image" => Ok(SourceType::Image),
            "compose" => Ok(SourceType::Compose),
            other => Err(format!("不明なsource_type: {other}")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ServiceStatus {
    Stopped,
    Running,
    Error,
}

impl ServiceStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            ServiceStatus::Stopped => "stopped",
            ServiceStatus::Running => "running",
            ServiceStatus::Error => "error",
        }
    }
}

impl TryFrom<&str> for ServiceStatus {
    type Error = String;
    // NOTE: `ServiceStatus::Error`variantと`TryFrom::Error`associated typeが同名で
    // 曖昧になるため、ここだけ`Self::Error`ではなく具体型`String`で明示する
    fn try_from(value: &str) -> Result<Self, String> {
        match value {
            "stopped" => Ok(ServiceStatus::Stopped),
            "running" => Ok(ServiceStatus::Running),
            "error" => Ok(ServiceStatus::Error),
            other => Err(format!("不明なstatus: {other}")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum HealthStatus {
    Unknown,
    Healthy,
    Unhealthy,
}

impl HealthStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            HealthStatus::Unknown => "unknown",
            HealthStatus::Healthy => "healthy",
            HealthStatus::Unhealthy => "unhealthy",
        }
    }
}

impl TryFrom<&str> for HealthStatus {
    type Error = String;
    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "unknown" => Ok(HealthStatus::Unknown),
            "healthy" => Ok(HealthStatus::Healthy),
            "unhealthy" => Ok(HealthStatus::Unhealthy),
            other => Err(format!("不明なhealth_status: {other}")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Protocol {
    Tcp,
    Udp,
}

impl Protocol {
    pub fn as_str(&self) -> &'static str {
        match self {
            Protocol::Tcp => "tcp",
            Protocol::Udp => "udp",
        }
    }
}

impl TryFrom<&str> for Protocol {
    type Error = String;
    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "tcp" => Ok(Protocol::Tcp),
            "udp" => Ok(Protocol::Udp),
            other => Err(format!("不明なprotocol: {other}")),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Service {
    pub id: i64,
    pub name: String,
    pub subdomain: String,
    pub source_type: SourceType,
    pub image: Option<String>,
    pub compose_content: Option<String>,
    pub env_vars: serde_json::Value,
    pub status: ServiceStatus,
    pub health_status: HealthStatus,
    pub last_health_check_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ServiceContainer {
    pub id: i64,
    pub service_id: i64,
    pub name: String,
    pub health_status: HealthStatus,
    pub last_health_check_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ServicePort {
    pub id: i64,
    pub container_id: i64,
    pub container_port: i64,
    pub host_port: i64,
    pub protocol: Protocol,
    pub is_http: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ServiceVolume {
    pub id: i64,
    pub container_id: i64,
    pub container_path: String,
}

/// `ServiceContainer`とその配下の`ports`/`volumes`をまとめたもの。
/// `#[serde(flatten)]`によりJSON上はcontainerの各フィールドが同じ階層へ展開される。
#[derive(Debug, Clone, Serialize)]
pub struct ContainerWithChildren {
    #[serde(flatten)]
    pub container: ServiceContainer,
    pub ports: Vec<ServicePort>,
    pub volumes: Vec<ServiceVolume>,
}

/// `GET /api/services/{id_or_name}` 等が返す詳細表現。
#[derive(Debug, Clone, Serialize)]
pub struct ServiceDetail {
    #[serde(flatten)]
    pub service: Service,
    pub containers: Vec<ContainerWithChildren>,
    /// 直前の操作(現状は`start`のみ)で、Dockerコンテナ自体は正常に起動したが
    /// Traefikルートの書き出しには失敗した場合に、その旨を伝えるための一時的な
    /// メッセージ。永続化はされない(呼び出しごとに`None`から組み立てる)。
    /// 通常は`None`でJSON上のキー自体を省略する。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub route_warning: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_type_round_trips_through_as_str() {
        for variant in [SourceType::Image, SourceType::Compose] {
            assert_eq!(SourceType::try_from(variant.as_str()).unwrap(), variant);
        }
    }

    #[test]
    fn source_type_rejects_unknown_value() {
        assert!(SourceType::try_from("bogus").is_err());
    }

    #[test]
    fn service_status_round_trips_through_as_str() {
        for variant in [
            ServiceStatus::Stopped,
            ServiceStatus::Running,
            ServiceStatus::Error,
        ] {
            assert_eq!(ServiceStatus::try_from(variant.as_str()).unwrap(), variant);
        }
    }

    #[test]
    fn service_status_rejects_unknown_value() {
        assert!(ServiceStatus::try_from("bogus").is_err());
    }

    #[test]
    fn health_status_round_trips_through_as_str() {
        for variant in [
            HealthStatus::Unknown,
            HealthStatus::Healthy,
            HealthStatus::Unhealthy,
        ] {
            assert_eq!(HealthStatus::try_from(variant.as_str()).unwrap(), variant);
        }
    }

    #[test]
    fn health_status_rejects_unknown_value() {
        assert!(HealthStatus::try_from("bogus").is_err());
    }

    #[test]
    fn protocol_round_trips_through_as_str() {
        for variant in [Protocol::Tcp, Protocol::Udp] {
            assert_eq!(Protocol::try_from(variant.as_str()).unwrap(), variant);
        }
    }

    #[test]
    fn protocol_rejects_unknown_value() {
        assert!(Protocol::try_from("bogus").is_err());
    }

    // as_str()とDBのCHECK制約(migrations/0001_initial_schema.sql)の許容値が
    // 食い違うと、DBには書けるがアプリ側では読めない(またはその逆)という
    // サイレントな不整合を生む。ここで許容値そのものを固定しておく
    #[test]
    fn as_str_values_match_db_check_constraint_literals() {
        assert_eq!(SourceType::Image.as_str(), "image");
        assert_eq!(SourceType::Compose.as_str(), "compose");
        assert_eq!(ServiceStatus::Stopped.as_str(), "stopped");
        assert_eq!(ServiceStatus::Running.as_str(), "running");
        assert_eq!(ServiceStatus::Error.as_str(), "error");
        assert_eq!(HealthStatus::Unknown.as_str(), "unknown");
        assert_eq!(HealthStatus::Healthy.as_str(), "healthy");
        assert_eq!(HealthStatus::Unhealthy.as_str(), "unhealthy");
        assert_eq!(Protocol::Tcp.as_str(), "tcp");
        assert_eq!(Protocol::Udp.as_str(), "udp");
    }

    #[test]
    fn serde_serialization_matches_as_str() {
        // JSON表現(小文字)とas_str()が一致することを保証する(APIの互換性に直結する)
        assert_eq!(
            serde_json::to_string(&SourceType::Compose).unwrap(),
            "\"compose\""
        );
        assert_eq!(
            serde_json::to_string(&ServiceStatus::Error).unwrap(),
            "\"error\""
        );
        assert_eq!(
            serde_json::to_string(&HealthStatus::Unhealthy).unwrap(),
            "\"unhealthy\""
        );
        assert_eq!(serde_json::to_string(&Protocol::Udp).unwrap(), "\"udp\"");
    }

    #[test]
    fn container_with_children_flattens_container_fields_in_json() {
        let value = ContainerWithChildren {
            container: ServiceContainer {
                id: 1,
                service_id: 2,
                name: "app".to_string(),
                health_status: HealthStatus::Healthy,
                last_health_check_at: None,
            },
            ports: vec![],
            volumes: vec![],
        };
        let json = serde_json::to_value(&value).unwrap();
        // #[serde(flatten)]によりネストせず直下にnameが来ること
        assert_eq!(json["name"], "app");
        assert!(json.get("container").is_none());
    }

    fn detail_with_route_warning(route_warning: Option<&str>) -> ServiceDetail {
        ServiceDetail {
            service: Service {
                id: 1,
                name: "myapp".to_string(),
                subdomain: "myapp.example.com".to_string(),
                source_type: SourceType::Image,
                image: Some("x:latest".to_string()),
                compose_content: None,
                env_vars: serde_json::json!({}),
                status: ServiceStatus::Running,
                health_status: HealthStatus::Unknown,
                last_health_check_at: None,
                created_at: "2026-01-01T00:00:00.000Z".to_string(),
                updated_at: "2026-01-01T00:00:00.000Z".to_string(),
            },
            containers: vec![],
            route_warning: route_warning.map(str::to_string),
        }
    }

    #[test]
    fn route_warning_is_omitted_from_json_when_none() {
        let json = serde_json::to_value(detail_with_route_warning(None)).unwrap();
        assert!(
            json.get("route_warning").is_none(),
            "route_warningがNoneのときはJSONキー自体を省略すべき: {json}"
        );
    }

    #[test]
    fn route_warning_is_included_in_json_when_some() {
        let json = serde_json::to_value(detail_with_route_warning(Some("boom"))).unwrap();
        assert_eq!(json["route_warning"], "boom");
    }
}
