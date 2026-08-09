use sqlx::sqlite::SqliteExecutor;

use crate::domain::ServiceVolume;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ServiceVolumeRow {
    pub id: i64,
    pub container_id: i64,
    pub container_path: String,
}

impl From<ServiceVolumeRow> for ServiceVolume {
    fn from(row: ServiceVolumeRow) -> Self {
        ServiceVolume {
            id: row.id,
            container_id: row.container_id,
            container_path: row.container_path,
        }
    }
}

pub async fn insert<'e, E: SqliteExecutor<'e>>(
    executor: E,
    container_id: i64,
    container_path: &str,
) -> Result<i64, sqlx::Error> {
    let row: (i64,) = sqlx::query_as(
        "INSERT INTO service_volumes (container_id, container_path) VALUES (?, ?) RETURNING id",
    )
    .bind(container_id)
    .bind(container_path)
    .fetch_one(executor)
    .await?;
    Ok(row.0)
}

pub async fn list_by_container<'e, E: SqliteExecutor<'e>>(
    executor: E,
    container_id: i64,
) -> Result<Vec<ServiceVolumeRow>, sqlx::Error> {
    sqlx::query_as::<_, ServiceVolumeRow>(
        "SELECT * FROM service_volumes WHERE container_id = ? ORDER BY id",
    )
    .bind(container_id)
    .fetch_all(executor)
    .await
}

/// PUT時の全置き換え。
pub async fn delete_by_container<'e, E: SqliteExecutor<'e>>(
    executor: E,
    container_id: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM service_volumes WHERE container_id = ?")
        .bind(container_id)
        .execute(executor)
        .await?;
    Ok(())
}
