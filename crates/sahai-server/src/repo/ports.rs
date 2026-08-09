use sqlx::sqlite::SqliteExecutor;

use crate::domain::{Protocol, ServicePort};

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ServicePortRow {
    pub id: i64,
    pub container_id: i64,
    pub container_port: i64,
    pub host_port: i64,
    pub protocol: String,
    pub is_http: i64,
}

impl TryFrom<ServicePortRow> for ServicePort {
    type Error = String;

    fn try_from(row: ServicePortRow) -> Result<Self, Self::Error> {
        Ok(ServicePort {
            id: row.id,
            container_id: row.container_id,
            container_port: row.container_port,
            host_port: row.host_port,
            protocol: Protocol::try_from(row.protocol.as_str())?,
            is_http: row.is_http != 0,
        })
    }
}

pub struct NewPort {
    pub container_port: i64,
    pub host_port: i64,
    pub protocol: Protocol,
    pub is_http: bool,
}

pub async fn insert<'e, E: SqliteExecutor<'e>>(
    executor: E,
    container_id: i64,
    port: &NewPort,
) -> Result<i64, sqlx::Error> {
    let row: (i64,) = sqlx::query_as(
        r#"
        INSERT INTO service_ports (container_id, container_port, host_port, protocol, is_http)
        VALUES (?, ?, ?, ?, ?)
        RETURNING id
        "#,
    )
    .bind(container_id)
    .bind(port.container_port)
    .bind(port.host_port)
    .bind(port.protocol.as_str())
    .bind(port.is_http as i64)
    .fetch_one(executor)
    .await?;
    Ok(row.0)
}

pub async fn list_by_container<'e, E: SqliteExecutor<'e>>(
    executor: E,
    container_id: i64,
) -> Result<Vec<ServicePortRow>, sqlx::Error> {
    sqlx::query_as::<_, ServicePortRow>(
        "SELECT * FROM service_ports WHERE container_id = ? ORDER BY id",
    )
    .bind(container_id)
    .fetch_all(executor)
    .await
}

/// サービス配下(全コンテナ)の`is_http`ポート数。
pub async fn count_http_ports_for_service<'e, E: SqliteExecutor<'e>>(
    executor: E,
    service_id: i64,
) -> Result<i64, sqlx::Error> {
    let row: (i64,) = sqlx::query_as(
        r#"
        SELECT COUNT(*) FROM service_ports sp
        JOIN service_containers sc ON sc.id = sp.container_id
        WHERE sc.service_id = ? AND sp.is_http = 1
        "#,
    )
    .bind(service_id)
    .fetch_one(executor)
    .await?;
    Ok(row.0)
}

/// PUT時の全置き換え。
pub async fn delete_by_container<'e, E: SqliteExecutor<'e>>(
    executor: E,
    container_id: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM service_ports WHERE container_id = ?")
        .bind(container_id)
        .execute(executor)
        .await?;
    Ok(())
}

/// 指定したhost_portを既に使っているサービス名を返す。使われていなければNone。
/// `exclude_service_id`のポートは衝突とみなさない(更新時に既存の値を保存し直せるようにする)。
pub async fn find_service_using_host_port<'e, E: SqliteExecutor<'e>>(
    executor: E,
    host_port: i64,
    exclude_service_id: Option<i64>,
) -> Result<Option<String>, sqlx::Error> {
    let row: Option<(String,)> = sqlx::query_as(
        r#"
        SELECT s.name
        FROM service_ports sp
        JOIN service_containers sc ON sc.id = sp.container_id
        JOIN services s ON s.id = sc.service_id
        WHERE sp.host_port = ? AND (? IS NULL OR s.id <> ?)
        LIMIT 1
        "#,
    )
    .bind(host_port)
    .bind(exclude_service_id)
    .bind(exclude_service_id)
    .fetch_optional(executor)
    .await?;
    Ok(row.map(|r| r.0))
}
