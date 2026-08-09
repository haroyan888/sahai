//! axumハンドラ・service層で共有するアプリケーション状態。

use std::sync::Arc;

use crate::config::Config;
use crate::docker::DockerClients;
use crate::repo::Db;
use crate::settings::SharedSettings;
use crate::traefik::RouteWriter;

pub struct AppStateInner {
    pub config: Config,
    pub settings: SharedSettings,
    pub db: Db,
    pub docker: DockerClients,
    pub traefik: RouteWriter,
    /// 設定画面でdomain/https_redirectが変更された際に管理画面静的ルートを
    /// 再生成するために保持する(settings.rs参照)。registryコンテナの
    /// docker-compose上のアドレスで、こちらはユーザー編集対象ではない。
    /// sahai-server自身のアドレスは`traefik: RouteWriter`が内部に保持している。
    pub registry_internal_url: String,
}

pub type AppState = Arc<AppStateInner>;
