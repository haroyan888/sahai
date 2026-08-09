use sqlx::sqlite::SqliteExecutor;

use crate::domain::{HealthStatus, ServiceContainer};

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ServiceContainerRow {
    pub id: i64,
    pub service_id: i64,
    pub name: String,
    pub health_status: String,
    pub last_health_check_at: Option<String>,
}

impl TryFrom<ServiceContainerRow> for ServiceContainer {
    type Error = String;

    fn try_from(row: ServiceContainerRow) -> Result<Self, Self::Error> {
        Ok(ServiceContainer {
            id: row.id,
            service_id: row.service_id,
            name: row.name,
            health_status: HealthStatus::try_from(row.health_status.as_str())?,
            last_health_check_at: row.last_health_check_at,
        })
    }
}

pub async fn insert<'e, E: SqliteExecutor<'e>>(
    executor: E,
    service_id: i64,
    name: &str,
) -> Result<i64, sqlx::Error> {
    let row: (i64,) = sqlx::query_as(
        "INSERT INTO service_containers (service_id, name) VALUES (?, ?) RETURNING id",
    )
    .bind(service_id)
    .bind(name)
    .fetch_one(executor)
    .await?;
    Ok(row.0)
}

pub async fn list_by_service<'e, E: SqliteExecutor<'e>>(
    executor: E,
    service_id: i64,
) -> Result<Vec<ServiceContainerRow>, sqlx::Error> {
    sqlx::query_as::<_, ServiceContainerRow>(
        "SELECT * FROM service_containers WHERE service_id = ? ORDER BY id",
    )
    .bind(service_id)
    .fetch_all(executor)
    .await
}

/// 全running中サービスのコンテナ一覧(ヘルスチェックタスク用)。
pub async fn list_for_running_services<'e, E: SqliteExecutor<'e>>(
    executor: E,
) -> Result<Vec<ServiceContainerRow>, sqlx::Error> {
    sqlx::query_as::<_, ServiceContainerRow>(
        r#"
        SELECT sc.* FROM service_containers sc
        JOIN services s ON s.id = sc.service_id
        WHERE s.status = 'running'
        ORDER BY sc.id
        "#,
    )
    .fetch_all(executor)
    .await
}

/// compose_content編集で削除されたサービスに対応する行を削除する。
/// CASCADEでservice_ports/service_volumesも削除される。
pub async fn delete<'e, E: SqliteExecutor<'e>>(executor: E, id: i64) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM service_containers WHERE id = ?")
        .bind(id)
        .execute(executor)
        .await?;
    Ok(())
}

pub async fn update_name<'e, E: SqliteExecutor<'e>>(
    executor: E,
    id: i64,
    name: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE service_containers SET name = ? WHERE id = ?")
        .bind(name)
        .bind(id)
        .execute(executor)
        .await?;
    Ok(())
}

pub async fn update_health<'e, E: SqliteExecutor<'e>>(
    executor: E,
    id: i64,
    health_status: HealthStatus,
    last_health_check_at: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE service_containers SET health_status = ?, last_health_check_at = ? WHERE id = ?",
    )
    .bind(health_status.as_str())
    .bind(last_health_check_at)
    .bind(id)
    .execute(executor)
    .await?;
    Ok(())
}
