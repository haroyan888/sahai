//! 環境変数からの設定読込。
//!
//! ここに残すのはプロセス起動に必須のブートストラップ値のみ(DB接続前に確定して
//! いなければならないもの)。domain・https_redirect・registry_url・api_token・
//! DNSプロバイダ関連はDB化しWeb UIから編集可能にしており、`settings.rs`にある。
//! 起動後に変更できる値をここへ足さないこと(再起動しないと反映されなくなる)。

use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Config {
    /// axumのbindアドレス。例: 0.0.0.0:8080
    pub bind_addr: String,
    /// SQLite DBファイルパス。例: /var/sahai/db/sahai.sqlite3
    pub database_path: PathBuf,
    /// 永続化データのルート。/var/sahai 。
    /// ボリュームパス生成・Traefik動的設定の書き出し先はすべてこの配下。
    pub sahai_data_root: PathBuf,
    /// Web UI(React SPA)のビルド済み静的ファイル配信元ディレクトリ。
    /// Dockerfileが`web/dist`をこのパスへコピーし、sahai-server自身が
    /// `tower-http::ServeDir`で配信する。
    pub web_dist_dir: PathBuf,
    /// `.sahai.env`ファイルの実パス(sahai_data_root直下)。DNSプロバイダ認証情報の
    /// 書き込み先。sahai専用の内部ブリッジファイルであることを名前で示すため、
    /// 汎用的な`.env`という名前は避けている。存在しない場合は`env_file::upsert`が
    /// ディレクトリごと自動作成する。`sahai_data_root`配下に置くのは、Windows
    /// (Docker Desktop)ではコンテナ側マウント先にWindowsパスを指定できず、
    /// ホストと同一パスでのbindマウントが原理的に成立しないため、全環境で確実に
    /// 機能する絶対パスに統一する必要があるからである
    /// (`traefik::container::recreate_traefik`参照)。
    ///
    /// `sahai service create`(サーバー側build+push)用のレジストリ資格情報は
    /// `settings.rs::Settings.registry_username/registry_password`が正である。
    /// `Config`は起動に必須なブートストラップ値のみを持つ方針(冒頭コメント)のため、
    /// DB化された値はここに置かない。環境変数(`SAHAI_REGISTRY_USERNAME`/`PASSWORD`)は
    /// `Settings::seed_from_env()`側で初回シード専用として読む。
    pub env_file_path: PathBuf,
}

impl Config {
    pub fn from_env() -> Result<Self, String> {
        let sahai_data_root = PathBuf::from(
            std::env::var("SAHAI_DATA_ROOT").unwrap_or_else(|_| "/var/sahai".to_string()),
        );
        let database_path = std::env::var("SAHAI_DATABASE_PATH")
            .map(PathBuf::from)
            .unwrap_or_else(|_| sahai_data_root.join("db").join("sahai.sqlite3"));
        let web_dist_dir = std::env::var("SAHAI_WEB_DIST_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("/app/web/dist"));
        let env_file_path = sahai_data_root.join(".sahai.env");

        Ok(Config {
            bind_addr: std::env::var("SAHAI_BIND_ADDR")
                .unwrap_or_else(|_| "0.0.0.0:8080".to_string()),
            database_path,
            sahai_data_root,
            web_dist_dir,
            env_file_path,
        })
    }

    /// Traefik動的設定ファイルの書き出し先ディレクトリ。
    pub fn traefik_dynamic_dir(&self) -> PathBuf {
        self.sahai_data_root.join("traefik").join("dynamic")
    }

    /// サービスのボリュームルート。
    pub fn services_volume_root(&self) -> PathBuf {
        self.sahai_data_root.join("services")
    }
}
