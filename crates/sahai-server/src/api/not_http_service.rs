//! Not Serviceページ用の公開情報API。
//! `is_http`ポートを持たない登録済みサービス、および未登録のサブドメインについて、
//! Web UIが未ログイン状態でも問い合わせできるJSON API。`/api/services/*`とは異なり
//! Bearerトークン認証を要求しない(authedルーターの外側に登録する。api/mod.rs参照)。
//!
//! Traefikは非HTTPサービス・未登録サブドメイン宛てのアクセスをすべてWeb UIコンテナへ
//! 転送する設計になった。Web UI(SPA)はどのHostで
//! アクセスされたかを`window.location.hostname`から取得し、このAPIに`?host=`として
//! 明示的に渡して問い合わせる(TraefikがこのAPI自身へリクエストを転送してくるわけではない)。

use axum::extract::{Query, State};
use axum::response::{IntoResponse, Json, Response};
use serde::{Deserialize, Serialize};

use crate::domain::ServicePort;
use crate::repo::{ports, services};
use crate::state::AppState;

#[derive(Deserialize)]
pub struct NotServiceQuery {
    host: String,
}

#[derive(Debug, Serialize, PartialEq)]
pub struct NotServicePortDto {
    pub host_port: i64,
    pub container_port: i64,
    pub protocol: String,
}

#[derive(Debug, Serialize, PartialEq)]
pub struct NotServiceInfoDto {
    pub found: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ports: Option<Vec<NotServicePortDto>>,
}

pub async fn handler(
    State(state): State<AppState>,
    Query(query): Query<NotServiceQuery>,
) -> Response {
    let Ok(Some(service_row)) = services::find_by_subdomain(state.db.pool(), &query.host).await
    else {
        return Json(not_found_response()).into_response();
    };

    let container_rows = crate::repo::containers::list_by_service(state.db.pool(), service_row.id)
        .await
        .unwrap_or_default();

    let mut all_ports: Vec<ServicePort> = Vec::new();
    for c in &container_rows {
        if let Ok(rows) = ports::list_by_container(state.db.pool(), c.id).await {
            for row in rows {
                if let Ok(port) = ServicePort::try_from(row) {
                    all_ports.push(port);
                }
            }
        }
    }

    Json(found_response(&service_row.name, &all_ports)).into_response()
}

fn not_found_response() -> NotServiceInfoDto {
    NotServiceInfoDto {
        found: false,
        name: None,
        ports: None,
    }
}

fn found_response(name: &str, ports: &[ServicePort]) -> NotServiceInfoDto {
    NotServiceInfoDto {
        found: true,
        name: Some(name.to_string()),
        ports: Some(
            ports
                .iter()
                .map(|p| NotServicePortDto {
                    host_port: p.host_port,
                    container_port: p.container_port,
                    protocol: p.protocol.as_str().to_string(),
                })
                .collect(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use crate::domain::Protocol;

    use super::*;

    fn port(host_port: i64, container_port: i64, protocol: Protocol) -> ServicePort {
        ServicePort {
            id: 1,
            container_id: 1,
            container_port,
            host_port,
            protocol,
            is_http: false,
        }
    }

    #[test]
    fn found_response_includes_name_and_ports() {
        let dto = found_response("mysql", &[port(20001, 3306, Protocol::Tcp)]);
        assert!(dto.found);
        assert_eq!(dto.name, Some("mysql".to_string()));
        assert_eq!(
            dto.ports,
            Some(vec![NotServicePortDto {
                host_port: 20001,
                container_port: 3306,
                protocol: "tcp".to_string(),
            }])
        );
    }

    #[test]
    fn found_response_with_no_ports_is_empty_vec_not_none() {
        let dto = found_response("mysql", &[]);
        assert_eq!(dto.ports, Some(vec![]));
    }

    #[test]
    fn not_found_response_has_found_false_and_no_details() {
        let dto = not_found_response();
        assert!(!dto.found);
        assert!(dto.name.is_none());
        assert!(dto.ports.is_none());
    }
}
