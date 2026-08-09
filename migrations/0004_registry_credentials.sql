-- ============================================================
-- 差配(Sahai): sahai register create(サーバー側build+push)専用の
-- レジストリ資格情報をDB化する。
-- 以前はSAHAI_REGISTRY_USERNAME/SAHAI_REGISTRY_PASSWORD環境変数のみで
-- 設定可能だったが、Web UIの「レジストリ設定」カードから編集・即時反映
-- できるようにする(2026-07-30)。env_varsと同様に平文保存、DBファイルの
-- パーミッション600をOSレベルの防御とする。
-- ============================================================

ALTER TABLE settings ADD COLUMN registry_username TEXT;
ALTER TABLE settings ADD COLUMN registry_password TEXT;
