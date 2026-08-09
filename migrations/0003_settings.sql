-- ============================================================
-- 差配(Sahai): 設定のDB化
-- これまで環境変数(SAHAI_DOMAIN等)でのみ設定可能だった値をDBに保存し、
-- Web UIの設定画面から編集・即時反映できるようにする。
-- 単一行(id=1固定)のシングルトンテーブルとして持つ。
-- 初回起動時、このテーブルが空の場合のみ既存の環境変数からシードする
-- (main.rs参照。以降はDBが正となり、環境変数は無視される)
-- ============================================================

CREATE TABLE settings (
    id                INTEGER PRIMARY KEY CHECK (id = 1),

    -- 全サービスのベースドメイン。以前はSAHAI_DOMAIN環境変数
    domain            TEXT NOT NULL,

    -- HTTP(:80)からHTTPS(:443)への恒久リダイレクトを行うか。以前はSAHAI_HTTPS_REDIRECT
    https_redirect    INTEGER NOT NULL DEFAULT 1
                          CHECK (https_redirect IN (0, 1)),

    -- コンテナレジストリURL。以前はSAHAI_REGISTRY_URL
    registry_url      TEXT NOT NULL,

    -- sahai-server APIへの固定Bearerトークン。以前はSAHAI_API_TOKEN。
    -- env_varsと同様に平文保存、DBファイルのパーミッション600をOSレベルの防御とする
    api_token         TEXT NOT NULL,

    -- DNS-01で使うDNSプロバイダ。legoが対応するプロバイダ名。
    -- 以前はSAHAI_DNS_PROVIDER環境変数
    dns_provider      TEXT NOT NULL DEFAULT 'cloudflare',

    -- Let's Encryptからの通知先メールアドレス。以前はSAHAI_ACME_EMAIL環境変数
    acme_email        TEXT NOT NULL DEFAULT '',

    updated_at        TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE TRIGGER trg_settings_updated_at
AFTER UPDATE ON settings
FOR EACH ROW
BEGIN
    UPDATE settings
    SET updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
    WHERE id = OLD.id;
END;


-- ------------------------------------------------------------
-- dns_provider_credentials: 選択中のDNSプロバイダが要求する認証情報。
-- legoの対応プロバイダごとに必要な環境変数名が異なる(例: cloudflareなら
-- CF_DNS_API_TOKEN)ため、固定カラムではなく汎用キーバリューで持つ。
-- Traefikコンテナへは、この内容から生成した.env経由でenv_fileとして渡す
-- (docker-compose.yml参照)
-- ------------------------------------------------------------
CREATE TABLE dns_provider_credentials (
    key    TEXT PRIMARY KEY,
    value  TEXT NOT NULL
);
