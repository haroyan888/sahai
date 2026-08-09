//! `docker login`のサブプロセス実行。起動時(main.rs)とWeb UI保存時
//! (service::settings::update_registry_config)の両方から呼ばれる共通処理。

use tokio::io::AsyncWriteExt;
use tokio::process::Command;

/// `docker login <registry_url> -u <username> --password-stdin` をサブプロセス実行する。
/// パスワードをコマンドライン引数に含めない(プロセス一覧から読めてしまうため)ように
/// 標準入力経由で渡す。
pub async fn login(registry_url: &str, username: &str, password: &str) -> Result<(), String> {
    let mut child = Command::new("docker")
        .args(["login", registry_url, "-u", username, "--password-stdin"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(password.as_bytes())
            .await
            .map_err(|e| e.to_string())?;
    }

    let output = child.wait_with_output().await.map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).to_string());
    }
    Ok(())
}
