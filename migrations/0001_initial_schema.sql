-- ============================================================
-- 差配(Sahai): 初期スキーマ
-- 初期スキーマ。以降の変更は必ず新しいマイグレーションを追加すること
-- (このファイルを書き換えると、適用済み環境でチェックサム不一致になり起動できなくなる)
--
-- 注意: SQLiteは外部キー制約をデフォルトで無効化している。
-- このファイル自体には影響しないが、CASCADE削除を機能させるには
-- アプリケーション側の各DB接続で `PRAGMA foreign_keys = ON;` を必ず実行すること
-- (sqlxでは SqliteConnectOptions::foreign_keys(true) で設定可能)。
-- ============================================================

-- ------------------------------------------------------------
-- services: サービス本体
-- ------------------------------------------------------------
CREATE TABLE services (
    id                          INTEGER PRIMARY KEY AUTOINCREMENT,

    -- 小文字英数字とハイフンのみ、先頭は英字、63文字以内(^[a-z][a-z0-9-]{0,61}[a-z0-9]$相当)。
    -- パターン全体の検証はアプリケーション層で行う(SQLiteのCHECKは長さのみの簡易チェック)。
    -- Dockerイメージタグ・サブドメインラベル・composeプロジェクト名の元にもなる。
    -- 登録後も変更可能(稼働中でも可)
    name                         TEXT NOT NULL UNIQUE
                                     CHECK (length(name) BETWEEN 2 AND 63),

    -- nameから自動生成する(`{name}.{domain}`)。手動指定不可。
    -- 通常の列であり、アプリケーション層(sahai_core::naming::subdomain_for)が
    -- INSERT・name変更時のUPDATEの両方で明示的に計算して書き込む。
    -- 以前はSQLiteのGENERATED列(`name || '.example.com'`)でnameとの不整合を構造的に
    -- 防いでいたが、ベースドメインを`SAHAI_DOMAIN`環境変数で
    -- コンテナ起動時に変更可能にするため、GENERATED列(環境変数を参照できない)から
    -- 通常列へ変更した(2026-07-20)。UNIQUE制約はそのまま維持し、
    -- 書き込み漏れによる不整合はアプリケーション層のテストでカバーする
    subdomain                    TEXT NOT NULL UNIQUE,

    source_type                  TEXT NOT NULL
                                     CHECK (source_type IN ('image', 'compose')),

    -- source_type='image' の場合の完全なイメージ参照(例: registry.example.com/myapp:latest)
    image                        TEXT,

    -- source_type='compose' の場合、元のdocker-composeファイル本体をそのまま保存
    -- (build:を無効化するoverrideは起動時にアプリケーション層で動的生成し、この値自体は書き換えない)。
    -- PUTで変更可能。source_type自体は登録後固定
    compose_content              TEXT,

    -- 環境変数。JSON object形式({"KEY": "VALUE", ...})。平文保存
    -- (DBファイルのパーミッション600をOSレベルの防御とする)
    env_vars                     TEXT NOT NULL DEFAULT '{}'
                                     CHECK (json_valid(env_vars)),

    -- コンテナのライフサイクル状態。起動処理(docker run/docker compose up)が
    -- 失敗した場合にerrorになる。ヘルスチェック結果では変化しない
    status                       TEXT NOT NULL DEFAULT 'stopped'
                                     CHECK (status IN ('stopped', 'running', 'error')),

    -- 配下のservice_containersのワーストケース集約値。一覧表示用
    health_status                TEXT NOT NULL DEFAULT 'unknown'
                                     CHECK (health_status IN ('unknown', 'healthy', 'unhealthy')),

    -- 配下のservice_containersの中で最新のチェック時刻。一覧表示用
    last_health_check_at         TEXT,

    created_at                   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at                   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),

    -- image/composeそれぞれで必須になるカラムの相互排他性をDBレベルで保証
    -- (table-level制約はSQLiteの仕様上、列定義の後に置く必要がある)
    CHECK (
        (source_type = 'image'   AND image IS NOT NULL AND compose_content IS NULL)
        OR
        (source_type = 'compose' AND compose_content IS NOT NULL AND image IS NULL)
    )
);

-- updated_atの自動更新
-- (SQLiteはデフォルトで再帰トリガーが無効なため、このトリガー内のUPDATEが
--  自分自身を無限に再発火させることはない)
CREATE TRIGGER trg_services_updated_at
AFTER UPDATE ON services
FOR EACH ROW
BEGIN
    UPDATE services
    SET updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
    WHERE id = OLD.id;
END;

-- Web UI一覧表示・ヘルスチェックバックグラウンドタスクでの絞り込み用
CREATE INDEX idx_services_status ON services(status);
CREATE INDEX idx_services_health_status ON services(health_status);


-- ------------------------------------------------------------
-- service_containers: サービス配下の個々のコンテナ
-- image型は暗黙的に1件(nameはservices.nameと同一に保つ)、
-- compose型はcompose_content内の各サービスに対応する
-- ------------------------------------------------------------
CREATE TABLE service_containers (
    id                           INTEGER PRIMARY KEY AUTOINCREMENT,
    service_id                   INTEGER NOT NULL
                                     REFERENCES services(id) ON DELETE CASCADE,

    -- 表示用ラベル。image型はサービス名と同一、compose型はcompose.yamlのサービス名。
    -- 実際のDockerコンテナ名(svc-{id})とは独立しており、
    -- このnameやservices.nameの変更はコンテナ実体に一切影響しない
    name                         TEXT NOT NULL,

    health_status                TEXT NOT NULL DEFAULT 'unknown'
                                     CHECK (health_status IN ('unknown', 'healthy', 'unhealthy')),
    last_health_check_at         TEXT,

    UNIQUE (service_id, name)
);

CREATE INDEX idx_service_containers_service_id ON service_containers(service_id);


-- ------------------------------------------------------------
-- service_ports: コンテナごとのポート(1コンテナに複数可、非HTTPも可)
-- ------------------------------------------------------------
CREATE TABLE service_ports (
    id                           INTEGER PRIMARY KEY AUTOINCREMENT,
    container_id                 INTEGER NOT NULL
                                     REFERENCES service_containers(id) ON DELETE CASCADE,

    container_port                INTEGER NOT NULL
                                     CHECK (container_port BETWEEN 1 AND 65535),

    -- 自動割り当てではなく手動指定。推奨レンジをDBレベルで強制する
    host_port                     INTEGER NOT NULL UNIQUE
                                     CHECK (host_port BETWEEN 20000 AND 29999),

    protocol                      TEXT NOT NULL DEFAULT 'tcp'
                                     CHECK (protocol IN ('tcp', 'udp')),

    -- Traefikルーティング対象か。「1サービスにつき最大1件」はコンテナを横断する制約のため
    -- DBのUNIQUE制約だけでは表現できず、アプリケーション層で担保する。
    -- 下記のUNIQUE INDEXは「同一コンテナ内での重複」のみを防ぐ部分的な安全策
    is_http                       INTEGER NOT NULL DEFAULT 0
                                     CHECK (is_http IN (0, 1)),

    UNIQUE (container_id, container_port, protocol)
);

CREATE INDEX idx_service_ports_container_id ON service_ports(container_id);
CREATE UNIQUE INDEX idx_service_ports_one_http_per_container
    ON service_ports(container_id) WHERE is_http = 1;


-- ------------------------------------------------------------
-- service_volumes: コンテナごとの永続化ボリューム
-- ホスト側マウント先は /var/sahai/services/<service_id>/<正規化パス>/ であり、
-- container_idには依存しない。
-- ここでのcontainer_idは、あくまで「起動時にどのコンテナへ注入するか」を表すのみ
-- ------------------------------------------------------------
CREATE TABLE service_volumes (
    id                           INTEGER PRIMARY KEY AUTOINCREMENT,
    container_id                 INTEGER NOT NULL
                                     REFERENCES service_containers(id) ON DELETE CASCADE,
    container_path                TEXT NOT NULL,

    UNIQUE (container_id, container_path)
);

CREATE INDEX idx_service_volumes_container_id ON service_volumes(container_id);
