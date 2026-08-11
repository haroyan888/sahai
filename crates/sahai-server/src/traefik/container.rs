//! Traefikコンテナ自体の再作成。DNSプロバイダ/ACMEメールはTraefikの
//! certificatesResolvers(静的設定・CLI引数として渡す。compose.yaml参照)としてしか
//! 渡せず、動的設定ファイルでホットリロードできないため、変更を反映するには
//! コンテナの再作成が必要になる。
//!
//! bollard直接操作で実装する。Windows(Docker Desktop)ではコンテナ側マウント先に
//! Windowsパスを指定できないため、`docker compose`をサブプロセスとして呼ぶ方式
//! (compose.yamlへのホスト同一パスマウントが必要)は使えない。既存のTraefikコンテナ
//! (ラベル`com.docker.compose.service=traefik`で検索)をinspectし、その設定
//! (イメージ・起動コマンド・マウント・ポート・ネットワーク等)を複製して作り直す。
//! Envだけは、現在のコンテナのEnvに`.sahai.env`ファイルの最新内容を重ねて
//! 組み立て直す(`env_file:`ディレクティブの実行時展開に相当する)。

use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::time::Duration;

use bollard::container::{
    Config, CreateContainerOptions, InspectContainerOptions, ListContainersOptions,
    RemoveContainerOptions, StartContainerOptions, StopContainerOptions,
};
use bollard::models::ContainerInspectResponse;
use bollard::Docker;

#[derive(Debug, thiserror::Error)]
pub enum ContainerError {
    #[error("Dockerデーモンとの通信に失敗しました: {0}")]
    Bollard(#[from] bollard::errors::Error),
    #[error("既存のTraefikコンテナが見つかりません(com.docker.compose.service=traefikラベル)")]
    NotFound,
    #[error(".sahai.envの読み込みに失敗しました: {0}")]
    EnvFile(#[from] crate::env_file::EnvFileError),
    #[error("再作成後もTraefikコンテナが起動状態になりませんでした")]
    NotRunning,
}

/// 「起動しているか」を確認する回数(初回+再試行)。Docker Desktop環境では、
/// 古いコンテナのネットワーク後始末が完了するまで最大30秒程度かかることがあるため、
/// 合計の待ち時間がそれを十分上回るようにしている。
const MAX_ATTEMPTS: u32 = 8;
const RETRY_DELAY: Duration = Duration::from_secs(6);

const TRAEFIK_SERVICE_LABEL: &str = "com.docker.compose.service=traefik";

/// ACMEの静的設定のうち、DBの設定値から毎回組み立て直すCLIフラグ。
/// `compose.yaml`が`${SAHAI_ACME_EMAIL}`/`${SAHAI_DNS_PROVIDER}`を
/// `docker compose up`時点の`.env`から展開して埋め込むが、初回セットアップでは
/// その時点でまだ両方とも未設定(空文字列)のため、複製元のCmdをそのまま使うと
/// 永久に空のままになる。
const ACME_EMAIL_FLAG: &str = "--certificatesresolvers.letsencrypt.acme.email=";
const ACME_DNS_PROVIDER_FLAG: &str =
    "--certificatesresolvers.letsencrypt.acme.dnschallenge.provider=";

/// 既存のTraefikコンテナをinspectして設定を複製し、停止・削除・再作成・起動する。
/// `env_file`(`.sahai.env`)の内容は呼び出し時点の最新の内容でEnvを組み立て直し、
/// ACME関連のCLIフラグ(`dns_provider`・`acme_email`)はDBの現在値で差し替える。
pub async fn recreate_traefik(
    docker: &Docker,
    env_file: &Path,
    dns_provider: &str,
    acme_email: &str,
) -> Result<(), ContainerError> {
    let container_id = find_traefik_container(docker).await?;
    let inspect = docker
        .inspect_container(&container_id, None::<InspectContainerOptions>)
        .await?;

    let name = inspect
        .name
        .as_deref()
        .unwrap_or(&container_id)
        .trim_start_matches('/')
        .to_string();

    let config = build_config(&inspect, env_file, dns_provider, acme_email).await?;

    // 既に停止している場合はエラーになるが、削除できれば十分なので無視する
    let _ = docker
        .stop_container(&container_id, None::<StopContainerOptions>)
        .await;
    docker
        .remove_container(
            &container_id,
            Some(RemoveContainerOptions {
                force: true,
                ..Default::default()
            }),
        )
        .await?;

    docker
        .create_container(
            Some(CreateContainerOptions {
                name: name.clone(),
                platform: None,
            }),
            config,
        )
        .await?;
    docker
        .start_container(&name, None::<StartContainerOptions<String>>)
        .await?;

    wait_until_running(docker, &name).await
}

/// 起動時の整合。TraefikコンテナのEnv/Cmdが現在の設定と一致していなければ作り直す。
///
/// 認証情報はcompose.yamlではなくsahai-serverがbollard経由で渡す設計のため、
/// `docker compose up`でTraefikが作り直されると失われる。設定を保存し直すまで
/// 気付けず、証明書の更新時になって初めて失敗するので、起動のたびに整合させる。
///
/// 一致していれば何もしない。毎回作り直すとsahai-serverの再起動のたびに
/// Traefikが瞬断し、稼働中のサービスへのアクセスが切れてしまう。
///
/// 戻り値は再作成したかどうか。
pub async fn reconcile_traefik(
    docker: &Docker,
    env_file: &Path,
    dns_provider: &str,
    acme_email: &str,
) -> Result<bool, ContainerError> {
    let container_id = find_traefik_container(docker).await?;
    let inspect = docker
        .inspect_container(&container_id, None::<InspectContainerOptions>)
        .await?;
    let desired = build_config(&inspect, env_file, dns_provider, acme_email).await?;

    let current = inspect.config.clone().unwrap_or_default();
    if env_matches(current.env.as_deref(), desired.env.as_deref())
        && current.cmd.as_deref() == desired.cmd.as_deref()
    {
        return Ok(false);
    }

    recreate_traefik(docker, env_file, dns_provider, acme_email).await?;
    Ok(true)
}

/// Envは順序が保証されない(Docker側が返す順序に依存する)ので、集合として比較する。
fn env_matches(current: Option<&[String]>, desired: Option<&[String]>) -> bool {
    let to_set = |v: Option<&[String]>| -> std::collections::BTreeSet<String> {
        v.unwrap_or_default().iter().cloned().collect()
    };
    to_set(current) == to_set(desired)
}

async fn find_traefik_container(docker: &Docker) -> Result<String, ContainerError> {
    let mut filters: HashMap<&str, Vec<&str>> = HashMap::new();
    filters.insert("label", vec![TRAEFIK_SERVICE_LABEL]);
    let containers = docker
        .list_containers(Some(ListContainersOptions {
            all: true,
            filters,
            ..Default::default()
        }))
        .await?;

    containers
        .into_iter()
        .next()
        .and_then(|c| c.id)
        .ok_or(ContainerError::NotFound)
}

/// inspect結果(既存コンテナ)から、再作成用のConfigを組み立てる。イメージ・
/// ポート・ラベル・HostConfig(マウント・ネットワーク・再起動ポリシー等)はそのまま複製し、
/// Envだけは`.sahai.env`の最新内容を重ねて組み立て直す(`env_file:`ディレクティブの
/// 実行時展開に相当)。起動コマンドも基本は複製だが、ACME関連の2フラグだけは
/// `override_acme_cmd_flags`でDBの現在値に差し替える。
async fn build_config(
    inspect: &ContainerInspectResponse,
    env_file: &Path,
    dns_provider: &str,
    acme_email: &str,
) -> Result<Config<String>, ContainerError> {
    let container_config = inspect.config.clone().unwrap_or_default();
    let image = container_config.image.clone();

    let env = merge_env(
        container_config.env.as_deref().unwrap_or_default(),
        &crate::env_file::load(env_file).await?,
    );

    let cmd = container_config
        .cmd
        .map(|cmd| override_acme_cmd_flags(&cmd, dns_provider, acme_email));

    Ok(Config {
        exposed_ports: container_config.exposed_ports,
        env: Some(env),
        cmd,
        image,
        working_dir: container_config.working_dir,
        entrypoint: container_config.entrypoint,
        labels: container_config.labels,
        host_config: inspect.host_config.clone(),
        ..Default::default()
    })
}

/// 現在のコンテナのEnvを土台に、`.sahai.env`の内容を上書きして組み立て直す
/// (純粋関数。Docker無しでテストできるよう分離している)。
///
/// 土台をイメージの既定Envにはしない。compose.yamlの`environment:`で渡された値まで
/// 落ちてしまい、再作成のたびにコンテナの設定が変わってしまうため(開発用composeの
/// `SAHAI_DOMAIN`がこれで消え、Traefikの開発用ルートが全て404になる問題を踏んだ)。
/// 現在のコンテナのEnvにはイメージの既定値も既に含まれている。
///
/// 代償として、`.sahai.env`から手で消したキーは再作成しても消えない
/// (`env_file::upsert`はキーを削除しないため、通常の設定変更では起きない)。
fn merge_env(current: &[String], overrides: &[(String, String)]) -> Vec<String> {
    let mut env_map: BTreeMap<&str, &str> = BTreeMap::new();
    for entry in current {
        if let Some((k, v)) = entry.split_once('=') {
            env_map.insert(k, v);
        }
    }
    for (k, v) in overrides {
        env_map.insert(k, v);
    }
    env_map
        .into_iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect()
}

/// 複製した起動コマンドのうち、ACMEのメールアドレスとDNSチャレンジのプロバイダ名だけを
/// 現在の設定値へ差し替える(純粋関数。Docker無しでテストできるよう分離している)。
///
/// 前方一致した要素のみを置き換え、それ以外の引数には一切触れない。該当フラグが
/// 存在しない場合は追加もしない — 利用者が`compose.yaml`を書き換えて意図的に
/// HTTPチャレンジ等へ変更している可能性があり、勝手にDNSチャレンジを注入すべきでは
/// ないため。
fn override_acme_cmd_flags(cmd: &[String], dns_provider: &str, acme_email: &str) -> Vec<String> {
    cmd.iter()
        .map(|arg| {
            if arg.starts_with(ACME_EMAIL_FLAG) {
                format!("{ACME_EMAIL_FLAG}{acme_email}")
            } else if arg.starts_with(ACME_DNS_PROVIDER_FLAG) {
                format!("{ACME_DNS_PROVIDER_FLAG}{dns_provider}")
            } else {
                arg.clone()
            }
        })
        .collect()
}

async fn wait_until_running(docker: &Docker, name: &str) -> Result<(), ContainerError> {
    for attempt in 1..=MAX_ATTEMPTS {
        let inspect = docker
            .inspect_container(name, None::<InspectContainerOptions>)
            .await?;
        if inspect.state.and_then(|s| s.running).unwrap_or(false) {
            return Ok(());
        }
        if attempt < MAX_ATTEMPTS {
            tokio::time::sleep(RETRY_DELAY).await;
            let _ = docker
                .start_container(name, None::<StartContainerOptions<String>>)
                .await;
        }
    }
    Err(ContainerError::NotRunning)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(entries: &[&str]) -> Vec<String> {
        entries.iter().map(|s| s.to_string()).collect()
    }

    fn overrides(entries: &[(&str, &str)]) -> Vec<(String, String)> {
        entries
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    /// composeの`environment:`で渡された値を土台に残す。イメージの既定Envを土台に
    /// すると、開発用composeのSAHAI_DOMAINのような値が再作成のたびに消える。
    #[test]
    fn merge_env_keeps_entries_not_in_env_file() {
        let current = env(&["PATH=/usr/bin", "SAHAI_DOMAIN=localhost"]);
        let merged = merge_env(&current, &overrides(&[("CF_DNS_API_TOKEN", "secret")]));
        assert_eq!(
            merged,
            env(&[
                "CF_DNS_API_TOKEN=secret",
                "PATH=/usr/bin",
                "SAHAI_DOMAIN=localhost"
            ])
        );
    }

    #[test]
    fn merge_env_overwrites_existing_keys() {
        let current = env(&["SAHAI_DNS_PROVIDER=route53"]);
        let merged = merge_env(
            &current,
            &overrides(&[("SAHAI_DNS_PROVIDER", "cloudflare")]),
        );
        assert_eq!(merged, env(&["SAHAI_DNS_PROVIDER=cloudflare"]));
    }

    /// 値に`=`を含む認証情報(base64等)を分断しない。
    #[test]
    fn merge_env_keeps_equals_sign_in_value() {
        let current = env(&["TOKEN=a=b=c"]);
        let merged = merge_env(&current, &[]);
        assert_eq!(merged, env(&["TOKEN=a=b=c"]));
    }

    /// Envはbuild_configが連想配列から組み立てるため、Docker側が返す順序とは限らない。
    /// 順序で比較すると、内容が同じでも毎回再作成が走りTraefikが瞬断する。
    #[test]
    fn env_matches_ignores_order() {
        let a = env(&["A=1", "B=2"]);
        let b = env(&["B=2", "A=1"]);
        assert!(env_matches(Some(&a), Some(&b)));
    }

    #[test]
    fn env_matches_detects_added_or_changed_entries() {
        let current = env(&["A=1"]);
        assert!(!env_matches(Some(&current), Some(&env(&["A=1", "B=2"]))));
        assert!(!env_matches(Some(&current), Some(&env(&["A=2"]))));
    }

    /// composeがTraefikを作り直すとDNS認証情報が落ちる。この状態を検出できないと
    /// 証明書の更新時まで気付けない。
    #[test]
    fn env_matches_detects_lost_credentials() {
        let after_compose_recreate = env(&["PATH=/usr/bin"]);
        let desired = env(&["PATH=/usr/bin", "CF_DNS_API_TOKEN=secret"]);
        assert!(!env_matches(Some(&after_compose_recreate), Some(&desired)));
    }

    #[test]
    fn env_matches_treats_none_as_empty() {
        assert!(env_matches(None, Some(&[])));
        assert!(env_matches(None, None));
        assert!(!env_matches(None, Some(&env(&["A=1"]))));
    }

    /// `docker compose up`はDNS設定を尋ねる前に実行されるため、複製元コンテナのCmdは
    /// `...acme.email=`・`...dnschallenge.provider=`が空文字列のまま焼き付いている。
    /// Cmdを差し替えずに複製すると、Traefikが「dnschallenge=trueなのにproviderが空」
    /// =チャレンジ未設定と解釈し、証明書取得に失敗し続ける。
    #[test]
    fn override_acme_cmd_flags_fills_in_empty_email_and_provider() {
        let cmd: Vec<String> = vec![
            "--entrypoints.web.address=:80".to_string(),
            "--certificatesresolvers.letsencrypt.acme.email=".to_string(),
            "--certificatesresolvers.letsencrypt.acme.dnschallenge=true".to_string(),
            "--certificatesresolvers.letsencrypt.acme.dnschallenge.provider=".to_string(),
        ];

        let result = override_acme_cmd_flags(&cmd, "route53", "admin@example.com");

        assert_eq!(
            result,
            vec![
                "--entrypoints.web.address=:80".to_string(),
                "--certificatesresolvers.letsencrypt.acme.email=admin@example.com".to_string(),
                "--certificatesresolvers.letsencrypt.acme.dnschallenge=true".to_string(),
                "--certificatesresolvers.letsencrypt.acme.dnschallenge.provider=route53"
                    .to_string(),
            ]
        );
    }

    #[test]
    fn override_acme_cmd_flags_replaces_stale_non_empty_values() {
        // プロバイダを変更した場合(既に別の値が入っている場合)も上書きできること
        let cmd: Vec<String> = vec![
            "--certificatesresolvers.letsencrypt.acme.email=old@example.com".to_string(),
            "--certificatesresolvers.letsencrypt.acme.dnschallenge.provider=cloudflare".to_string(),
        ];

        let result = override_acme_cmd_flags(&cmd, "route53", "new@example.com");

        assert_eq!(
            result,
            vec![
                "--certificatesresolvers.letsencrypt.acme.email=new@example.com".to_string(),
                "--certificatesresolvers.letsencrypt.acme.dnschallenge.provider=route53"
                    .to_string(),
            ]
        );
    }

    /// `dnschallenge.provider`は`dnschallenge`に前方一致する。前方一致の判定順序を
    /// 誤ると`--...dnschallenge=true`のほうを書き換えてしまうため、他の引数が
    /// 一切変化しないことを明示的に固定する。
    #[test]
    fn override_acme_cmd_flags_leaves_all_other_arguments_untouched() {
        let cmd: Vec<String> = vec![
            "--entrypoints.websecure.address=:443".to_string(),
            "--providers.file.directory=/var/sahai/traefik/dynamic".to_string(),
            "--certificatesresolvers.letsencrypt.acme.dnschallenge=true".to_string(),
            "--certificatesresolvers.letsencrypt.acme.storage=/acme/acme.json".to_string(),
            "--certificatesresolvers.letsencrypt.acme.dnschallenge.resolvers=1.1.1.1:53"
                .to_string(),
        ];

        let result = override_acme_cmd_flags(&cmd, "cloudflare", "admin@example.com");

        assert_eq!(result, cmd, "該当フラグが無い引数は素通しされるべき");
    }

    /// 利用者がcompose.yamlを書き換えてHTTPチャレンジ等に変更している可能性があるため、
    /// フラグ自体が存在しない場合に勝手に追加してはいけない。
    #[test]
    fn override_acme_cmd_flags_does_not_append_missing_flags() {
        let cmd: Vec<String> = vec!["--entrypoints.web.address=:80".to_string()];

        let result = override_acme_cmd_flags(&cmd, "cloudflare", "admin@example.com");

        assert_eq!(result.len(), 1);
        assert_eq!(result, cmd);
    }

    #[tokio::test]
    async fn returns_an_error_when_docker_is_unreachable() {
        // 実Dockerデーモンには一切触れない(実行中のTraefikコンテナを誤って
        // 再作成しないため。crate::docker::mod参照)。到達不能なクライアントでは
        // 必ず何らかのエラーになるはず
        let docker = crate::docker::unreachable_docker_client_for_test();
        let result = find_traefik_container(&docker).await;
        assert!(result.is_err());
    }

    /// 実Dockerデーモンに対する結合テスト。`cargo test -- --ignored`で明示的に実行する。
    /// 開発用スタック(dev.compose.yaml)が起動済みで、com.docker.compose.service=traefik
    /// ラベルを持つコンテナが存在する前提。再作成後も同じ名前でinspectでき、
    /// 実行状態になっていることを確認する
    #[tokio::test]
    #[ignore = "requires a running Docker daemon and the dev stack already up (docker compose -f dev.compose.yaml up -d)"]
    async fn e2e_recreates_traefik_container() {
        let docker = Docker::connect_with_local_defaults().unwrap();
        let env_file = Path::new("/nonexistent/.sahai.env");

        let before = find_traefik_container(&docker)
            .await
            .expect("事前にdev.compose.yamlのtraefikコンテナが起動している前提");
        let before_inspect = docker
            .inspect_container(&before, None::<InspectContainerOptions>)
            .await
            .unwrap();
        let name = before_inspect
            .name
            .unwrap()
            .trim_start_matches('/')
            .to_string();

        recreate_traefik(&docker, env_file, "cloudflare", "e2e@example.com")
            .await
            .expect("既存コンテナの設定を複製して再作成できるはず");

        let inspect = docker
            .inspect_container(&name, None::<InspectContainerOptions>)
            .await
            .expect("再作成後も同じ名前でinspectできるはず");
        assert_eq!(inspect.state.and_then(|s| s.running), Some(true));
    }

    /// `.sahai.env`に書かれた内容が、再作成後のTraefikコンテナのEnvに実際に反映される
    /// ことを検証する結合テスト。
    #[tokio::test]
    #[ignore = "requires a running Docker daemon and the dev stack already up (docker compose -f dev.compose.yaml up -d)"]
    async fn e2e_passes_env_file_values_to_recreated_container() {
        let docker = Docker::connect_with_local_defaults().unwrap();
        let env_file = std::env::temp_dir().join(format!(
            "sahai_traefik_e2e_{:x}.env",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        crate::env_file::upsert(
            &env_file,
            &[("SAHAI_ACME_EMAIL", "container-test@example.com")],
        )
        .await
        .expect(".sahai.envへ書き込めるはず");

        let before = find_traefik_container(&docker)
            .await
            .expect("事前にdev.compose.yamlのtraefikコンテナが起動している前提");
        let before_inspect = docker
            .inspect_container(&before, None::<InspectContainerOptions>)
            .await
            .unwrap();
        let name = before_inspect
            .name
            .unwrap()
            .trim_start_matches('/')
            .to_string();

        recreate_traefik(
            &docker,
            &env_file,
            "cloudflare",
            "container-test@example.com",
        )
        .await
        .expect("既存コンテナの設定を複製して再作成できるはず");
        let _ = tokio::fs::remove_file(&env_file).await;

        let inspect = docker
            .inspect_container(&name, None::<InspectContainerOptions>)
            .await
            .expect("再作成後も同じ名前でinspectできるはず");
        let env = inspect
            .config
            .and_then(|c| c.env)
            .expect("コンテナのenvが取得できるはず");
        assert!(env
            .iter()
            .any(|e| e == "SAHAI_ACME_EMAIL=container-test@example.com"));
    }
}
