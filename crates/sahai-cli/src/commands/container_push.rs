//! `sahai container push <name>`: ビルド+レジストリpush。

use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::Value;

use crate::api_client::ApiClient;

pub struct PushArgs {
    pub name: String,
    pub context: PathBuf,
    /// 使用するcomposeファイル。省略時は既定の名前から探す
    pub compose_file: Option<String>,
    pub build_args: Vec<(String, String)>,
    pub platform: Option<String>,
    pub deploy: bool,
}

#[derive(Deserialize)]
struct ServiceSummary {
    source_type: String,
}

pub async fn run(client: &ApiClient, registry_url: &str, args: PushArgs) -> Result<(), String> {
    // 1. 存在確認
    let existing: Result<ServiceSummary, String> =
        client.get(&format!("/api/services/{}", args.name)).await;
    let service = existing.map_err(|e| describe_lookup_error(&args.name, &e))?;

    // 2. compose_content配下の存在確認とsource_typeとの照合
    let compose_path =
        sahai_core::compose::resolve_compose_file(&args.context, args.compose_file.as_deref())
            .map_err(|e| e.to_string())?;
    let is_compose_dir = compose_path.is_some();
    let is_compose_registered = service.source_type == "compose";
    if is_compose_dir != is_compose_registered {
        return Err(format!(
            "登録内容(source_type={})とディレクトリ構成が一致しません",
            service.source_type
        ));
    }

    match compose_path {
        Some(path) => push_compose(&args, registry_url, &path).await?,
        None => push_image(&args, registry_url).await?,
    }

    // 5. --deploy指定時のみrestartを呼ぶ
    if args.deploy {
        let _: Value = client
            .post_empty(&format!("/api/services/{}/restart", args.name))
            .await?;
        println!("再デプロイしました。");
    }

    Ok(())
}

/// サービス存在確認(`GET /api/services/{name}`)が失敗した際のメッセージを組み立てる。
/// `ApiClient::get`のエラー文字列はHTTP応答エラー(`"HTTP {status}: ..."`)と
/// 通信自体の失敗(reqwestのエラー文字列。TLS証明書検証エラー等を含む)の
/// どちらもあり得るが、以前はどちらも一律「未登録」扱いにしていたため、
/// 実際には登録済みでも通信できないだけの状況が「未登録」と誤解される原因になっていた。
/// HTTP 404のときだけ「未登録」の案内を出し、それ以外は実際のエラーをそのまま見せる。
fn describe_lookup_error(name: &str, err: &str) -> String {
    if err.starts_with("HTTP 404") {
        format!("サービス '{name}' が見つかりません。先にWeb UIで登録してください")
    } else {
        format!("サービス '{name}' の情報取得に失敗しました: {err}")
    }
}

async fn push_image(args: &PushArgs, registry_url: &str) -> Result<(), String> {
    sahai_core::validation::validate_service_name(&args.name).map_err(|e| e.to_string())?;
    let tag = format!("{registry_url}/{}:latest", args.name);
    docker_build(
        &args.context,
        &tag,
        &args.build_args,
        args.platform.as_deref(),
        None,
    )?;
    docker_push(&tag)?;
    Ok(())
}

async fn push_compose(
    args: &PushArgs,
    registry_url: &str,
    compose_path: &Path,
) -> Result<(), String> {
    let content = std::fs::read_to_string(compose_path).map_err(|e| e.to_string())?;
    let build_specs =
        sahai_core::compose::parse_build_specs(&content).map_err(|e| e.to_string())?;

    for (compose_service_name, spec) in &build_specs {
        sahai_core::validation::validate_compose_service_name(compose_service_name)
            .map_err(|e| format!("'{compose_service_name}': {e}"))?;
        let tag_name =
            sahai_core::naming::registry_tag_name(&args.name, Some(compose_service_name));
        sahai_core::validation::validate_registry_tag_length(&tag_name)
            .map_err(|e| format!("'{compose_service_name}': {e}"))?;

        // サービスごとにbuild.contextが異なりうる(フロントエンド/バックエンドを
        // 別ディレクトリでビルドする構成等)ため、CLI起動時のcontextをそのまま
        // 使い回さず、各サービスのcontextをcompose_content基準で解決する
        // (以前は全サービスを一律CLIのcontextでビルドしており、context違いの
        // サービスが誤った内容でビルドされていた)。
        let service_context = args.context.join(&spec.context);
        let tag = format!("{registry_url}/{tag_name}:latest");
        docker_build(
            &service_context,
            &tag,
            &args.build_args,
            args.platform.as_deref(),
            spec.dockerfile.as_deref(),
        )?;
        docker_push(&tag)?;
    }
    Ok(())
}

fn docker_build(
    context: &Path,
    tag: &str,
    build_args: &[(String, String)],
    platform: Option<&str>,
    dockerfile: Option<&str>,
) -> Result<(), String> {
    let mut cmd = std::process::Command::new("docker");
    cmd.args(sahai_core::docker_args::build_args(
        context, tag, dockerfile, platform, build_args,
    ));
    run_and_check(cmd)
}

fn docker_push_args(tag: &str) -> Vec<String> {
    vec!["push".to_string(), tag.to_string()]
}

fn docker_push(tag: &str) -> Result<(), String> {
    let mut cmd = std::process::Command::new("docker");
    cmd.args(docker_push_args(tag));
    run_and_check(cmd)
}

fn run_and_check(mut cmd: std::process::Command) -> Result<(), String> {
    let status = cmd.status().map_err(|e| e.to_string())?;
    if !status.success() {
        return Err(format!("コマンドが失敗しました(終了コード: {status})"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn describe_lookup_error_gives_registration_hint_for_http_404() {
        assert_eq!(
            describe_lookup_error(
                "myapp",
                "HTTP 404 Not Found: {\"error\":{\"message\":\"not found\"}}"
            ),
            "サービス 'myapp' が見つかりません。先にWeb UIで登録してください"
        );
    }

    // 404以外(ネットワーク・TLS証明書・認証エラー等)は「未登録」という誤った断定を
    // せず、実際のエラー内容をそのまま利用者に見せる。以前は全エラーを一律
    // 「未登録」扱いにしていたため、実際には登録済みなのにTLS証明書検証エラー
    // (自己署名証明書のドメインでreqwestのデフォルト検証が失敗するケース等)で
    // 通信できないだけの状況が「未登録」と誤解される原因になっていた
    #[test]
    fn describe_lookup_error_passes_through_non_404_errors() {
        let msg = describe_lookup_error(
            "myapp",
            "error sending request for url (https://sahai.localhost/api/services/myapp): error trying to connect: invalid peer certificate",
        );
        assert!(
            msg.contains("invalid peer certificate"),
            "実際のエラー内容が含まれるべき: {msg}"
        );
        assert!(
            !msg.contains("先にWeb UIで登録してください"),
            "未登録だと誤断定してはいけない: {msg}"
        );
    }

    #[test]
    fn docker_push_args_is_push_and_tag() {
        assert_eq!(
            docker_push_args("registry.sahai.example.com/myapp:latest"),
            vec!["push", "registry.sahai.example.com/myapp:latest"]
        );
    }
}
