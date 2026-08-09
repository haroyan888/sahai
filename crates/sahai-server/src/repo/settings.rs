//! settings/dns_provider_credentialsテーブルへのクエリのみを持つ(ビジネスロジックなし)。

use sqlx::sqlite::SqliteExecutor;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct SettingsRow {
    pub domain: String,
    pub https_redirect: bool,
    pub registry_url: String,
    pub api_token: String,
    pub dns_provider: String,
    pub acme_email: String,
    /// sahai service create専用のレジストリ資格情報。env_varsと同様に平文保存。
    pub registry_username: Option<String>,
    pub registry_password: Option<String>,
}

pub async fn load<'e>(
    executor: impl SqliteExecutor<'e>,
) -> Result<Option<SettingsRow>, sqlx::Error> {
    sqlx::query_as::<_, SettingsRow>(
        "SELECT domain, https_redirect, registry_url, api_token, dns_provider, acme_email,
                registry_username, registry_password
         FROM settings WHERE id = 1",
    )
    .fetch_optional(executor)
    .await
}

/// 初回起動時のシード用。既に行があれば何もしない(INSERT OR IGNORE)。
pub async fn seed<'e>(
    executor: impl SqliteExecutor<'e>,
    row: &SettingsRow,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT OR IGNORE INTO settings
            (id, domain, https_redirect, registry_url, api_token, dns_provider, acme_email,
             registry_username, registry_password)
         VALUES (1, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&row.domain)
    .bind(row.https_redirect)
    .bind(&row.registry_url)
    .bind(&row.api_token)
    .bind(&row.dns_provider)
    .bind(&row.acme_email)
    .bind(&row.registry_username)
    .bind(&row.registry_password)
    .execute(executor)
    .await?;
    Ok(())
}

pub async fn update<'e>(
    executor: impl SqliteExecutor<'e>,
    row: &SettingsRow,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE settings
         SET domain = ?, https_redirect = ?, registry_url = ?, api_token = ?,
             dns_provider = ?, acme_email = ?, registry_username = ?, registry_password = ?
         WHERE id = 1",
    )
    .bind(&row.domain)
    .bind(row.https_redirect)
    .bind(&row.registry_url)
    .bind(&row.api_token)
    .bind(&row.dns_provider)
    .bind(&row.acme_email)
    .bind(&row.registry_username)
    .bind(&row.registry_password)
    .execute(executor)
    .await?;
    Ok(())
}

pub async fn list_dns_provider_credentials<'e>(
    executor: impl SqliteExecutor<'e>,
) -> Result<Vec<(String, String)>, sqlx::Error> {
    let rows: Vec<(String, String)> =
        sqlx::query_as("SELECT key, value FROM dns_provider_credentials ORDER BY key")
            .fetch_all(executor)
            .await?;
    Ok(rows)
}

/// 既存の認証情報を全削除してから置き換える(単純な全量置き換え。
/// 件数が少ない設定データのためトランザクション分割の複雑さより単純さを優先)
pub async fn replace_dns_provider_credentials(
    pool: &sqlx::SqlitePool,
    credentials: &[(String, String)],
) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM dns_provider_credentials")
        .execute(&mut *tx)
        .await?;
    for (key, value) in credentials {
        sqlx::query("INSERT INTO dns_provider_credentials (key, value) VALUES (?, ?)")
            .bind(key)
            .bind(value)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await
}
