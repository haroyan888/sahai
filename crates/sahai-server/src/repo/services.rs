use sqlx::sqlite::SqliteExecutor;

use crate::domain::{HealthStatus, Service, ServiceStatus, SourceType};

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ServiceRow {
    pub id: i64,
    pub name: String,
    pub subdomain: String,
    pub source_type: String,
    pub image: Option<String>,
    pub compose_content: Option<String>,
    pub env_vars: String,
    pub status: String,
    pub health_status: String,
    pub last_health_check_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

impl TryFrom<ServiceRow> for Service {
    type Error = String;

    fn try_from(row: ServiceRow) -> Result<Self, Self::Error> {
        Ok(Service {
            id: row.id,
            name: row.name,
            subdomain: row.subdomain,
            source_type: SourceType::try_from(row.source_type.as_str())?,
            image: row.image,
            compose_content: row.compose_content,
            env_vars: serde_json::from_str(&row.env_vars)
                .map_err(|e| format!("env_varsのJSON解析に失敗: {e}"))?,
            status: ServiceStatus::try_from(row.status.as_str())?,
            health_status: HealthStatus::try_from(row.health_status.as_str())?,
            last_health_check_at: row.last_health_check_at,
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
    }
}

pub struct NewService<'a> {
    pub name: &'a str,
    /// `sahai_core::naming::subdomain_for(name, domain)`で呼び出し側が計算する
    /// (以前はSQLiteのGENERATED列で自動計算されていたが、ベースドメインを
    /// 環境変数で変更可能にするため通常列へ変更した。config.rs参照)
    pub subdomain: &'a str,
    pub source_type: SourceType,
    pub image: Option<&'a str>,
    pub compose_content: Option<&'a str>,
    pub env_vars: &'a serde_json::Value,
}

/// サービス本体をINSERTし、採番されたidを返す。
/// 呼び出し側は`ImmediateTransaction`を渡すことで排他制御下で実行する。
pub async fn insert<'e, E: SqliteExecutor<'e>>(
    executor: E,
    new: NewService<'_>,
) -> Result<i64, sqlx::Error> {
    let env_vars_json = new.env_vars.to_string();
    let row: (i64,) = sqlx::query_as(
        r#"
        INSERT INTO services (name, subdomain, source_type, image, compose_content, env_vars)
        VALUES (?, ?, ?, ?, ?, ?)
        RETURNING id
        "#,
    )
    .bind(new.name)
    .bind(new.subdomain)
    .bind(new.source_type.as_str())
    .bind(new.image)
    .bind(new.compose_content)
    .bind(env_vars_json)
    .fetch_one(executor)
    .await?;
    Ok(row.0)
}

pub async fn find_by_id<'e, E: SqliteExecutor<'e>>(
    executor: E,
    id: i64,
) -> Result<Option<ServiceRow>, sqlx::Error> {
    sqlx::query_as::<_, ServiceRow>("SELECT * FROM services WHERE id = ?")
        .bind(id)
        .fetch_optional(executor)
        .await
}

pub async fn find_by_name<'e, E: SqliteExecutor<'e>>(
    executor: E,
    name: &str,
) -> Result<Option<ServiceRow>, sqlx::Error> {
    sqlx::query_as::<_, ServiceRow>("SELECT * FROM services WHERE name = ?")
        .bind(name)
        .fetch_optional(executor)
        .await
}

/// Hostヘッダー(サブドメイン)からサービスを特定する(Not HTTP Serviceページ用)。
/// 個別ルートを持たないサービスもcatch-all経由でここに来るため、DBが唯一の手掛かりになる。
pub async fn find_by_subdomain<'e, E: SqliteExecutor<'e>>(
    executor: E,
    subdomain: &str,
) -> Result<Option<ServiceRow>, sqlx::Error> {
    sqlx::query_as::<_, ServiceRow>("SELECT * FROM services WHERE subdomain = ?")
        .bind(subdomain)
        .fetch_optional(executor)
        .await
}

/// `{id_or_name}`を解決する。
pub async fn find_by_id_or_name<'e, E: SqliteExecutor<'e>>(
    executor: E,
    id_or_name: &str,
) -> Result<Option<ServiceRow>, sqlx::Error> {
    if let Ok(id) = id_or_name.parse::<i64>() {
        find_by_id(executor, id).await
    } else {
        find_by_name(executor, id_or_name).await
    }
}

pub async fn list_all<'e, E: SqliteExecutor<'e>>(
    executor: E,
) -> Result<Vec<ServiceRow>, sqlx::Error> {
    sqlx::query_as::<_, ServiceRow>("SELECT * FROM services ORDER BY id")
        .fetch_all(executor)
        .await
}

/// サービス名を更新する。呼び出し側が`sahai_core::naming::subdomain_for`で計算した
/// 新しいsubdomainもあわせて渡す。
pub async fn update_name<'e, E: SqliteExecutor<'e>>(
    executor: E,
    id: i64,
    name: &str,
    subdomain: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE services SET name = ?, subdomain = ? WHERE id = ?")
        .bind(name)
        .bind(subdomain)
        .bind(id)
        .execute(executor)
        .await?;
    Ok(())
}

pub async fn update_env_vars<'e, E: SqliteExecutor<'e>>(
    executor: E,
    id: i64,
    env_vars: &serde_json::Value,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE services SET env_vars = ? WHERE id = ?")
        .bind(env_vars.to_string())
        .bind(id)
        .execute(executor)
        .await?;
    Ok(())
}

pub async fn update_image<'e, E: SqliteExecutor<'e>>(
    executor: E,
    id: i64,
    image: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE services SET image = ? WHERE id = ?")
        .bind(image)
        .bind(id)
        .execute(executor)
        .await?;
    Ok(())
}

pub async fn update_compose_content<'e, E: SqliteExecutor<'e>>(
    executor: E,
    id: i64,
    compose_content: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE services SET compose_content = ? WHERE id = ?")
        .bind(compose_content)
        .bind(id)
        .execute(executor)
        .await?;
    Ok(())
}

/// 起動/停止の結果を反映する。
pub async fn update_status<'e, E: SqliteExecutor<'e>>(
    executor: E,
    id: i64,
    status: ServiceStatus,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE services SET status = ? WHERE id = ?")
        .bind(status.as_str())
        .bind(id)
        .execute(executor)
        .await?;
    Ok(())
}

/// 配下のServiceContainerのワーストケース集約値を反映する。
pub async fn update_health_aggregate<'e, E: SqliteExecutor<'e>>(
    executor: E,
    id: i64,
    health_status: HealthStatus,
    last_health_check_at: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE services SET health_status = ?, last_health_check_at = ? WHERE id = ?")
        .bind(health_status.as_str())
        .bind(last_health_check_at)
        .bind(id)
        .execute(executor)
        .await?;
    Ok(())
}

/// CASCADEでservice_containers/service_ports/service_volumesも削除される。
pub async fn delete<'e, E: SqliteExecutor<'e>>(executor: E, id: i64) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM services WHERE id = ?")
        .bind(id)
        .execute(executor)
        .await?;
    Ok(())
}
