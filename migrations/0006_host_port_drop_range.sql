-- ============================================================
-- 差配(Sahai): service_portsの再構築(host_portの範囲制限を撤廃)
--
-- 0001時点ではhost_portに`CHECK (host_port BETWEEN 20000 AND 29999)`を課していたが、
-- 利用者が任意のポートを選べるようにするため、ポート番号として有効な1-65535のみに緩める。
-- 実際に使えるかどうかは範囲ではなく衝突の有無で決まるため、他サービスとの重複と
-- 差配自身が公開するポートはアプリケーション層が保存時に検証する。
-- 全サービスを通した一意性はUNIQUE制約で引き続き保証する。
--
-- SQLiteはCHECK制約を削除するALTER TABLEをサポートしないため、
-- SQLite公式ドキュメントが推奨する「テーブル再作成」手順で対応する。
--
-- 既存行はすべて20000-29999にあり新しいCHECKを満たすため、移行でデータは失われない。
--
-- service_portsはservice_containersを参照する子テーブルであり、
-- このテーブルを参照する子は存在しないため、DROPで連鎖して失われるデータはない。
-- なお`repo/mod.rs`の`Db::run_migrations`は外部キー制約を無効にした専用コネクションで
-- 実行するため、DROP時にcontainer_idの参照が問題になることもない(0002参照)。
-- ============================================================

CREATE TABLE service_ports_new (
    id                           INTEGER PRIMARY KEY AUTOINCREMENT,
    container_id                 INTEGER NOT NULL
                                     REFERENCES service_containers(id) ON DELETE CASCADE,

    container_port                INTEGER NOT NULL
                                     CHECK (container_port BETWEEN 1 AND 65535),

    -- 手動指定。範囲は設けず、衝突の検証はアプリケーション層が行う
    -- (0は「Dockerが空きポートを自動選択」の意味になってしまうため除外する)
    host_port                     INTEGER NOT NULL UNIQUE
                                     CHECK (host_port BETWEEN 1 AND 65535),

    protocol                      TEXT NOT NULL DEFAULT 'tcp'
                                     CHECK (protocol IN ('tcp', 'udp')),

    -- Traefikルーティング対象か。「1サービスにつき最大1件」はコンテナを横断する制約のため
    -- DBのUNIQUE制約だけでは表現できず、アプリケーション層で担保する。
    -- 下記のUNIQUE INDEXは「同一コンテナ内での重複」のみを防ぐ部分的な安全策
    is_http                       INTEGER NOT NULL DEFAULT 0
                                     CHECK (is_http IN (0, 1)),

    UNIQUE (container_id, container_port, protocol)
);

INSERT INTO service_ports_new (id, container_id, container_port, host_port, protocol, is_http)
SELECT id, container_id, container_port, host_port, protocol, is_http
FROM service_ports;

DROP TABLE service_ports;
ALTER TABLE service_ports_new RENAME TO service_ports;

-- 旧テーブルのDROPに伴いインデックスも失われるため再作成する(0001参照)
CREATE INDEX idx_service_ports_container_id ON service_ports(container_id);
CREATE UNIQUE INDEX idx_service_ports_one_http_per_container
    ON service_ports(container_id) WHERE is_http = 1;
