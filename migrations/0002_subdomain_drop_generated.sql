-- ============================================================
-- 差配(Sahai): servicesテーブルの再構築(subdomainをGENERATED列から通常列へ)
--
-- 0001時点ではsubdomainを`GENERATED ALWAYS AS (name || '.example.com') STORED`列
-- として定義していたが、ベースドメインを`SAHAI_DOMAIN`環境変数で実行時に変更
-- 可能にするため、アプリケーション層
-- (sahai_core::naming::subdomain_for)が明示的に計算して書き込む通常列に変更する。
-- GENERATED列はSQL式(環境変数などの実行時値を参照不可)でのみ定義できるため、
-- 環境変数依存の値には使えないことが判明した。
--
-- SQLiteはGENERATED列を通常列に変換するALTER TABLEをサポートしないため、
-- SQLite公式ドキュメントが推奨する「テーブル再作成」手順で対応する
-- (https://www.sqlite.org/lang_altertable.html #7)。
--
-- 【重要】servicesはservice_containersからON DELETE CASCADEで参照される親テーブル
-- のため、外部キー制約が有効なままDROP TABLEすると、SQLiteの仕様上「親テーブルの
-- DROPは暗黙のDELETEとして扱われ外部キーアクションを発火させる」ため、
-- service_containers(及びCASCADEで連なるservice_ports/service_volumes)のデータが
-- 失われてしまう。この挙動は`sqlite3`コマンドラインで実際に再現・確認済み。
-- そのためこのマイグレーションは、`repo/mod.rs`の`Db::run_migrations`が
-- 外部キー制約を無効にした専用コネクションで実行する前提とする
-- (PRAGMA foreign_keysはトランザクション内ではno-opであり、sqlxは各マイグレーション
-- ファイルを単一トランザクションで実行するため、このSQLファイル内でPRAGMAを
-- 発行しても無効化できない。接続確立時にfalseを指定する必要がある)。
-- ============================================================

CREATE TABLE services_new (
    id                          INTEGER PRIMARY KEY AUTOINCREMENT,

    name                         TEXT NOT NULL UNIQUE
                                     CHECK (length(name) BETWEEN 2 AND 63),

    -- SAHAI_DOMAIN環境変数を反映してアプリケーション層(subdomain_for)が計算する
    subdomain                    TEXT NOT NULL UNIQUE,

    source_type                  TEXT NOT NULL
                                     CHECK (source_type IN ('image', 'compose')),

    image                        TEXT,

    compose_content              TEXT,

    env_vars                     TEXT NOT NULL DEFAULT '{}'
                                     CHECK (json_valid(env_vars)),

    status                       TEXT NOT NULL DEFAULT 'stopped'
                                     CHECK (status IN ('stopped', 'running', 'error')),

    health_status                TEXT NOT NULL DEFAULT 'unknown'
                                     CHECK (health_status IN ('unknown', 'healthy', 'unhealthy')),

    last_health_check_at         TEXT,

    created_at                   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at                   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),

    CHECK (
        (source_type = 'image'   AND image IS NOT NULL AND compose_content IS NULL)
        OR
        (source_type = 'compose' AND compose_content IS NOT NULL AND image IS NULL)
    )
);

INSERT INTO services_new (
    id, name, subdomain, source_type, image, compose_content, env_vars,
    status, health_status, last_health_check_at, created_at, updated_at
)
SELECT
    id, name, subdomain, source_type, image, compose_content, env_vars,
    status, health_status, last_health_check_at, created_at, updated_at
FROM services;

DROP TABLE services;
ALTER TABLE services_new RENAME TO services;

-- 旧テーブルのDROPに伴いトリガー・インデックスも失われるため再作成する(0001参照)
CREATE TRIGGER trg_services_updated_at
AFTER UPDATE ON services
FOR EACH ROW
BEGIN
    UPDATE services
    SET updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
    WHERE id = OLD.id;
END;

CREATE INDEX idx_services_status ON services(status);
CREATE INDEX idx_services_health_status ON services(health_status);
