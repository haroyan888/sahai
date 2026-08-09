-- ============================================================
-- 差配(Sahai): settings.dns_providerからハードコード既定値'cloudflare'を除去
--
-- 「DNSプロバイダは特定プロバイダに固定しない」という方針を
-- 明記しているが、domain列(2026-07-23に既定値を撤廃済み)とは異なり、
-- dns_provider列だけはDEFAULT 'cloudflare'が残ったままだった。settings.rsの
-- seed_from_env/unconfigured、api/setup.rsの初期セットアップからも同じ
-- ハードコード既定値を除去する対応の一環として、DBスキーマ側もdomain列と
-- 揃える(OSS化に向け、特定プロバイダの利用を前提にしないため)。
--
-- SQLiteはALTER TABLEで列のDEFAULTを直接変更できないため、0002と同じ
-- 「テーブル再作成」手順で対応する(https://www.sqlite.org/lang_altertable.html #7)。
-- settingsは他テーブルからFK参照されない単一行のシングルトンテーブルのため、
-- 0002のような外部キー無効化コネクションの考慮は不要。
-- ============================================================

CREATE TABLE settings_new (
    id                INTEGER PRIMARY KEY CHECK (id = 1),

    domain            TEXT NOT NULL,

    https_redirect    INTEGER NOT NULL DEFAULT 1
                          CHECK (https_redirect IN (0, 1)),

    registry_url      TEXT NOT NULL,

    api_token         TEXT NOT NULL,

    -- DEFAULT 'cloudflare'を除去(domain列と同じ「既定値なし」方針に統一)
    dns_provider      TEXT NOT NULL,

    acme_email        TEXT NOT NULL DEFAULT '',

    registry_username TEXT,
    registry_password TEXT,

    updated_at        TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

INSERT INTO settings_new (
    id, domain, https_redirect, registry_url, api_token, dns_provider,
    acme_email, registry_username, registry_password, updated_at
)
SELECT
    id, domain, https_redirect, registry_url, api_token, dns_provider,
    acme_email, registry_username, registry_password, updated_at
FROM settings;

DROP TABLE settings;
ALTER TABLE settings_new RENAME TO settings;

-- 旧テーブルのDROPに伴いトリガーも失われるため再作成する(0003参照)
CREATE TRIGGER trg_settings_updated_at
AFTER UPDATE ON settings
FOR EACH ROW
BEGIN
    UPDATE settings
    SET updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
    WHERE id = OLD.id;
END;
