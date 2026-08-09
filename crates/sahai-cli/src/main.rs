//! `sahai` CLI。サブコマンドの定義とディスパッチのみを持つ。

mod api_client;
mod commands;
mod config;

use std::path::PathBuf;

use clap::{Parser, Subcommand};

use api_client::ApiClient;
use commands::container_push::PushArgs;
use commands::service_create::CreateArgs;
use commands::service_update::UpdateArgs;
use config::CliConfig;

#[derive(Parser)]
#[command(name = "sahai")]
struct Cli {
    #[command(subcommand)]
    command: Command,

    /// TLS証明書検証をスキップする。SAHAI_DOMAIN=localhost等、DNS-01での証明書
    /// 発行ができず自己署名証明書のままのローカルテスト環境向け
    /// (実運用のドメインでは使うべきではない)。config.tomlの`[control_plane]`に
    /// `insecure = true`を設定すれば毎回指定しなくてもよい
    #[arg(long, global = true)]
    insecure: bool,
}

#[derive(Subcommand)]
enum Command {
    /// レジストリへのビルド+push(登録済みサービスのイメージ更新用)
    Container {
        #[command(subcommand)]
        action: ContainerAction,
    },
    /// サービスの作成・登録済みサービスへのライフサイクル操作
    Service {
        #[command(subcommand)]
        action: ServiceAction,
    },
    /// Control Plane APIへの認証トークンを保存
    Login,
    /// 設定ファイルの参照
    Config,
}

#[derive(Subcommand)]
enum ContainerAction {
    /// ビルド + レジストリpush(compose/image自動判別)
    Push {
        name: String,
        #[arg(long, default_value = ".")]
        context: PathBuf,
        #[arg(long = "build-arg", value_parser = parse_key_val)]
        build_arg: Vec<(String, String)>,
        #[arg(long)]
        platform: Option<String>,
        #[arg(long)]
        deploy: bool,
    },
}

#[derive(Subcommand)]
enum ServiceAction {
    /// プロジェクトをサーバーへアップロードし、サーバー側でビルド+push+新規登録まで行う。
    /// compose型にする場合、composeファイルはプロジェクトルート直下に置くこと
    /// (build.contextもルート配下しか指せない)。
    /// ポート・env・ボリュームはWeb UIで別途設定する(この場では扱わない)
    Create {
        name: String,
        #[arg(long, default_value = ".")]
        context: PathBuf,
        #[arg(long = "build-arg", value_parser = parse_key_val)]
        build_arg: Vec<(String, String)>,
        #[arg(long)]
        platform: Option<String>,
    },
    /// 登録済みサービスのプロジェクトを現在のディレクトリの状態でアップロードし、
    /// サーバー側でビルド+push(上書き方式)を行う。名前・source_typeは変更できない。
    /// ポート・env・ボリュームはWeb UIの責務のまま変更しない(compose型のみ、
    /// compose_content内のサービス追加/削除がServiceContainerへ自動反映される)
    Update {
        name: String,
        #[arg(long, default_value = ".")]
        context: PathBuf,
        #[arg(long = "build-arg", value_parser = parse_key_val)]
        build_arg: Vec<(String, String)>,
        #[arg(long)]
        platform: Option<String>,
        /// ビルド+push後にrestartを呼び、実際にデプロイまで行う
        #[arg(long)]
        deploy: bool,
    },
    /// 登録済みサービス一覧
    List {
        /// 生JSONで出力する(既定は人間可読なテーブル形式)
        #[arg(long)]
        json: bool,
    },
    /// 詳細・ヘルス・リソース使用量
    Status {
        name: String,
        /// 生JSONで出力する(既定は人間可読な整形表示)
        #[arg(long)]
        json: bool,
    },
    /// 起動
    Start {
        name: String,
        /// 生JSONで出力する(既定は人間可読な整形表示)
        #[arg(long)]
        json: bool,
    },
    /// 停止
    Stop {
        name: String,
        /// 生JSONで出力する(既定は人間可読な整形表示)
        #[arg(long)]
        json: bool,
    },
    /// 再起動(イメージ上書き後の再デプロイに使用)
    Restart {
        name: String,
        /// 生JSONで出力する(既定は人間可読な整形表示)
        #[arg(long)]
        json: bool,
    },
}

fn parse_key_val(s: &str) -> Result<(String, String), String> {
    let (k, v) = s
        .split_once('=')
        .ok_or_else(|| format!("KEY=VALUE形式で指定してください: {s}"))?;
    Ok((k.to_string(), v.to_string()))
}

#[tokio::main]
async fn main() -> std::process::ExitCode {
    let cli = Cli::parse();

    let result = run(cli).await;
    match result {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("エラー: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}

async fn run(cli: Cli) -> Result<(), String> {
    let cli_insecure = cli.insecure;
    match cli.command {
        Command::Login => commands::login::run(),
        Command::Config => commands::config_cmd::run(),
        Command::Container {
            action:
                ContainerAction::Push {
                    name,
                    context,
                    build_arg,
                    platform,
                    deploy,
                },
        } => {
            let config = CliConfig::load()?;
            let insecure = cli_insecure || config.control_plane.insecure;
            let client = ApiClient::new(
                config.control_plane.url,
                config.control_plane.token,
                insecure,
            );
            commands::container_push::run(
                &client,
                &config.registry.url,
                PushArgs {
                    name,
                    context,
                    build_args: build_arg,
                    platform,
                    deploy,
                },
            )
            .await
        }
        Command::Service { action } => {
            let config = CliConfig::load()?;
            let insecure = cli_insecure || config.control_plane.insecure;
            let client = ApiClient::new(
                config.control_plane.url,
                config.control_plane.token,
                insecure,
            );
            match action {
                ServiceAction::Create {
                    name,
                    context,
                    build_arg,
                    platform,
                } => {
                    commands::service_create::run(
                        &client,
                        CreateArgs {
                            name,
                            context,
                            build_args: build_arg,
                            platform,
                        },
                    )
                    .await
                }
                ServiceAction::Update {
                    name,
                    context,
                    build_arg,
                    platform,
                    deploy,
                } => {
                    commands::service_update::run(
                        &client,
                        UpdateArgs {
                            name,
                            context,
                            build_args: build_arg,
                            platform,
                            deploy,
                        },
                    )
                    .await
                }
                ServiceAction::List { json } => commands::service::list(&client, json).await,
                ServiceAction::Status { name, json } => {
                    commands::service::status(&client, &name, json).await
                }
                ServiceAction::Start { name, json } => {
                    commands::service::start(&client, &name, json).await
                }
                ServiceAction::Stop { name, json } => {
                    commands::service::stop(&client, &name, json).await
                }
                ServiceAction::Restart { name, json } => {
                    commands::service::restart(&client, &name, json).await
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_key_val_splits_on_first_equals() {
        assert_eq!(
            parse_key_val("KEY=VALUE").unwrap(),
            ("KEY".to_string(), "VALUE".to_string())
        );
    }

    #[test]
    fn parse_key_val_allows_equals_signs_in_value() {
        assert_eq!(
            parse_key_val("KEY=a=b=c").unwrap(),
            ("KEY".to_string(), "a=b=c".to_string())
        );
    }

    #[test]
    fn parse_key_val_rejects_missing_equals() {
        assert!(parse_key_val("NOEQUALSIGN").is_err());
    }

    #[test]
    fn cli_parses_container_push_with_all_options() {
        let cli = Cli::try_parse_from([
            "sahai",
            "container",
            "push",
            "myapp",
            "--context",
            "./build",
            "--build-arg",
            "FOO=bar",
            "--platform",
            "linux/amd64",
            "--deploy",
        ])
        .unwrap();

        match cli.command {
            Command::Container {
                action:
                    ContainerAction::Push {
                        name,
                        context,
                        build_arg,
                        platform,
                        deploy,
                    },
            } => {
                assert_eq!(name, "myapp");
                assert_eq!(context, PathBuf::from("./build"));
                assert_eq!(build_arg, vec![("FOO".to_string(), "bar".to_string())]);
                assert_eq!(platform.as_deref(), Some("linux/amd64"));
                assert!(deploy);
            }
            _ => panic!("Container Pushとしてパースされるべき"),
        }
    }

    #[test]
    fn cli_container_push_defaults_context_to_current_dir_and_deploy_false() {
        let cli = Cli::try_parse_from(["sahai", "container", "push", "myapp"]).unwrap();
        match cli.command {
            Command::Container {
                action:
                    ContainerAction::Push {
                        context, deploy, ..
                    },
            } => {
                assert_eq!(context, PathBuf::from("."));
                assert!(!deploy);
            }
            _ => panic!("Container Pushとしてパースされるべき"),
        }
    }

    #[test]
    fn cli_parses_service_create_with_all_options() {
        let cli = Cli::try_parse_from([
            "sahai",
            "service",
            "create",
            "myapp",
            "--context",
            "./build",
            "--build-arg",
            "FOO=bar",
            "--platform",
            "linux/amd64",
        ])
        .unwrap();

        match cli.command {
            Command::Service {
                action:
                    ServiceAction::Create {
                        name,
                        context,
                        build_arg,
                        platform,
                    },
            } => {
                assert_eq!(name, "myapp");
                assert_eq!(context, PathBuf::from("./build"));
                assert_eq!(build_arg, vec![("FOO".to_string(), "bar".to_string())]);
                assert_eq!(platform.as_deref(), Some("linux/amd64"));
            }
            _ => panic!("Service Createとしてパースされるべき"),
        }
    }

    #[test]
    fn cli_service_create_defaults_context_to_current_dir() {
        let cli = Cli::try_parse_from(["sahai", "service", "create", "myapp"]).unwrap();
        match cli.command {
            Command::Service {
                action: ServiceAction::Create { context, .. },
            } => {
                assert_eq!(context, PathBuf::from("."));
            }
            _ => panic!("Service Createとしてパースされるべき"),
        }
    }

    #[test]
    fn cli_parses_service_update_with_all_options() {
        let cli = Cli::try_parse_from([
            "sahai",
            "service",
            "update",
            "myapp",
            "--context",
            "./build",
            "--build-arg",
            "FOO=bar",
            "--platform",
            "linux/amd64",
            "--deploy",
        ])
        .unwrap();

        match cli.command {
            Command::Service {
                action:
                    ServiceAction::Update {
                        name,
                        context,
                        build_arg,
                        platform,
                        deploy,
                    },
            } => {
                assert_eq!(name, "myapp");
                assert_eq!(context, PathBuf::from("./build"));
                assert_eq!(build_arg, vec![("FOO".to_string(), "bar".to_string())]);
                assert_eq!(platform.as_deref(), Some("linux/amd64"));
                assert!(deploy);
            }
            _ => panic!("Service Updateとしてパースされるべき"),
        }
    }

    #[test]
    fn cli_service_update_defaults_context_to_current_dir_and_deploy_false() {
        let cli = Cli::try_parse_from(["sahai", "service", "update", "myapp"]).unwrap();
        match cli.command {
            Command::Service {
                action:
                    ServiceAction::Update {
                        context, deploy, ..
                    },
            } => {
                assert_eq!(context, PathBuf::from("."));
                assert!(!deploy);
            }
            _ => panic!("Service Updateとしてパースされるべき"),
        }
    }

    #[test]
    fn cli_parses_service_start_with_name() {
        let cli = Cli::try_parse_from(["sahai", "service", "start", "myapp"]).unwrap();
        match cli.command {
            Command::Service {
                action: ServiceAction::Start { name, json },
            } => {
                assert_eq!(name, "myapp");
                assert!(!json, "--json未指定時は既定でfalseのはず");
            }
            _ => panic!("Service Startとしてパースされるべき"),
        }
    }

    #[test]
    fn cli_service_start_accepts_json_flag() {
        let cli = Cli::try_parse_from(["sahai", "service", "start", "myapp", "--json"]).unwrap();
        match cli.command {
            Command::Service {
                action: ServiceAction::Start { json, .. },
            } => assert!(json),
            _ => panic!("Service Startとしてパースされるべき"),
        }
    }

    #[test]
    fn cli_service_stop_json_flag_defaults_to_false_and_can_be_set() {
        let default = Cli::try_parse_from(["sahai", "service", "stop", "myapp"]).unwrap();
        match default.command {
            Command::Service {
                action: ServiceAction::Stop { json, .. },
            } => assert!(!json),
            _ => panic!("Service Stopとしてパースされるべき"),
        }

        let with_json =
            Cli::try_parse_from(["sahai", "service", "stop", "myapp", "--json"]).unwrap();
        match with_json.command {
            Command::Service {
                action: ServiceAction::Stop { json, .. },
            } => assert!(json),
            _ => panic!("Service Stopとしてパースされるべき"),
        }
    }

    #[test]
    fn cli_service_restart_json_flag_defaults_to_false_and_can_be_set() {
        let default = Cli::try_parse_from(["sahai", "service", "restart", "myapp"]).unwrap();
        match default.command {
            Command::Service {
                action: ServiceAction::Restart { json, .. },
            } => assert!(!json),
            _ => panic!("Service Restartとしてパースされるべき"),
        }

        let with_json =
            Cli::try_parse_from(["sahai", "service", "restart", "myapp", "--json"]).unwrap();
        match with_json.command {
            Command::Service {
                action: ServiceAction::Restart { json, .. },
            } => assert!(json),
            _ => panic!("Service Restartとしてパースされるべき"),
        }
    }

    #[test]
    fn cli_service_status_json_flag_defaults_to_false_and_can_be_set() {
        let default = Cli::try_parse_from(["sahai", "service", "status", "myapp"]).unwrap();
        match default.command {
            Command::Service {
                action: ServiceAction::Status { json, .. },
            } => assert!(!json),
            _ => panic!("Service Statusとしてパースされるべき"),
        }

        let with_json =
            Cli::try_parse_from(["sahai", "service", "status", "myapp", "--json"]).unwrap();
        match with_json.command {
            Command::Service {
                action: ServiceAction::Status { json, .. },
            } => assert!(json),
            _ => panic!("Service Statusとしてパースされるべき"),
        }
    }

    #[test]
    fn cli_service_list_json_flag_defaults_to_false() {
        let cli = Cli::try_parse_from(["sahai", "service", "list"]).unwrap();
        match cli.command {
            Command::Service {
                action: ServiceAction::List { json },
            } => assert!(!json),
            _ => panic!("Service Listとしてパースされるべき"),
        }
    }

    #[test]
    fn cli_service_list_accepts_json_flag() {
        let cli = Cli::try_parse_from(["sahai", "service", "list", "--json"]).unwrap();
        match cli.command {
            Command::Service {
                action: ServiceAction::List { json },
            } => assert!(json),
            _ => panic!("Service Listとしてパースされるべき"),
        }
    }

    #[test]
    fn cli_parses_login_and_config_with_no_args() {
        assert!(matches!(
            Cli::try_parse_from(["sahai", "login"]).unwrap().command,
            Command::Login
        ));
        assert!(matches!(
            Cli::try_parse_from(["sahai", "config"]).unwrap().command,
            Command::Config
        ));
    }

    #[test]
    fn cli_rejects_unknown_subcommand() {
        assert!(Cli::try_parse_from(["sahai", "bogus"]).is_err());
    }

    // register push/register createというコマンド体系は存在せず、互換エイリアスも
    // 無い。誤って復活させていないことを確認する回帰テスト
    #[test]
    fn cli_rejects_removed_register_subcommand() {
        assert!(Cli::try_parse_from(["sahai", "register", "push", "myapp"]).is_err());
        assert!(Cli::try_parse_from(["sahai", "register", "create", "myapp"]).is_err());
    }

    #[test]
    fn cli_insecure_flag_defaults_to_false() {
        let cli = Cli::try_parse_from(["sahai", "service", "list"]).unwrap();
        assert!(!cli.insecure);
    }

    // TLS証明書検証をスキップする--insecureはサブコマンドの前後どちらでも
    // 指定できる(clapのglobal属性による)。SAHAI_DOMAIN=localhost等の
    // ローカルテスト環境で自己署名証明書を許容するために使う
    #[test]
    fn cli_insecure_flag_can_be_set_before_or_after_subcommand() {
        let before = Cli::try_parse_from(["sahai", "--insecure", "service", "list"]).unwrap();
        assert!(before.insecure);

        let after = Cli::try_parse_from(["sahai", "service", "list", "--insecure"]).unwrap();
        assert!(after.insecure);
    }
}
