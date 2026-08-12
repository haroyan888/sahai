//! Traefikルート生成層。is_httpの有無で実サービス/Not HTTP Serviceページに分岐する
//! ルートは登録時ではなく start/restart 時に毎回、その時点のDB状態から冪等に生成する。
//! これによりポート編集や compose_content 編集による is_http 対象の変更が確実に反映される
//! (名前変更時だけは restart を待たず直ちに書き換える。update.rs参照)。

use std::path::PathBuf;

use serde::Serialize;

use crate::domain::ServiceDetail;
use crate::settings::SharedSettings;

#[derive(Debug, thiserror::Error)]
pub enum TraefikError {
    #[error("Traefikルートの書き込みに失敗しました: {0}")]
    Io(#[from] std::io::Error),
    #[error("Traefikルートの生成に失敗しました: {0}")]
    Serialize(#[from] serde_yaml::Error),
}

#[derive(Debug, Serialize)]
struct DynamicConfig {
    http: HttpConfig,
}

#[derive(Debug, Serialize)]
struct HttpConfig {
    routers: std::collections::BTreeMap<String, Router>,
    services: std::collections::BTreeMap<String, TraefikService>,
}

#[derive(Debug, Serialize)]
struct Router {
    rule: String,
    service: String,
    #[serde(rename = "entryPoints", skip_serializing_if = "Option::is_none")]
    entry_points: Option<Vec<String>>,
    /// `tls`キーの有無がプロトコル可用性を直接左右する(実機検証で判明):
    /// `tls`を指定する(空でも)とそのルーターはwebsecure(:443)専用になりweb(:80)
    /// では一切応答せず、逆に`tls`を省略するとweb(:80)専用になりwebsecure(:443)
    /// では一切応答しない(entryPointsフィールドの指定に関わらずこの挙動になる)。
    /// 単一ルーターで両プロトコルに応答させることはできないため、
    /// https_redirect=falseのときはweb用とwebsecure用の2本に分ける(tls_twin参照)。
    #[serde(skip_serializing_if = "Option::is_none")]
    tls: Option<Tls>,
}

#[derive(Debug, Serialize)]
struct Tls {
    /// https_redirect=falseのwebsecure用ルーターでは省略する。ローカルドメインで
    /// 無駄なACME証明書取得を試みないためで、Traefik既定の自己署名証明書で応答する。
    #[serde(rename = "certResolver", skip_serializing_if = "Option::is_none")]
    cert_resolver: Option<String>,
}

/// 管理画面の静的ルート専用。per-serviceのRouter/Tlsとは異なり`priority`と
/// ワイルドカード証明書用の`tls.domains`を持つ(通常のper-serviceルートは
/// `Host()`から対象ドメインを自動導出できるため不要)。
#[derive(Debug, Serialize)]
struct StaticRouter {
    rule: String,
    service: String,
    priority: u32,
    #[serde(rename = "entryPoints", skip_serializing_if = "Option::is_none")]
    entry_points: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    middlewares: Option<Vec<String>>,
    /// `tls`キーの有無がプロトコル可用性を左右する(Router.tls参照)。
    /// https_redirect=falseのとき、およびHTTP専用のリダイレクト用ルーター
    /// (entryPoints=["web"])では省略する。
    #[serde(skip_serializing_if = "Option::is_none")]
    tls: Option<StaticTls>,
}

#[derive(Debug, Serialize)]
struct StaticTls {
    /// registryルーターのみhttps_redirect=falseでも`certResolver`なしでtls自体は
    /// 維持するため、Optionにしている(StaticRouter.tls参照)。
    #[serde(rename = "certResolver", skip_serializing_if = "Option::is_none")]
    cert_resolver: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    domains: Option<Vec<TlsDomain>>,
}

#[derive(Debug, Serialize)]
struct TlsDomain {
    main: String,
    sans: Vec<String>,
}

#[derive(Debug, Serialize)]
struct StaticDynamicConfig {
    http: StaticHttpConfig,
}

#[derive(Debug, Serialize)]
struct StaticHttpConfig {
    routers: std::collections::BTreeMap<String, StaticRouter>,
    services: std::collections::BTreeMap<String, TraefikService>,
    #[serde(skip_serializing_if = "Option::is_none")]
    middlewares: Option<std::collections::BTreeMap<String, StaticMiddleware>>,
}

/// entryPoint`web`(:80)宛のリクエストをwebsecure(:443)へ301リダイレクトする
/// ミドルウェア(`SAHAI_HTTPS_REDIRECT=true`のときのみ生成する)。
#[derive(Debug, Serialize)]
struct StaticMiddleware {
    #[serde(rename = "redirectScheme")]
    redirect_scheme: RedirectScheme,
}

#[derive(Debug, Serialize)]
struct RedirectScheme {
    scheme: String,
    permanent: bool,
}

#[derive(Debug, Serialize)]
struct TraefikService {
    #[serde(rename = "loadBalancer")]
    load_balancer: LoadBalancer,
}

#[derive(Debug, Serialize)]
struct LoadBalancer {
    servers: Vec<Server>,
}

#[derive(Debug, Serialize)]
struct Server {
    url: String,
}

pub struct RouteWriter {
    dynamic_dir: PathBuf,
    /// `is_http`ポートを持たないサービスの転送先、および管理画面(Web UI+API)自体の
    /// 転送先。sahai-server自身のdocker-compose上のアドレス(例: `http://sahai-server:8080`)。
    /// Web UI(SPA)はsahai-serverが自分自身で配信し(`tower-http::ServeDir`)、
    /// `window.location.hostname`からアクセス元のサブドメインを判定して
    /// `/api/not-service`へ問い合わせる。
    app_internal_url: String,
    cert_resolver: String,
    /// domain・https_redirectはWeb UIの設定画面から保存後すぐ変更できるため、
    /// 構築時に固定値をコピーせず`SharedSettings`を保持して呼び出しのたびに
    /// `.read().await`で最新値を読む(settings.rs参照)。
    settings: SharedSettings,
}

impl RouteWriter {
    pub fn new(
        dynamic_dir: PathBuf,
        app_internal_url: String,
        cert_resolver: String,
        settings: SharedSettings,
    ) -> Self {
        RouteWriter {
            dynamic_dir,
            app_internal_url,
            cert_resolver,
            settings,
        }
    }

    fn route_file_path(&self, subdomain: &str) -> PathBuf {
        self.dynamic_dir.join(format!("{subdomain}.yml"))
    }

    /// 冪等にルートファイルを書き出す。is_httpポートがあればそのhost_portへ、
    /// なければNot HTTP Serviceページ(Control Plane自身)へ向ける。
    pub async fn write_route(&self, service: &ServiceDetail) -> Result<(), TraefikError> {
        let https_redirect = self.settings.read().await.https_redirect;
        let target_url = self.resolve_target_url(service);
        let router_name = service.service.name.clone();
        let rule = format!("Host(`{}`)", service.service.subdomain);

        let mut routers = std::collections::BTreeMap::new();
        if https_redirect {
            routers.insert(
                router_name.clone(),
                Router {
                    rule,
                    service: router_name.clone(),
                    entry_points: Some(vec![ENTRY_POINT_WEBSECURE.to_string()]),
                    tls: Some(Tls {
                        cert_resolver: Some(self.cert_resolver.clone()),
                    }),
                },
            );
        } else {
            routers.insert(
                router_name.clone(),
                Router {
                    rule: rule.clone(),
                    service: router_name.clone(),
                    entry_points: Some(vec![ENTRY_POINT_WEB.to_string()]),
                    tls: None,
                },
            );
            routers.insert(
                tls_twin_router_name(&router_name),
                Router {
                    rule,
                    service: router_name.clone(),
                    entry_points: Some(vec![ENTRY_POINT_WEBSECURE.to_string()]),
                    tls: Some(Tls {
                        cert_resolver: None,
                    }),
                },
            );
        }

        let mut services = std::collections::BTreeMap::new();
        services.insert(
            router_name,
            TraefikService {
                load_balancer: LoadBalancer {
                    servers: vec![Server { url: target_url }],
                },
            },
        );

        let config = DynamicConfig {
            http: HttpConfig { routers, services },
        };
        let yaml = serde_yaml::to_string(&config)?;

        tokio::fs::create_dir_all(&self.dynamic_dir).await?;
        tokio::fs::write(self.route_file_path(&service.service.subdomain), yaml).await?;
        Ok(())
    }

    /// 管理画面(sahai.example.com、Web UI+API統合)のルート+レジストリ用ルート+
    /// 未登録サブドメイン用のワイルドカードcatch-allルートを冪等に書き出す
    /// per-serviceのルートと異なりstart/restart時
    /// ではなく起動時に一度だけ呼ばれる(main.rs参照)。Web UIとAPIは同一の
    /// sahai-serverコンテナが配信するため単一ルーター・単一サービスで済む。
    /// `registry_internal_url`はregistryコンテナのdocker-compose上のアドレス
    /// (例: `http://registry:5000`)。レジストリもTraefik配下でホストするため
    /// 専用ルートが要る。このルートが無いとワイルドカードcatch-allに
    /// 飲み込まれ、`docker login`のリクエストがWeb UIへ誤って転送されてしまう。
    pub async fn write_static_admin_routes(
        &self,
        registry_internal_url: &str,
    ) -> Result<(), TraefikError> {
        let (domain, https_redirect) = {
            let settings = self.settings.read().await;
            (settings.domain.clone(), settings.https_redirect)
        };
        let admin_host = format!("sahai.{domain}");
        let registry_host = format!("registry.sahai.{domain}");
        // ルーター名・サービス名にはアンダースコアを使う。Traefikのファイルプロバイダは
        // dynamicディレクトリ配下を1つの設定へマージするため、ここの名前はサービス別
        // ルート(ルーター名=サービス名)と同じ名前空間に載る。サービス名は[a-z0-9-]
        // しか使えないので、アンダースコアを含めておけばどんなサービス名とも衝突しない
        let mut routers = std::collections::BTreeMap::new();
        insert_static_router_pair(
            &mut routers,
            StaticRouteSpec {
                name: "sahai_app",
                service: "sahai_app",
                rule: format!("Host(`{admin_host}`)"),
                priority: 100,
                cert_resolver: &self.cert_resolver,
                domains: None,
            },
            https_redirect,
        );
        routers.insert(
            "sahai_registry".to_string(),
            StaticRouter {
                rule: format!("Host(`{registry_host}`)"),
                service: "sahai_registry".to_string(),
                priority: 100,
                // registryだけはhttps_redirectの値に関わらず常にwebsecure専用・tls付き
                // にする。`docker push`/`docker login`等のDockerツールチェーンは
                // 既定でHTTPS必須でplain httpへのフォールバックを行わないため
                // (実機の`docker push`検証で発覚: https_redirect=falseでtlsを消したら
                // 「404 Not Found」でpushが失敗した)。certResolverだけは
                // https_redirect=falseのとき省略し、公開TLDでないローカルドメインでの
                // 無駄なACME証明書取得の試み(失敗ログ)を防ぐ。tlsブロック自体は
                // 残るためTraefikの既定(自己署名)証明書でHTTPSに応答でき、
                // Docker側は自己署名証明書でも実際にpushが成功することを確認済み
                entry_points: Some(vec!["websecure".to_string()]),
                middlewares: None,
                tls: Some(StaticTls {
                    cert_resolver: https_redirect.then(|| self.cert_resolver.clone()),
                    domains: None,
                }),
            },
        );
        insert_static_router_pair(
            &mut routers,
            StaticRouteSpec {
                name: "sahai_catchall",
                service: "sahai_app",
                // per-serviceルーター(Host(`<name>.<domain>`)、動的生成)・上記2つより
                // 優先度を下げ、どれにもマッチしなかった場合の受け皿とする
                rule: format!(r"HostRegexp(`^.+\.{}$`)", domain.replace('.', r"\.")),
                priority: 1,
                cert_resolver: &self.cert_resolver,
                domains: Some(vec![TlsDomain {
                    main: domain.clone(),
                    sans: vec![format!("*.{domain}")],
                }]),
            },
            https_redirect,
        );

        let mut middlewares = None;
        if https_redirect {
            // entryPoint web(:80)宛の全リクエストをwebsecure(:443)へリダイレクトする。
            // Traefikの静的設定(entryPoint自体)ではなくこの動的ルートで実現することで、
            // SAHAI_HTTPS_REDIRECT環境変数による起動時トグルが可能になる
            // (entryPointの静的設定はTraefik起動後に変更できないため)。
            let mut mw = std::collections::BTreeMap::new();
            mw.insert(
                "sahai_https_redirect".to_string(),
                StaticMiddleware {
                    redirect_scheme: RedirectScheme {
                        scheme: "https".to_string(),
                        permanent: true,
                    },
                },
            );
            middlewares = Some(mw);

            routers.insert(
                "sahai_https_redirect".to_string(),
                StaticRouter {
                    rule: "PathPrefix(`/`)".to_string(),
                    service: "sahai_app".to_string(),
                    priority: 1,
                    entry_points: Some(vec!["web".to_string()]),
                    middlewares: Some(vec!["sahai_https_redirect".to_string()]),
                    tls: None,
                },
            );
        }

        let mut services = std::collections::BTreeMap::new();
        services.insert(
            "sahai_app".to_string(),
            TraefikService {
                load_balancer: LoadBalancer {
                    servers: vec![Server {
                        url: self.app_internal_url.clone(),
                    }],
                },
            },
        );
        services.insert(
            "sahai_registry".to_string(),
            TraefikService {
                load_balancer: LoadBalancer {
                    servers: vec![Server {
                        url: registry_internal_url.to_string(),
                    }],
                },
            },
        );

        let config = StaticDynamicConfig {
            http: StaticHttpConfig {
                routers,
                services,
                middlewares,
            },
        };
        let yaml = serde_yaml::to_string(&config)?;

        tokio::fs::create_dir_all(&self.dynamic_dir).await?;
        tokio::fs::write(self.dynamic_dir.join("static-routes.yml"), yaml).await?;
        Ok(())
    }

    /// 初期設定完了前(domainがまだ空でHost()ルールを組み立てられない)でも
    /// Web UI/APIへ到達できるよう、Hostに依存しない暫定ルートを書き出す。
    /// 設定完了時にwrite_static_admin_routesが同じファイルを上書きし、
    /// 通常のdomainベースのルートに置き換わる
    pub async fn write_bootstrap_routes(&self) -> Result<(), TraefikError> {
        let mut routers = std::collections::BTreeMap::new();
        // 初期設定前はhttps_redirectの設定値も未確定なため、http・httpsのどちらで
        // アクセスされても初期設定画面へ到達できるよう常に両方を書き出す
        insert_static_router_pair(
            &mut routers,
            StaticRouteSpec {
                name: "sahai_app",
                service: "sahai_app",
                rule: "PathPrefix(`/`)".to_string(),
                priority: 1,
                cert_resolver: &self.cert_resolver,
                domains: None,
            },
            false,
        );

        let mut services = std::collections::BTreeMap::new();
        services.insert(
            "sahai_app".to_string(),
            TraefikService {
                load_balancer: LoadBalancer {
                    servers: vec![Server {
                        url: self.app_internal_url.clone(),
                    }],
                },
            },
        );

        let config = StaticDynamicConfig {
            http: StaticHttpConfig {
                routers,
                services,
                middlewares: None,
            },
        };
        let yaml = serde_yaml::to_string(&config)?;

        tokio::fs::create_dir_all(&self.dynamic_dir).await?;
        tokio::fs::write(self.dynamic_dir.join("static-routes.yml"), yaml).await?;
        Ok(())
    }

    /// サービス削除時・名前変更時の旧ルート削除。削除は外側(ルート)から順に行うため、
    /// コンテナ停止・DBレコード削除より先にこれを呼ぶ(deletion.rs参照)。
    pub async fn remove_route(&self, subdomain: &str) -> Result<(), TraefikError> {
        let path = self.route_file_path(subdomain);
        match tokio::fs::remove_file(&path).await {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(TraefikError::Io(e)),
        }
    }

    /// is_httpポートを持つコンテナへ、sahaiネットワーク越しにコンテナ名で直接向ける。
    /// ホスト経由にしないのは、`is_http`ポートをホストへ公開しないため
    /// (公開すると`https://<name>.<domain>`とは別に平文の到達経路ができる)。
    fn resolve_target_url(&self, service: &ServiceDetail) -> String {
        let http_target = service
            .containers
            .iter()
            .find_map(|c| c.ports.iter().find(|p| p.is_http).map(|p| (c, p)));

        match http_target {
            Some((container, port)) => format!(
                "http://{}:{}",
                sahai_core::naming::container_docker_name(container.container.id),
                port.container_port
            ),
            None => self.app_internal_url.clone(),
        }
    }
}

const ENTRY_POINT_WEB: &str = "web";
const ENTRY_POINT_WEBSECURE: &str = "websecure";

/// `https_redirect=false`のとき、web用ルーターと対で書き出すwebsecure用ルーターの名前。
/// サービス名に`_`は使えないため、どのサービス名とも衝突しない。
fn tls_twin_router_name(base: &str) -> String {
    format!("{base}_tls")
}

/// 1つの論理ルートに対応するルーターを登録する。
///
/// `https_redirect=true`ならwebsecure(:443)専用の1本だけを書き出す
/// (web(:80)は`sahai_https_redirect`ミドルウェアがhttpsへ飛ばす)。
/// falseのときはweb用とwebsecure用の**2本**に分ける。`tls`キーの有無が
/// プロトコル可用性を直接左右し、単一ルーターでは両プロトコルに応答できないため
/// (Router.tls参照)。2本に分けないと、片方のプロトコルだけ404になり
/// http・httpsで挙動が食い違う。
struct StaticRouteSpec<'a> {
    name: &'a str,
    /// ルーター名とは別に指定する。catchallは`sahai_app`へ相乗りするため一致しない。
    service: &'a str,
    rule: String,
    priority: u32,
    /// https_redirect=trueのときのwebsecure用ルーターに付ける証明書リゾルバ。
    cert_resolver: &'a str,
    /// ワイルドカード証明書が要るルート(catchall)のみ指定する。
    domains: Option<Vec<TlsDomain>>,
}

fn insert_static_router_pair(
    routers: &mut std::collections::BTreeMap<String, StaticRouter>,
    spec: StaticRouteSpec<'_>,
    https_redirect: bool,
) {
    let StaticRouteSpec {
        name,
        service,
        rule,
        priority,
        cert_resolver,
        domains,
    } = spec;

    if https_redirect {
        routers.insert(
            name.to_string(),
            StaticRouter {
                rule,
                service: service.to_string(),
                priority,
                entry_points: Some(vec![ENTRY_POINT_WEBSECURE.to_string()]),
                middlewares: None,
                tls: Some(StaticTls {
                    cert_resolver: Some(cert_resolver.to_string()),
                    domains,
                }),
            },
        );
        return;
    }

    routers.insert(
        name.to_string(),
        StaticRouter {
            rule: rule.clone(),
            service: service.to_string(),
            priority,
            entry_points: Some(vec![ENTRY_POINT_WEB.to_string()]),
            middlewares: None,
            tls: None,
        },
    );
    routers.insert(
        tls_twin_router_name(name),
        StaticRouter {
            rule,
            service: service.to_string(),
            priority,
            entry_points: Some(vec![ENTRY_POINT_WEBSECURE.to_string()]),
            middlewares: None,
            // certResolverもdomainsも付けない。ローカルドメインでの無駄なACME証明書
            // 取得の試み(失敗ログ)を避け、Traefik既定の自己署名証明書で応答する
            tls: Some(StaticTls {
                cert_resolver: None,
                domains: None,
            }),
        },
    );
}

#[cfg(test)]
mod tests {
    use crate::domain::{
        ContainerWithChildren, HealthStatus, Protocol, Service, ServiceContainer, ServiceDetail,
        ServicePort, ServiceStatus, SourceType,
    };

    use super::*;

    fn test_settings(domain: &str, https_redirect: bool) -> SharedSettings {
        std::sync::Arc::new(tokio::sync::RwLock::new(crate::settings::Settings {
            domain: domain.to_string(),
            https_redirect,
            registry_url: "registry.sahai.example.test".to_string(),
            api_token: "test".to_string(),
            dns_provider: "cloudflare".to_string(),
            acme_email: "admin@example.test".to_string(),
            registry_username: None,
            registry_password: None,
        }))
    }

    fn writer(dynamic_dir: PathBuf) -> RouteWriter {
        RouteWriter::new(
            dynamic_dir,
            "http://sahai-server:8080".to_string(),
            "cloudflare".to_string(),
            test_settings("example.com", true),
        )
    }

    fn service_with_ports(ports: Vec<ServicePort>) -> ServiceDetail {
        ServiceDetail {
            service: Service {
                id: 1,
                name: "myapp".to_string(),
                subdomain: "myapp.example.com".to_string(),
                source_type: SourceType::Image,
                image: Some("x:latest".to_string()),
                compose_content: None,
                env_vars: serde_json::json!({}),
                status: ServiceStatus::Stopped,
                last_error: None,
                health_status: HealthStatus::Unknown,
                last_health_check_at: None,
                created_at: "2026-01-01T00:00:00.000Z".to_string(),
                updated_at: "2026-01-01T00:00:00.000Z".to_string(),
            },
            containers: vec![ContainerWithChildren {
                container: ServiceContainer {
                    id: 10,
                    service_id: 1,
                    name: "myapp".to_string(),
                    health_status: HealthStatus::Unknown,
                    last_health_check_at: None,
                },
                ports,
                volumes: vec![],
            }],
            route_warning: None,
        }
    }

    /// is_httpのポートはホストに公開しないためhost_portを持たない
    fn http_port() -> ServicePort {
        ServicePort {
            id: 100,
            container_id: 10,
            container_port: 80,
            host_port: None,
            protocol: Protocol::Tcp,
            is_http: true,
        }
    }

    fn non_http_port(host_port: i64) -> ServicePort {
        ServicePort {
            id: 101,
            container_id: 10,
            container_port: 3306,
            host_port: Some(host_port),
            protocol: Protocol::Tcp,
            is_http: false,
        }
    }

    fn temp_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "sahai_traefik_test_{label}_{:x}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        dir
    }

    /// is_httpポートはホストに公開しないため、コンテナ名で直接向ける。
    /// ホスト経由に戻すと平文の到達経路ができるため、この期待値は動かさないこと。
    #[test]
    fn resolves_to_container_name_when_http_port_present() {
        let w = writer(PathBuf::from("/unused"));
        let service = service_with_ports(vec![non_http_port(3306), http_port()]);
        assert_eq!(w.resolve_target_url(&service), "http://svc-10:80");
    }

    #[test]
    fn resolves_to_own_app_when_no_http_port() {
        let w = writer(PathBuf::from("/unused"));
        let service = service_with_ports(vec![non_http_port(3306)]);
        assert_eq!(w.resolve_target_url(&service), "http://sahai-server:8080");
    }

    #[test]
    fn resolves_to_own_app_when_no_ports_at_all() {
        let w = writer(PathBuf::from("/unused"));
        let service = service_with_ports(vec![]);
        assert_eq!(w.resolve_target_url(&service), "http://sahai-server:8080");
    }

    #[tokio::test]
    async fn write_route_creates_file_with_expected_content() {
        let dir = temp_dir("write");
        let w = writer(dir.clone());
        let service = service_with_ports(vec![http_port()]);

        w.write_route(&service).await.unwrap();

        let path = dir.join("myapp.example.com.yml");
        let content = tokio::fs::read_to_string(&path).await.unwrap();
        let parsed: serde_yaml::Value = serde_yaml::from_str(&content).unwrap();

        assert_eq!(
            parsed["http"]["routers"]["myapp"]["rule"].as_str(),
            Some("Host(`myapp.example.com`)")
        );
        assert_eq!(
            parsed["http"]["routers"]["myapp"]["tls"]["certResolver"].as_str(),
            Some("cloudflare")
        );
        assert_eq!(
            parsed["http"]["services"]["myapp"]["loadBalancer"]["servers"][0]["url"].as_str(),
            Some("http://svc-10:80")
        );

        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    #[tokio::test]
    async fn write_route_is_idempotent_and_overwrites() {
        let dir = temp_dir("idempotent");
        let w = writer(dir.clone());

        w.write_route(&service_with_ports(vec![http_port()]))
            .await
            .unwrap();
        w.write_route(&service_with_ports(vec![http_port()]))
            .await
            .unwrap();

        let path = dir.join("myapp.example.com.yml");
        let content = tokio::fs::read_to_string(&path).await.unwrap();
        // 2回書いても1件のルートに収束し、転送先はコンテナ名のまま
        assert!(content.contains("http://svc-10:80"), "{content}");
        assert_eq!(content.matches("http://svc-10:80").count(), 1, "{content}");

        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    #[tokio::test]
    async fn remove_route_is_idempotent_when_file_missing() {
        let dir = temp_dir("remove_missing");
        let w = writer(dir.clone());
        // ディレクトリ自体を作っていない状態でも、存在しないファイルの削除はエラーにしない
        let result = w.remove_route("neverexisted.example.com").await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn remove_route_deletes_existing_file() {
        let dir = temp_dir("remove_existing");
        let w = writer(dir.clone());
        w.write_route(&service_with_ports(vec![http_port()]))
            .await
            .unwrap();

        let path = dir.join("myapp.example.com.yml");
        assert!(path.exists());

        w.remove_route("myapp.example.com").await.unwrap();
        assert!(!path.exists());

        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    // 管理画面(sahai.example.com)のパス分割ルート+未登録サブドメイン用の
    // ワイルドカードcatch-allルートを、他の動的ルートと同じ仕組み(sahai-serverが
    // /var/sahai/traefik/dynamic/配下へ書き出す)で生成する。以前はこの内容を
    // リポジトリ管理の静的YAMLとしてbind-mountしていたが、Dockerの
    // 「read-onlyでマウントしたディレクトリの中に単一ファイルを重ねてマウントできない」
    // という制約(実機のdocker-compose upで実際に踏んだ)を避けるため、この書き込み方式に
    // 統一した。
    /// 静的ルートのルーター名・サービス名がサービス名と衝突しないことを押さえる。
    /// Traefikのファイルプロバイダはdynamicディレクトリ配下を1つの設定へマージするため、
    /// 静的ルートと同名のサービスを登録できてしまうと、どちらかのルートが失われる。
    /// サービス名は`[a-z0-9-]`しか使えないので、アンダースコアを含めて回避する。
    #[tokio::test]
    async fn static_route_names_cannot_collide_with_any_service_name() {
        let dir = temp_dir("static_names");
        let w = writer(dir.clone());

        w.write_static_admin_routes("http://registry:5000")
            .await
            .unwrap();

        let content = tokio::fs::read_to_string(dir.join("static-routes.yml"))
            .await
            .unwrap();
        let parsed: serde_yaml::Value = serde_yaml::from_str(&content).unwrap();

        for section in ["routers", "services"] {
            let map = parsed["http"][section].as_mapping().unwrap();
            for key in map.keys() {
                let name = key.as_str().unwrap();
                assert!(
                    name.contains('_'),
                    "{section}の'{name}'はサービス名として登録可能なため衝突しうる"
                );
                assert!(
                    sahai_core::validation::validate_service_name(name).is_err(),
                    "{section}の'{name}'はサービス名として有効なため衝突しうる"
                );
            }
        }

        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    #[tokio::test]
    async fn write_static_admin_routes_creates_file_with_expected_content() {
        let dir = temp_dir("static_admin");
        let w = writer(dir.clone());

        w.write_static_admin_routes("http://registry:5000")
            .await
            .unwrap();

        let path = dir.join("static-routes.yml");
        let content = tokio::fs::read_to_string(&path).await.unwrap();
        let parsed: serde_yaml::Value = serde_yaml::from_str(&content).unwrap();

        // Web UIとAPIは同一のsahai-serverコンテナが配信するため単一ルーターで済む
        assert_eq!(
            parsed["http"]["routers"]["sahai_app"]["rule"].as_str(),
            Some("Host(`sahai.example.com`)")
        );
        assert_eq!(
            parsed["http"]["routers"]["sahai_app"]["priority"].as_i64(),
            Some(100)
        );
        assert_eq!(
            parsed["http"]["services"]["sahai_app"]["loadBalancer"]["servers"][0]["url"].as_str(),
            Some("http://sahai-server:8080")
        );

        // registry.sahai.<domain>宛のルート。
        // 以前はこのルートが存在せず、ワイルドカードcatch-allに飲み込まれて
        // 誤ってWeb UIへ転送されていた(実機のdocker login検証で発覚)
        assert_eq!(
            parsed["http"]["routers"]["sahai_registry"]["rule"].as_str(),
            Some("Host(`registry.sahai.example.com`)")
        );
        assert_eq!(
            parsed["http"]["routers"]["sahai_registry"]["priority"].as_i64(),
            Some(100)
        );
        assert_eq!(
            parsed["http"]["services"]["sahai_registry"]["loadBalancer"]["servers"][0]["url"]
                .as_str(),
            Some("http://registry:5000")
        );

        assert_eq!(
            parsed["http"]["routers"]["sahai_catchall"]["rule"].as_str(),
            Some("HostRegexp(`^.+\\.example\\.com$`)")
        );
        assert_eq!(
            parsed["http"]["routers"]["sahai_catchall"]["priority"].as_i64(),
            Some(1)
        );
        assert_eq!(
            parsed["http"]["routers"]["sahai_catchall"]["service"].as_str(),
            Some("sahai_app")
        );
        assert_eq!(
            parsed["http"]["routers"]["sahai_catchall"]["tls"]["domains"][0]["main"].as_str(),
            Some("example.com")
        );
        assert_eq!(
            parsed["http"]["routers"]["sahai_catchall"]["tls"]["domains"][0]["sans"][0].as_str(),
            Some("*.example.com")
        );

        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    #[tokio::test]
    async fn write_bootstrap_routes_creates_host_agnostic_routes() {
        let dir = temp_dir("bootstrap");
        let w = writer(dir.clone());

        w.write_bootstrap_routes().await.unwrap();

        let path = dir.join("static-routes.yml");
        let content = tokio::fs::read_to_string(&path).await.unwrap();
        let parsed: serde_yaml::Value = serde_yaml::from_str(&content).unwrap();

        assert_eq!(
            parsed["http"]["routers"]["sahai_app"]["rule"].as_str(),
            Some("PathPrefix(`/`)")
        );
        assert_eq!(
            parsed["http"]["routers"]["sahai_app"]["priority"].as_i64(),
            Some(1)
        );
        assert!(parsed["http"]["routers"]["sahai_app"]["tls"].is_null());
        // 初期設定前はhttps_redirectの設定値も未確定なため、httpでもhttpsでも
        // 初期設定画面へ到達できるようwebsecure用ルーターも書き出す
        assert_eq!(
            parsed["http"]["routers"]["sahai_app_tls"]["rule"].as_str(),
            Some("PathPrefix(`/`)")
        );
        assert!(parsed["http"]["routers"]["sahai_app_tls"]["tls"].is_mapping());
        assert_eq!(
            parsed["http"]["services"]["sahai_app"]["loadBalancer"]["servers"][0]["url"].as_str(),
            Some("http://sahai-server:8080")
        );

        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    #[tokio::test]
    async fn write_static_admin_routes_uses_configured_domain() {
        let dir = temp_dir("static_admin_custom_domain");
        let w = RouteWriter::new(
            dir.clone(),
            "http://sahai-server:8080".to_string(),
            "cloudflare".to_string(),
            test_settings("example.com", true),
        );

        w.write_static_admin_routes("http://registry:5000")
            .await
            .unwrap();

        let path = dir.join("static-routes.yml");
        let content = tokio::fs::read_to_string(&path).await.unwrap();
        let parsed: serde_yaml::Value = serde_yaml::from_str(&content).unwrap();

        assert_eq!(
            parsed["http"]["routers"]["sahai_app"]["rule"].as_str(),
            Some("Host(`sahai.example.com`)")
        );
        assert_eq!(
            parsed["http"]["routers"]["sahai_registry"]["rule"].as_str(),
            Some("Host(`registry.sahai.example.com`)")
        );
        assert_eq!(
            parsed["http"]["routers"]["sahai_catchall"]["rule"].as_str(),
            Some("HostRegexp(`^.+\\.example\\.com$`)")
        );
        assert_eq!(
            parsed["http"]["routers"]["sahai_catchall"]["tls"]["domains"][0]["main"].as_str(),
            Some("example.com")
        );
        assert_eq!(
            parsed["http"]["routers"]["sahai_catchall"]["tls"]["domains"][0]["sans"][0].as_str(),
            Some("*.example.com")
        );

        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    #[tokio::test]
    async fn write_static_admin_routes_is_idempotent_and_overwrites() {
        let dir = temp_dir("static_admin_idempotent");
        let w = writer(dir.clone());

        w.write_static_admin_routes("http://registry:5000")
            .await
            .unwrap();
        w.write_static_admin_routes("http://registry:9999")
            .await
            .unwrap();

        let path = dir.join("static-routes.yml");
        let content = tokio::fs::read_to_string(&path).await.unwrap();
        assert!(content.contains("9999"));
        assert!(!content.contains("registry:5000"));

        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    // SAHAI_HTTPS_REDIRECT=true(既定)のとき、per-serviceルートはwebsecureのみに絞られ、
    // entryPoint web(:80)宛の全リクエストをwebsecureへリダイレクトするミドルウェアが
    // 生成されることを確認する。
    #[tokio::test]
    async fn write_route_restricts_to_websecure_when_https_redirect_enabled() {
        let dir = temp_dir("https_redirect_on_route");
        let w = writer(dir.clone());
        w.write_route(&service_with_ports(vec![http_port()]))
            .await
            .unwrap();

        let path = dir.join("myapp.example.com.yml");
        let content = tokio::fs::read_to_string(&path).await.unwrap();
        let parsed: serde_yaml::Value = serde_yaml::from_str(&content).unwrap();

        assert_eq!(
            parsed["http"]["routers"]["myapp"]["entryPoints"][0].as_str(),
            Some("websecure")
        );

        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    // SAHAI_HTTPS_REDIRECT=falseのとき、per-serviceルートはweb(:80)用と
    // websecure(:443)用の2本に分かれる。tlsキーの有無がプロトコル可用性を左右し
    // (空でもwebsecure専用になりweb:80から一切応答しなくなる。実機検証で判明)、
    // 単一ルーターでは両プロトコルに応答できないため。2本に分けないと、
    // httpでは見えるのにhttpsでは404という食い違いが起きる。
    #[tokio::test]
    async fn write_route_serves_both_protocols_when_https_redirect_disabled() {
        let dir = temp_dir("https_redirect_off_route");
        let w = RouteWriter::new(
            dir.clone(),
            "http://sahai-server:8080".to_string(),
            "cloudflare".to_string(),
            test_settings("example.com", false),
        );
        w.write_route(&service_with_ports(vec![http_port()]))
            .await
            .unwrap();

        let path = dir.join("myapp.example.com.yml");
        let content = tokio::fs::read_to_string(&path).await.unwrap();
        let parsed: serde_yaml::Value = serde_yaml::from_str(&content).unwrap();

        // web(:80)用は平文。tlsブロックを付けるとweb:80から応答しなくなる
        assert_eq!(
            parsed["http"]["routers"]["myapp"]["entryPoints"][0].as_str(),
            Some("web")
        );
        assert!(
            parsed["http"]["routers"]["myapp"]["tls"].is_null(),
            "web(:80)用ルーターにtlsブロックを付けてはいけない: {content}"
        );

        // websecure(:443)用。certResolverは付けず、Traefik既定の自己署名証明書で応答する
        // (ローカルドメインでの無駄なACME証明書取得を試みないため)
        assert_eq!(
            parsed["http"]["routers"]["myapp_tls"]["entryPoints"][0].as_str(),
            Some("websecure")
        );
        assert!(
            parsed["http"]["routers"]["myapp_tls"]["tls"].is_mapping(),
            "websecure(:443)用ルーターにはtlsブロックが要る: {content}"
        );
        assert!(
            parsed["http"]["routers"]["myapp_tls"]["tls"]["certResolver"].is_null(),
            "https_redirect=falseのときcertResolverは省略すべき: {content}"
        );
        // 2本とも同じルール・同じ転送先を指す
        assert_eq!(
            parsed["http"]["routers"]["myapp_tls"]["rule"].as_str(),
            parsed["http"]["routers"]["myapp"]["rule"].as_str()
        );
        assert_eq!(
            parsed["http"]["routers"]["myapp_tls"]["service"].as_str(),
            Some("myapp")
        );

        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    /// websecure用ルーターの名前は、どのサービス名とも衝突してはいけない
    /// (Traefikのファイルプロバイダは全ルートを1つの名前空間へマージするため)。
    /// サービス名に`_`は使えないので、それを含めることで回避している。
    #[tokio::test]
    async fn tls_twin_router_name_cannot_collide_with_any_service_name() {
        let name = tls_twin_router_name("myapp");
        assert_eq!(name, "myapp_tls");
        assert!(sahai_core::validation::validate_service_name(&name).is_err());
    }

    #[tokio::test]
    async fn write_static_admin_routes_adds_https_redirect_router_when_enabled() {
        let dir = temp_dir("https_redirect_on_static");
        let w = writer(dir.clone());
        w.write_static_admin_routes("http://registry:5000")
            .await
            .unwrap();

        let path = dir.join("static-routes.yml");
        let content = tokio::fs::read_to_string(&path).await.unwrap();
        let parsed: serde_yaml::Value = serde_yaml::from_str(&content).unwrap();

        assert_eq!(
            parsed["http"]["routers"]["sahai_app"]["entryPoints"][0].as_str(),
            Some("websecure")
        );
        assert_eq!(
            parsed["http"]["routers"]["sahai_https_redirect"]["rule"].as_str(),
            Some("PathPrefix(`/`)")
        );
        assert_eq!(
            parsed["http"]["routers"]["sahai_https_redirect"]["entryPoints"][0].as_str(),
            Some("web")
        );
        assert_eq!(
            parsed["http"]["routers"]["sahai_https_redirect"]["middlewares"][0].as_str(),
            Some("sahai_https_redirect")
        );
        assert!(
            parsed["http"]["routers"]["sahai_https_redirect"]["tls"].is_null(),
            "web(:80)専用ルーターにtlsを設定してはいけない"
        );
        assert_eq!(
            parsed["http"]["middlewares"]["sahai_https_redirect"]["redirectScheme"]["scheme"]
                .as_str(),
            Some("https")
        );
        assert_eq!(
            parsed["http"]["middlewares"]["sahai_https_redirect"]["redirectScheme"]["permanent"]
                .as_bool(),
            Some(true)
        );

        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    #[tokio::test]
    async fn write_static_admin_routes_omits_redirect_when_https_redirect_disabled() {
        let dir = temp_dir("https_redirect_off_static");
        let w = RouteWriter::new(
            dir.clone(),
            "http://sahai-server:8080".to_string(),
            "cloudflare".to_string(),
            test_settings("example.com", false),
        );
        w.write_static_admin_routes("http://registry:5000")
            .await
            .unwrap();

        let path = dir.join("static-routes.yml");
        let content = tokio::fs::read_to_string(&path).await.unwrap();
        let parsed: serde_yaml::Value = serde_yaml::from_str(&content).unwrap();

        assert!(
            parsed["http"]["routers"]["sahai_https_redirect"].is_null(),
            "https_redirect=falseのときリダイレクトルーターを生成してはいけない: {content}"
        );
        assert!(
            parsed["http"]["middlewares"].is_null(),
            "https_redirect=falseのときmiddlewaresキー自体を出力すべきではない: {content}"
        );
        // 管理画面・catchallとも、web(:80)用の平文ルーターとwebsecure(:443)用の
        // tls付きルーターの2本に分かれる。片方だけだと、そのプロトコルでしか
        // 応答せず(tlsキーの有無がプロトコル可用性を左右する)、http・httpsで
        // 挙動が食い違ってしまう
        for name in ["sahai_app", "sahai_catchall"] {
            assert_eq!(
                parsed["http"]["routers"][name]["entryPoints"][0].as_str(),
                Some("web"),
                "{name}はweb(:80)専用の平文ルーターであるべき: {content}"
            );
            assert!(
                parsed["http"]["routers"][name]["tls"].is_null(),
                "{name}(web用)にtlsブロックを付けてはいけない: {content}"
            );

            let tls_name = format!("{name}_tls");
            assert_eq!(
                parsed["http"]["routers"][&tls_name]["entryPoints"][0].as_str(),
                Some("websecure"),
                "{tls_name}が無いとhttpsで404になる: {content}"
            );
            assert!(
                parsed["http"]["routers"][&tls_name]["tls"].is_mapping(),
                "{tls_name}にはtlsブロックが要る: {content}"
            );
            assert!(
                parsed["http"]["routers"][&tls_name]["tls"]["certResolver"].is_null(),
                "https_redirect=falseのときcertResolverは省略すべき: {content}"
            );
            assert_eq!(
                parsed["http"]["routers"][&tls_name]["rule"].as_str(),
                parsed["http"]["routers"][name]["rule"].as_str(),
                "2本のルーターは同じルールを持つべき: {content}"
            );
            assert_eq!(
                parsed["http"]["routers"][&tls_name]["priority"].as_i64(),
                parsed["http"]["routers"][name]["priority"].as_i64(),
                "2本のルーターは同じ優先度を持つべき: {content}"
            );
        }
        // web(:80)からも平文httpで直接アクセスできることを示すため、通常ルートは残る
        assert_eq!(
            parsed["http"]["routers"]["sahai_registry"]["rule"].as_str(),
            Some("Host(`registry.sahai.example.com`)")
        );
        // registryだけはhttps_redirectの値に関わらず常にtlsブロックを持つ(websecure専用)。
        // `docker push`/`docker login`等のDockerツールチェーンは既定でHTTPS必須であり、
        // plain httpへのフォールバックを行わないため(実機の`docker push`検証で発覚:
        // https_redirectをfalseにしてregistryルートのtlsを消したところ、
        // 「404 Not Found」でpushが失敗した)。certResolverはhttps_redirect=falseのとき
        // 省略し無駄なACME試行はしないが、tlsブロック自体は残して自己署名証明書での
        // HTTPS応答を維持する
        assert!(
            parsed["http"]["routers"]["sahai_registry"]["tls"].is_mapping(),
            "registryはhttps_redirect=falseでもtlsブロックを維持すべき(Dockerツールチェーンの既定HTTPS要求のため): {content}"
        );
        assert!(
            parsed["http"]["routers"]["sahai_registry"]["tls"]["certResolver"].is_null(),
            "https_redirect=falseのときcertResolverは省略すべき(ACME証明書取得を試みない)"
        );
        assert_eq!(
            parsed["http"]["routers"]["sahai_registry"]["entryPoints"][0].as_str(),
            Some("websecure")
        );

        let _ = tokio::fs::remove_dir_all(&dir).await;
    }
}
