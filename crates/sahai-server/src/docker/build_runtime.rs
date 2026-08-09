//! `docker build`/`docker push`をサブプロセス実行する(サービスアップロードによる新規登録)。
//! `compose_runtime.rs`の`run_compose`と同じパターン(`tokio::process::Command`+`.output()`+
//! ステータスチェック)を踏襲する。bollardはcompose型の複数サービス一括ビルドを提供しないため、
//! image/composeで非対称な実装にしないよう、ここでもサブプロセス方式に統一する
//! (CLI側`sahai-cli::commands::register_push`の`docker build`/`docker push`呼び出しと
//! 挙動を一致させる狙いもある)。

use std::path::Path;

use tokio::process::Command;

use super::DockerError;

/// `docker build -t <tag> ... <context>` → `docker push <tag>` を実行する。
pub async fn build_and_push(
    context: &Path,
    tag: &str,
    dockerfile: Option<&str>,
    platform: Option<&str>,
    build_args: &[(String, String)],
) -> Result<(), DockerError> {
    run_docker(&sahai_core::docker_args::build_args(
        context, tag, dockerfile, platform, build_args,
    ))
    .await?;
    run_docker(&["push".to_string(), tag.to_string()]).await
}

async fn run_docker(args: &[String]) -> Result<(), DockerError> {
    let output = Command::new("docker")
        .args(args)
        .output()
        .await
        .map_err(|e| DockerError::BuildExec(e.to_string()))?;
    if !output.status.success() {
        return Err(DockerError::BuildExec(
            String::from_utf8_lossy(&output.stderr).to_string(),
        ));
    }
    Ok(())
}
