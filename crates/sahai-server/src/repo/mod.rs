//! DBアクセス層。sqlxクエリのみを持ち、ビジネスロジックは持たない
//! (判断はservice層に置く。この層を薄く保つことでservice層をDB込みでテストしやすくする)。

pub mod containers;
pub mod ports;
pub mod services;
pub mod settings;
pub mod volumes;

use std::str::FromStr;

use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{Sqlite, SqlitePool};

use crate::config::Config;

#[derive(Clone)]
pub struct Db {
    pool: SqlitePool,
}

impl Db {
    pub async fn connect(config: &Config) -> Result<Self, sqlx::Error> {
        Self::run_migrations(config).await?;
        let opts = Self::connect_options(config)?;
        let pool = SqlitePoolOptions::new().connect_with(opts).await?;
        crate::fs_perms::secure_file(&config.database_path)
            .await
            .map_err(sqlx::Error::Io)?;
        Ok(Db { pool })
    }

    /// テスト専用: コネクションプールを1本に固定する。`ImmediateTransaction`が
    /// commit/rollbackされずに返却された場合の不整合を、確実に同一コネクションの
    /// 再利用で検出できるようにするため(service::registration::testsのRED参照)。
    #[cfg(test)]
    pub async fn connect_for_test(config: &Config) -> Result<Self, sqlx::Error> {
        Self::run_migrations(config).await?;
        let opts = Self::connect_options(config)?;
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            // コネクションが返却されないバグがあった場合、テストが無限に
            // ハングするのではなく短時間で明確に失敗するようにする
            .acquire_timeout(std::time::Duration::from_secs(3))
            .connect_with(opts)
            .await?;
        Ok(Db { pool })
    }

    /// マイグレーションを外部キー制約を無効にした専用コネクションで実行する。
    /// 0002マイグレーション(servicesテーブルの再構築)はDROP TABLEを含み、
    /// servicesはservice_containersからON DELETE CASCADEで参照される親テーブルの
    /// ため、外部キー制約が有効なままだと「親テーブルのDROPは暗黙のDELETEとして
    /// 扱われ外部キーアクションを発火させる」というSQLiteの仕様により子テーブルの
    /// データが失われてしまう(実際に実機デプロイ環境で踏んだ事故。
    /// repo::tests::applying_0002_...参照)。マイグレーション完了後の通常運用では
    /// 引き続き外部キー制約を有効にする(CASCADE削除が要件のため)。
    async fn run_migrations(config: &Config) -> Result<(), sqlx::Error> {
        let opts = Self::connect_options(config)?.foreign_keys(false);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await?;
        sqlx::migrate!("../../migrations")
            .run(&pool)
            .await
            .map_err(|e| sqlx::Error::Migrate(Box::new(e)))?;
        pool.close().await;
        Ok(())
    }

    fn connect_options(config: &Config) -> Result<SqliteConnectOptions, sqlx::Error> {
        if let Some(parent) = config.database_path.parent() {
            std::fs::create_dir_all(parent).map_err(sqlx::Error::Io)?;
        }
        Ok(
            SqliteConnectOptions::from_str(&format!(
                "sqlite://{}",
                config.database_path.display()
            ))?
            .create_if_missing(true)
            // CASCADE削除を機能させるために必須(migrations/0001_initial_schema.sql冒頭の注記参照)
            .foreign_keys(true),
        )
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }
}

/// `BEGIN IMMEDIATE`による排他制御トランザクション。サービスの登録・更新・削除を
/// 直列化し、同時リクエストによるhost_portの重複などの競合を防ぐ。
/// sqlxの`Pool::begin()`は常に素の`BEGIN`を発行し`BEGIN IMMEDIATE`を指定できないため、
/// コネクションを直接借りてSQLで制御する。
pub struct ImmediateTransaction {
    conn: sqlx::pool::PoolConnection<Sqlite>,
}

impl ImmediateTransaction {
    pub async fn begin(db: &Db) -> Result<Self, sqlx::Error> {
        let mut conn = db.pool.acquire().await?;
        sqlx::query("BEGIN IMMEDIATE").execute(&mut *conn).await?;
        Ok(ImmediateTransaction { conn })
    }

    pub fn conn(&mut self) -> &mut sqlx::SqliteConnection {
        &mut self.conn
    }

    pub async fn commit(mut self) -> Result<(), sqlx::Error> {
        sqlx::query("COMMIT").execute(&mut *self.conn).await?;
        Ok(())
    }

    pub async fn rollback(mut self) -> Result<(), sqlx::Error> {
        sqlx::query("ROLLBACK").execute(&mut *self.conn).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::domain::{Protocol, ServiceStatus, SourceType};

    async fn test_db() -> Db {
        let dir = std::env::temp_dir().join(format!("sahai_test_{}", uuid_like()));
        std::fs::create_dir_all(&dir).unwrap();
        let config = Config {
            bind_addr: "127.0.0.1:0".to_string(),
            database_path: dir.join("test.sqlite3"),
            sahai_data_root: dir,
            web_dist_dir: PathBuf::from("/unused"),
            env_file_path: PathBuf::from("/unused/.env"),
        };
        // Db::connect内でマイグレーションが実行される
        Db::connect(&config).await.unwrap()
    }

    /// 0001時点でGENERATED列だったsubdomainを持つ実デプロイ環境を模擬し、
    /// 0002マイグレーション(servicesテーブルの再構築)適用後もservice_containers等の
    /// 子テーブルのデータが失われないことを確認する回帰テスト。
    /// 実機で`SAHAI_DOMAIN`環境変数対応のため0001を直接書き換えた際に、外部キー制約が
    /// 有効なまま親テーブル(services)をDROPすると、SQLiteの仕様上「親テーブルのDROPは
    /// 暗黙のDELETEとして扱われ外部キーアクションを発火させる」ため
    /// service_containersのデータが失われる、という実際の事故を防ぐためのテスト
    /// (`Db::connect`が外部キー制約を無効にした専用コネクションでマイグレーションを
    /// 実行するようになった経緯。repo/mod.rsの`Db::run_migrations`参照)。
    #[tokio::test]
    async fn applying_0002_to_a_database_with_existing_data_preserves_child_rows() {
        let dir = std::env::temp_dir().join(format!("sahai_test_migrate_{}", uuid_like()));
        std::fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("test.sqlite3");

        // ステップ1: 0001(GENERATED列版)のみが適用された既存デプロイを模擬する
        {
            let opts = sqlx::sqlite::SqliteConnectOptions::from_str(&format!(
                "sqlite://{}",
                db_path.display()
            ))
            .unwrap()
            .create_if_missing(true)
            .foreign_keys(true);
            let pool = SqlitePoolOptions::new().connect_with(opts).await.unwrap();

            sqlx::query(
                r#"
                CREATE TABLE services (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    name TEXT NOT NULL UNIQUE,
                    subdomain TEXT NOT NULL UNIQUE GENERATED ALWAYS AS (name || '.example.com') STORED,
                    source_type TEXT NOT NULL,
                    image TEXT,
                    compose_content TEXT,
                    env_vars TEXT NOT NULL DEFAULT '{}',
                    status TEXT NOT NULL DEFAULT 'stopped',
                    health_status TEXT NOT NULL DEFAULT 'unknown',
                    last_health_check_at TEXT,
                    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
                    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
                );
                CREATE TABLE service_containers (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    service_id INTEGER NOT NULL REFERENCES services(id) ON DELETE CASCADE,
                    name TEXT NOT NULL,
                    health_status TEXT NOT NULL DEFAULT 'unknown',
                    last_health_check_at TEXT,
                    UNIQUE (service_id, name)
                );
                CREATE TABLE service_ports (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    container_id INTEGER NOT NULL REFERENCES service_containers(id) ON DELETE CASCADE,
                    container_port INTEGER NOT NULL,
                    host_port INTEGER NOT NULL UNIQUE,
                    protocol TEXT NOT NULL DEFAULT 'tcp',
                    is_http INTEGER NOT NULL DEFAULT 0
                );
                CREATE TABLE service_volumes (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    container_id INTEGER NOT NULL REFERENCES service_containers(id) ON DELETE CASCADE,
                    container_path TEXT NOT NULL
                );
                "#,
            )
            .execute(&pool)
            .await
            .unwrap();

            let (service_id,): (i64,) =
                sqlx::query_as("INSERT INTO services (name, source_type, image) VALUES ('myapp', 'image', 'x:latest') RETURNING id")
                    .fetch_one(&pool)
                    .await
                    .unwrap();
            sqlx::query("INSERT INTO service_containers (service_id, name) VALUES (?, 'myapp')")
                .bind(service_id)
                .execute(&pool)
                .await
                .unwrap();

            // sqlxのマイグレーション追跡テーブルにも0001が適用済みとして記録しておく
            // (バージョン不一致チェックを避けるため、現在の0001ファイルのchecksumで記録する)
            sqlx::query(
                "CREATE TABLE _sqlx_migrations (
                    version BIGINT PRIMARY KEY,
                    description TEXT NOT NULL,
                    installed_on TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
                    success BOOLEAN NOT NULL,
                    checksum BLOB NOT NULL,
                    execution_time BIGINT NOT NULL
                )",
            )
            .execute(&pool)
            .await
            .unwrap();
            let migration_0001_sql = include_str!("../../../../migrations/0001_initial_schema.sql");
            use sha2::{Digest, Sha384};
            let checksum = Sha384::digest(migration_0001_sql.as_bytes()).to_vec();
            sqlx::query(
                "INSERT INTO _sqlx_migrations (version, description, success, checksum, execution_time) VALUES (1, 'initial schema', 1, ?, 0)",
            )
            .bind(checksum)
            .execute(&pool)
            .await
            .unwrap();

            pool.close().await;
        }

        // ステップ2: 通常の起動フロー(Db::connect)で0002を含む残りのマイグレーションを適用
        let config = Config {
            bind_addr: "127.0.0.1:0".to_string(),
            database_path: db_path,
            sahai_data_root: dir,
            web_dist_dir: PathBuf::from("/unused"),
            env_file_path: PathBuf::from("/unused/.env"),
        };
        // Db::connect内でマイグレーション(0001+0002)が実行される
        let db = Db::connect(&config).await.unwrap();

        // 0002が実際に適用され、subdomainがGENERATED列でなくなっていること
        // (GENERATED列のままなら以下のINSERTはコンパイルは通っても実行時エラーになる)
        sqlx::query("INSERT INTO services (name, subdomain, source_type, image) VALUES ('other', 'other.example.test', 'image', 'y:latest')")
            .execute(db.pool())
            .await
            .expect("0002適用後はsubdomainへの明示的な値指定が成功するはず(GENERATED列なら失敗する)");

        // 子テーブル(service_containers)のデータが失われていないこと
        let service = services::find_by_name(db.pool(), "myapp")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(service.subdomain, "myapp.example.com");
        let containers = containers::list_by_service(db.pool(), service.id)
            .await
            .unwrap();
        assert_eq!(
            containers.len(),
            1,
            "0002マイグレーションでservice_containersのデータが失われてはいけない"
        );
        assert_eq!(containers[0].name, "myapp");
    }

    fn uuid_like() -> String {
        format!(
            "{:x}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        )
    }

    /// env_varsは平文保存のため、DBファイルの
    /// パーミッション600をOSレベルの防御とする。この開発機(Windows)ではunixの
    /// パーミッションモデルが存在しないため実行されず、実Linux環境でのみ検証される。
    #[cfg(unix)]
    #[tokio::test]
    async fn database_file_has_owner_only_permissions_after_connect() {
        use std::os::unix::fs::PermissionsExt;

        let dir = std::env::temp_dir().join(format!("sahai_test_perms_{}", uuid_like()));
        std::fs::create_dir_all(&dir).unwrap();
        let config = Config {
            bind_addr: "127.0.0.1:0".to_string(),
            database_path: dir.join("test.sqlite3"),
            sahai_data_root: dir.clone(),
            web_dist_dir: PathBuf::from("/unused"),
            env_file_path: PathBuf::from("/unused/.env"),
        };

        let _db = Db::connect(&config).await.unwrap();

        let metadata = std::fs::metadata(&config.database_path).unwrap();
        let mode = metadata.permissions().mode() & 0o777;
        assert_eq!(
            mode, 0o600,
            "DBファイルは所有者のみ読み書き可能(600)であるべき"
        );
    }

    #[tokio::test]
    async fn insert_find_update_delete_roundtrip() {
        let db = test_db().await;

        // 登録: サービス + コンテナ + ポート + ボリューム
        let mut tx = ImmediateTransaction::begin(&db).await.unwrap();
        let service_id = services::insert(
            tx.conn(),
            services::NewService {
                name: "myapp",
                subdomain: "myapp.example.com",
                source_type: SourceType::Image,
                image: Some("registry.example.test/myapp:latest"),
                compose_content: None,
                env_vars: &serde_json::json!({"FOO": "bar"}),
            },
        )
        .await
        .unwrap();
        let container_id = containers::insert(tx.conn(), service_id, "myapp")
            .await
            .unwrap();
        ports::insert(
            tx.conn(),
            container_id,
            &ports::NewPort {
                container_port: 8080,
                host_port: 20001,
                protocol: Protocol::Tcp,
                is_http: true,
            },
        )
        .await
        .unwrap();
        volumes::insert(tx.conn(), container_id, "/data")
            .await
            .unwrap();
        tx.commit().await.unwrap();

        // subdomainがアプリケーション層の計算通り保存されていること
        let row = services::find_by_name(db.pool(), "myapp")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.subdomain, "myapp.example.com");
        assert_eq!(row.status, "stopped");

        // {id_or_name}解決(数値IDでも一意なサービス名でも引ける)
        let by_id = services::find_by_id_or_name(db.pool(), &service_id.to_string())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(by_id.id, service_id);

        // name変更時、呼び出し側が計算したsubdomainも一緒に更新されること
        // (GENERATED列廃止によりrepo層は自動追従しないため、呼び出し側の責務になった)
        services::update_name(db.pool(), service_id, "renamed", "renamed.example.com")
            .await
            .unwrap();
        let renamed = services::find_by_id(db.pool(), service_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(renamed.subdomain, "renamed.example.com");

        // is_httpはサービスにつき最大1件。DB制約では表現できないためアプリ層で担保する
        let http_count = ports::count_http_ports_for_service(db.pool(), service_id)
            .await
            .unwrap();
        assert_eq!(http_count, 1);

        // ステータス遷移
        services::update_status(db.pool(), service_id, ServiceStatus::Running)
            .await
            .unwrap();
        let running = services::find_by_id(db.pool(), service_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(running.status, "running");

        // CASCADE削除
        services::delete(db.pool(), service_id).await.unwrap();
        assert!(services::find_by_id(db.pool(), service_id)
            .await
            .unwrap()
            .is_none());
        let remaining_ports = ports::list_by_container(db.pool(), container_id)
            .await
            .unwrap();
        assert!(remaining_ports.is_empty());
    }

    #[tokio::test]
    async fn host_port_unique_across_services() {
        let db = test_db().await;
        let mut tx = ImmediateTransaction::begin(&db).await.unwrap();
        let s1 = services::insert(
            tx.conn(),
            services::NewService {
                name: "svc1",
                subdomain: "svc1.example.com",
                source_type: SourceType::Image,
                image: Some("x:latest"),
                compose_content: None,
                env_vars: &serde_json::json!({}),
            },
        )
        .await
        .unwrap();
        let c1 = containers::insert(tx.conn(), s1, "svc1").await.unwrap();
        ports::insert(
            tx.conn(),
            c1,
            &ports::NewPort {
                container_port: 80,
                host_port: 20005,
                protocol: Protocol::Tcp,
                is_http: true,
            },
        )
        .await
        .unwrap();

        let s2 = services::insert(
            tx.conn(),
            services::NewService {
                name: "svc2",
                subdomain: "svc2.example.com",
                source_type: SourceType::Image,
                image: Some("y:latest"),
                compose_content: None,
                env_vars: &serde_json::json!({}),
            },
        )
        .await
        .unwrap();
        let c2 = containers::insert(tx.conn(), s2, "svc2").await.unwrap();

        // host_portは全サービスを通して一意。別サービスでもUNIQUE制約違反になる
        let result = ports::insert(
            tx.conn(),
            c2,
            &ports::NewPort {
                container_port: 81,
                host_port: 20005,
                protocol: Protocol::Tcp,
                is_http: false,
            },
        )
        .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn find_service_using_host_port_reports_the_owner() {
        let db = test_db().await;
        let mut tx = ImmediateTransaction::begin(&db).await.unwrap();
        let s1 = services::insert(
            tx.conn(),
            services::NewService {
                name: "owner",
                subdomain: "owner.example.com",
                source_type: SourceType::Image,
                image: Some("x:latest"),
                compose_content: None,
                env_vars: &serde_json::json!({}),
            },
        )
        .await
        .unwrap();
        let c1 = containers::insert(tx.conn(), s1, "owner").await.unwrap();
        // DB側にも範囲のCHECK制約が無いことを兼ねて確かめる
        ports::insert(
            tx.conn(),
            c1,
            &ports::NewPort {
                container_port: 80,
                host_port: 8080,
                protocol: Protocol::Tcp,
                is_http: true,
            },
        )
        .await
        .unwrap();

        let found = ports::find_service_using_host_port(tx.conn(), 8080, None)
            .await
            .unwrap();
        assert_eq!(found.as_deref(), Some("owner"));

        // 未使用のポートは誰も使っていない
        let free = ports::find_service_using_host_port(tx.conn(), 8081, None)
            .await
            .unwrap();
        assert_eq!(free, None);

        // 自分自身を除外すると衝突とみなさない(更新時に同じ値を保存し直すケース)
        let excluded = ports::find_service_using_host_port(tx.conn(), 8080, Some(s1))
            .await
            .unwrap();
        assert_eq!(excluded, None);
    }
}
