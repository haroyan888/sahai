-- ============================================================
-- 差配(Sahai): service_portsの再構築(host_portをNULL可にする)
--
-- is_httpのポートはホストに公開しなくなった。サービスのコンテナがsahaiネットワークに
-- 参加し、Traefikがコンテナ名で直接到達するため、ホスト側ポートを持つ必要がない。
-- 公開すると`https://<サービス名>.<ドメイン>`とは別に平文の到達経路ができてしまう
-- という問題への対応でもある(要件定義書6章)。
--
-- host_portのNOT NULLを外し、既存のis_httpポートはNULLへ移行する。
-- 利用者の操作は不要で、次回のstart/restartから新しい経路に切り替わる。
--
-- SQLiteはNOT NULLを外すALTER TABLEをサポートしないため、
-- SQLite公式ドキュメントが推奨する「テーブル再作成」手順で対応する(0006と同じ)。
--
-- UNIQUE制約は維持する。SQLiteのUNIQUEは複数のNULLを許容するため、
-- is_httpポートが何件NULLになっても衝突しない。
-- ============================================================

CREATE TABLE service_ports_new (
    id                           INTEGER PRIMARY KEY AUTOINCREMENT,
    container_id                 INTEGER NOT NULL
                                     REFERENCES service_containers(id) ON DELETE CASCADE,

    container_port                INTEGER NOT NULL
                                     CHECK (container_port BETWEEN 1 AND 65535),

    -- is_httpのポートはホストに公開しないためNULL。それ以外は手動指定で、
    -- 範囲は設けず衝突の検証はアプリケーション層が行う
    host_port                     INTEGER UNIQUE
                                     CHECK (host_port IS NULL OR host_port BETWEEN 1 AND 65535),

    protocol                      TEXT NOT NULL DEFAULT 'tcp'
                                     CHECK (protocol IN ('tcp', 'udp')),

    -- Traefikルーティング対象か。「1サービスにつき最大1件」はコンテナを横断する制約のため
    -- DBのUNIQUE制約だけでは表現できず、アプリケーション層で担保する。
    -- 下記のUNIQUE INDEXは「同一コンテナ内での重複」のみを防ぐ部分的な安全策
    is_http                       INTEGER NOT NULL DEFAULT 0
                                     CHECK (is_http IN (0, 1)),

    UNIQUE (container_id, container_port, protocol)
);

-- is_httpのポートはhost_portを捨てる。残すとUNIQUE制約が不要にその番号を
-- 押さえ続け、他のサービスが非HTTPポートとして使えなくなる
INSERT INTO service_ports_new (id, container_id, container_port, host_port, protocol, is_http)
SELECT id, container_id, container_port,
       CASE WHEN is_http = 1 THEN NULL ELSE host_port END,
       protocol, is_http
FROM service_ports;

DROP TABLE service_ports;
ALTER TABLE service_ports_new RENAME TO service_ports;

-- 旧テーブルのDROPに伴いインデックスも失われるため再作成する(0001参照)
CREATE INDEX idx_service_ports_container_id ON service_ports(container_id);
CREATE UNIQUE INDEX idx_service_ports_one_http_per_container
    ON service_ports(container_id) WHERE is_http = 1;
