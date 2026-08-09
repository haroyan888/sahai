//! host_portの衝突検証。登録(POST)と更新(PUT)で同じ判定を共有する。
//!
//! 範囲による制限は設けず、実際に使えないポートだけを弾く。判定は2段階に分かれる:
//! DBを見ずに決まるもの(値の妥当性・予約ポート・リクエスト内の重複)と、
//! 既存レコードを引かないと分からないもの(他サービスとの重複)。
//!
//! 後者はトランザクション内で呼ぶこと。`BEGIN IMMEDIATE`で直列化されているため、
//! 検証と挿入の間に別リクエストが同じポートを取ることはない。

use sahai_core::validation;

use crate::api::dto::ContainerInput;
use crate::error::{AppError, FieldError};
use crate::repo::ports;

fn field_path(container_index: usize, port_index: usize) -> String {
    format!("containers[{container_index}].ports[{port_index}].host_port")
}

/// DBを見ずに判定できる分を検証し、見つかったエラーをすべて返す。
/// 呼び出し側が他の検証結果と束ねてから`AppError::Validation`にできるよう、
/// エラーはVecで返して`Result`にはしない。
pub fn collect_request_errors(containers: &[ContainerInput]) -> Vec<FieldError> {
    let mut errors = Vec::new();
    // 同じポートを指定した最初の位置。2件目以降を重複として報告する
    let mut seen: Vec<(i64, String)> = Vec::new();

    for (i, c) in containers.iter().enumerate() {
        for (j, p) in c.ports.iter().enumerate() {
            let field = field_path(i, j);
            // is_httpのポートはホストに公開しないためhost_portを持たない。
            // 指定されていても無視する(公開されない値を検証しても意味がない)
            let Some(host_port) = p.host_port.filter(|_| !p.is_http) else {
                continue;
            };
            if let Err(e) = validation::validate_host_port(host_port) {
                errors.push(FieldError {
                    field,
                    message: e.to_string(),
                });
                continue;
            }
            if let Some((_, first)) = seen.iter().find(|(port, _)| *port == host_port) {
                errors.push(FieldError {
                    field,
                    message: format!(
                        "ポート{host_port}はこのリクエスト内の{first}と重複しています"
                    ),
                });
                continue;
            }
            seen.push((host_port, field));
        }
    }
    errors
}

/// 既に別のサービスが使っているhost_portが無いかを検証する。
/// `exclude_service_id`自身のポートは対象外とし、更新時に同じ値を保存し直せるようにする。
pub async fn check_against_existing(
    conn: &mut sqlx::SqliteConnection,
    containers: &[ContainerInput],
    exclude_service_id: Option<i64>,
) -> Result<(), AppError> {
    let mut errors = Vec::new();
    for (i, c) in containers.iter().enumerate() {
        for (j, p) in c.ports.iter().enumerate() {
            // is_httpのポートはホストに公開しないため衝突しようがない
            let Some(host_port) = p.host_port.filter(|_| !p.is_http) else {
                continue;
            };
            if let Some(owner) =
                ports::find_service_using_host_port(&mut *conn, host_port, exclude_service_id)
                    .await?
            {
                errors.push(FieldError {
                    field: field_path(i, j),
                    message: format!("ポート{host_port}はサービス'{owner}'が使用中です"),
                });
            }
        }
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
    use crate::api::dto::PortInput;

    fn container(name: &str, host_ports: &[i64]) -> ContainerInput {
        ContainerInput {
            name: name.to_string(),
            ports: host_ports
                .iter()
                .map(|&host_port| PortInput {
                    container_port: 8080,
                    host_port: Some(host_port),
                    protocol: "tcp".to_string(),
                    is_http: false,
                })
                .collect(),
            volumes: vec![],
        }
    }

    #[test]
    fn どの帯のポートも通る() {
        let errors = collect_request_errors(&[container("app", &[22, 8080, 50000])]);
        assert!(errors.is_empty(), "{errors:?}");
    }

    #[test]
    fn 無効な値を該当ポートのfieldで報告する() {
        let errors = collect_request_errors(&[container("app", &[8080, 0])]);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].field, "containers[0].ports[1].host_port");
    }

    #[test]
    fn 予約ポートを弾く() {
        let errors = collect_request_errors(&[container("app", &[443])]);
        assert_eq!(errors.len(), 1);
        assert!(errors[0].message.contains("差配自身"), "{errors:?}");
    }

    #[test]
    fn リクエスト内の重複を2件目の位置で報告する() {
        let errors = collect_request_errors(&[container("app", &[8080, 8080])]);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].field, "containers[0].ports[1].host_port");
        assert!(errors[0].message.contains("重複"), "{errors:?}");
    }

    #[test]
    fn コンテナをまたぐ重複も検出する() {
        let errors =
            collect_request_errors(&[container("web", &[8080]), container("api", &[8080])]);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].field, "containers[1].ports[0].host_port");
    }
}
