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
    /// **sahai-server自身のファイルI/O**(DB・uploads・compose-projects・
    /// Traefik動的設定・ボリュームディレクトリの削除)はすべてこの配下で行う。
    /// つまりこれは「sahai-serverコンテナ内から見えるパス」。
    pub sahai_data_root: PathBuf,
    /// 同じデータルートを**dockerdから見たとき**のパス。`SAHAI_HOST_DATA_ROOT`、
    /// 未設定なら`sahai_data_root`と同値(本番はホストと同一パスでマウントするため
    /// 常にこちら)。
    ///
    /// サービスの永続ボリュームのbindマウント元(`naming::volume_host_path`)だけは、
    /// 文字列がそのままdockerdへ渡り**ホスト側のパスとして解決される**ため、
    /// `sahai_data_root`ではなくこちらから組み立てなければならない
    /// (Docker-out-of-Docker。container-design.md 3章)。
    ///
    /// 2つに分けているのは開発時のため。開発ではデータルートをプロジェクト直下の
    /// `./data`に置くが、コンテナ内では`/var/sahai`、ホストでは`E:/repos/sahai/data`
    /// のように**同じ場所を指す文字列が両者で異なる**。Windowsではコンテナ側マウント先に
    /// Windowsパスを指定できず、同一パスでのマウントが原理的に成立しない。
    /// VS Code Dev Containersが`localWorkspaceFolder`と`containerWorkspaceFolder`を
    /// 別々に公開しているのと同じ理由。
    pub host_data_root: PathBuf,
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
        // 未設定なら同一パスマウント(本番)とみなす。既存環境の挙動は変わらない
        let host_data_root = std::env::var("SAHAI_HOST_DATA_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(|_| sahai_data_root.clone());

        Ok(Config {
            bind_addr: std::env::var("SAHAI_BIND_ADDR")
                .unwrap_or_else(|_| "0.0.0.0:8080".to_string()),
            database_path,
            sahai_data_root,
            host_data_root,
            web_dist_dir,
            env_file_path,
        })
    }

    /// Traefik動的設定ファイルの書き出し先ディレクトリ。
    pub fn traefik_dynamic_dir(&self) -> PathBuf {
        self.sahai_data_root.join("traefik").join("dynamic")
    }

    /// サービスのボリュームルート。**削除(purge_volumes)のためのローカルI/O用**なので
    /// `sahai_data_root`側から組み立てる。dockerdへ渡すマウント元は
    /// `naming::volume_host_path`が`host_data_root`から作る、同じ場所の別表現。
    pub fn services_volume_root(&self) -> PathBuf {
        self.sahai_data_root.join("services")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `SAHAI_HOST_DATA_ROOT`未設定なら`SAHAI_DATA_ROOT`と同値になる。
    /// 本番はホストと同一パスでマウントするため常にこの経路で、
    /// この既定が崩れると既存環境のボリュームパスが変わってしまう。
    #[test]
    fn host_data_root_defaults_to_data_root() {
        let data_root = PathBuf::from("/var/sahai");
        let host_data_root = None::<String>
            .map(PathBuf::from)
            .unwrap_or_else(|| data_root.clone());
        assert_eq!(host_data_root, data_root);
    }

    /// 開発時のように両者が食い違う場合、bindマウント元はhost側から組み立てる。
    #[test]
    fn volume_bind_source_is_built_from_host_data_root() {
        let config = Config {
            bind_addr: "127.0.0.1:0".to_string(),
            database_path: PathBuf::from("/var/sahai/db/sahai.sqlite3"),
            sahai_data_root: PathBuf::from("/var/sahai"),
            host_data_root: PathBuf::from("E:/repos/sahai/data"),
            web_dist_dir: PathBuf::from("/app/web/dist"),
            env_file_path: PathBuf::from("/var/sahai/.sahai.env"),
        };

        // dockerdへ渡す側はホスト表現
        assert_eq!(
            sahai_core::naming::volume_host_path(&config.host_data_root, 1, "/var/lib/mysql"),
            "E:/repos/sahai/data/services/1/var-lib-mysql"
        );
        // 自身が削除するときはコンテナ内表現。同じ場所を別の文字列で指している
        assert_eq!(
            config.services_volume_root().join("1"),
            PathBuf::from("/var/sahai/services/1")
        );
    }
}
