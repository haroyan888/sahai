//! `.sahai.env`ファイル(sahai専用の内部ブリッジファイル、`SAHAI_DATA_ROOT`直下)の
//! 特定キーだけを書き換える/追記するユーティリティ。既存の他の行(コメント・
//! 無関係なキー)はそのまま保持する(DNSプロバイダ認証情報の書き込み時に、手動で
//! 書いた他の設定を消さないため)。
//!
//! ファイル・親ディレクトリが存在しない場合は自動作成する。`SAHAI_DATA_ROOT`直下に
//! 置くのは、Windows(Docker Desktop)ではホスト同一パスマウントが原理的に成立せず、
//! それ以外の場所だとsahai-server内部の書き込み先とTraefikが実際に読む実ファイルが
//! 乖離するため(`config.rs::env_file_path`参照)。

use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum EnvFileError {
    #[error(
        ".sahai.envのパスがファイルではありません(ディレクトリになっている可能性があります): {0}"
    )]
    NotAFile(String),
    #[error(".sahai.envの読み書きに失敗しました: {0}")]
    Io(#[from] std::io::Error),
}

/// `updates`で渡されたキーが既存の`.sahai.env`内にあれば値を置き換え、無ければ末尾に
/// 追記する。他の行は一切変更しない。ファイル・親ディレクトリが存在しない場合は
/// 新規作成する。DNSプロバイダの認証情報という秘匿値を含むため、書き込みのたびに
/// 0600を適用し直す。
pub async fn upsert(path: &Path, updates: &[(&str, &str)]) -> Result<(), EnvFileError> {
    let (content, file_existed) = match tokio::fs::metadata(path).await {
        Ok(metadata) if metadata.is_file() => (tokio::fs::read_to_string(path).await?, true),
        Ok(_not_a_file) => return Err(EnvFileError::NotAFile(path.display().to_string())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (String::new(), false),
        Err(e) => return Err(e.into()),
    };

    if !file_existed {
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
    }

    let new_content = upsert_content(&content, updates);
    tokio::fs::write(path, new_content).await?;
    crate::fs_perms::secure_file(path).await?;
    Ok(())
}

/// `.sahai.env`をキー・値のペアの一覧として読み込む。コメント行(`#`始まり)・空行は
/// 無視する。ファイルが存在しない場合は空のVecを返す(DNS設定が一度も保存されて
/// いない初回起動直後を許容するため)。Traefikコンテナ再作成時にEnvを構築するために使う
/// (`traefik::container`参照)。
pub async fn load(path: &Path) -> Result<Vec<(String, String)>, EnvFileError> {
    let content = match tokio::fs::read_to_string(path).await {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e.into()),
    };
    Ok(parse_content(&content))
}

fn parse_content(content: &str) -> Vec<(String, String)> {
    content
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                return None;
            }
            trimmed
                .split_once('=')
                .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        })
        .collect()
}

/// ファイルI/Oを含まない純粋なテキスト変換部分(テスト容易化のため分離)。
fn upsert_content(content: &str, updates: &[(&str, &str)]) -> String {
    let mut remaining: Vec<(&str, &str)> = updates.to_vec();
    let mut lines: Vec<String> = content.lines().map(|l| l.to_string()).collect();

    for line in lines.iter_mut() {
        let trimmed = line.trim_start();
        if trimmed.starts_with('#') || trimmed.is_empty() {
            continue;
        }
        if let Some(eq_pos) = line.find('=') {
            let key = line[..eq_pos].trim();
            if let Some(pos) = remaining.iter().position(|(k, _)| *k == key) {
                let (_, value) = remaining.remove(pos);
                *line = format!("{key}={value}");
            }
        }
    }

    for (key, value) in remaining {
        lines.push(format!("{key}={value}"));
    }

    let mut result = lines.join("\n");
    result.push('\n');
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replaces_existing_key_in_place() {
        let content = "SAHAI_DNS_PROVIDER=cloudflare\nSAHAI_ACME_EMAIL=old@example.com\n";
        let result = upsert_content(content, &[("SAHAI_ACME_EMAIL", "new@example.com")]);
        assert_eq!(
            result,
            "SAHAI_DNS_PROVIDER=cloudflare\nSAHAI_ACME_EMAIL=new@example.com\n"
        );
    }

    #[test]
    fn appends_missing_key_at_end() {
        let content = "SAHAI_DNS_PROVIDER=cloudflare\n";
        let result = upsert_content(content, &[("CF_DNS_API_TOKEN", "secret123")]);
        assert_eq!(
            result,
            "SAHAI_DNS_PROVIDER=cloudflare\nCF_DNS_API_TOKEN=secret123\n"
        );
    }

    #[test]
    fn preserves_unrelated_lines_and_comments() {
        let content = "# コメント\nSAHAI_API_TOKEN=keep-me\n\nSAHAI_DNS_PROVIDER=cloudflare\n";
        let result = upsert_content(content, &[("SAHAI_DNS_PROVIDER", "route53")]);
        assert_eq!(
            result,
            "# コメント\nSAHAI_API_TOKEN=keep-me\n\nSAHAI_DNS_PROVIDER=route53\n"
        );
    }

    #[test]
    fn handles_empty_input_by_appending_all_keys() {
        let result = upsert_content(
            "",
            &[
                ("SAHAI_DNS_PROVIDER", "cloudflare"),
                ("SAHAI_ACME_EMAIL", "a@b.com"),
            ],
        );
        assert_eq!(
            result,
            "SAHAI_DNS_PROVIDER=cloudflare\nSAHAI_ACME_EMAIL=a@b.com\n"
        );
    }

    #[test]
    fn replaces_multiple_keys_in_one_call() {
        let content = "SAHAI_DNS_PROVIDER=cloudflare\nSAHAI_ACME_EMAIL=old@example.com\nCF_DNS_API_TOKEN=old-token\n";
        let result = upsert_content(
            content,
            &[
                ("SAHAI_DNS_PROVIDER", "route53"),
                ("SAHAI_ACME_EMAIL", "new@example.com"),
                ("AWS_ACCESS_KEY_ID", "AKIA..."),
            ],
        );
        assert_eq!(
            result,
            "SAHAI_DNS_PROVIDER=route53\nSAHAI_ACME_EMAIL=new@example.com\nCF_DNS_API_TOKEN=old-token\nAWS_ACCESS_KEY_ID=AKIA...\n"
        );
    }

    fn temp_path(label: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "sahai_env_file_test_{label}_{:x}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[tokio::test]
    async fn upsert_writes_changes_to_real_file() {
        let path = temp_path("write");
        tokio::fs::write(&path, "SAHAI_DNS_PROVIDER=cloudflare\n")
            .await
            .unwrap();

        upsert(&path, &[("SAHAI_ACME_EMAIL", "a@b.com")])
            .await
            .unwrap();

        let content = tokio::fs::read_to_string(&path).await.unwrap();
        assert_eq!(
            content,
            "SAHAI_DNS_PROVIDER=cloudflare\nSAHAI_ACME_EMAIL=a@b.com\n"
        );

        let _ = tokio::fs::remove_file(&path).await;
    }

    #[tokio::test]
    async fn upsert_creates_file_and_parent_dir_when_missing() {
        // sahai_data/サブディレクトリ自体も事前に作らない(初回チェックアウト直後を模す)。
        let base = temp_path("create_parent");
        let path = base.join("sahai_data").join(".sahai.env");

        let result = upsert(&path, &[("SAHAI_DNS_PROVIDER", "cloudflare")]).await;

        assert!(result.is_ok());
        let content = tokio::fs::read_to_string(&path).await.unwrap();
        assert_eq!(content, "SAHAI_DNS_PROVIDER=cloudflare\n");

        let _ = tokio::fs::remove_dir_all(&base).await;
    }

    #[tokio::test]
    async fn upsert_errors_when_path_is_a_directory() {
        let path = temp_path("dir");
        tokio::fs::create_dir_all(&path).await.unwrap();

        let result = upsert(&path, &[("SAHAI_ACME_EMAIL", "a@b.com")]).await;

        let _ = tokio::fs::remove_dir_all(&path).await;
        assert!(matches!(result, Err(EnvFileError::NotAFile(_))));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn upsert_sets_owner_only_permissions_when_creating_new_file() {
        use std::os::unix::fs::PermissionsExt;

        let path = temp_path("new_perms");
        upsert(&path, &[("SAHAI_DNS_PROVIDER", "cloudflare")])
            .await
            .unwrap();

        let metadata = tokio::fs::metadata(&path).await.unwrap();
        assert_eq!(metadata.permissions().mode() & 0o777, 0o600);

        let _ = tokio::fs::remove_file(&path).await;
    }

    #[test]
    fn parse_content_ignores_comments_and_blank_lines() {
        let content = "# コメント\n\nSAHAI_DNS_PROVIDER=cloudflare\nCF_DNS_API_TOKEN=secret\n";
        let result = parse_content(content);
        assert_eq!(
            result,
            vec![
                ("SAHAI_DNS_PROVIDER".to_string(), "cloudflare".to_string()),
                ("CF_DNS_API_TOKEN".to_string(), "secret".to_string()),
            ]
        );
    }

    #[test]
    fn parse_content_trims_whitespace_around_key_and_value() {
        let content = "  SAHAI_ACME_EMAIL = a@b.com  \n";
        let result = parse_content(content);
        assert_eq!(
            result,
            vec![("SAHAI_ACME_EMAIL".to_string(), "a@b.com".to_string())]
        );
    }

    #[tokio::test]
    async fn load_returns_empty_vec_when_file_does_not_exist() {
        let path = temp_path("load_missing");
        let result = load(&path).await.unwrap();
        assert_eq!(result, Vec::new());
    }

    #[tokio::test]
    async fn load_reads_existing_file_content() {
        let path = temp_path("load_existing");
        tokio::fs::write(
            &path,
            "SAHAI_DNS_PROVIDER=cloudflare\nCF_DNS_API_TOKEN=secret\n",
        )
        .await
        .unwrap();

        let result = load(&path).await.unwrap();

        let _ = tokio::fs::remove_file(&path).await;
        assert_eq!(
            result,
            vec![
                ("SAHAI_DNS_PROVIDER".to_string(), "cloudflare".to_string()),
                ("CF_DNS_API_TOKEN".to_string(), "secret".to_string()),
            ]
        );
    }

    /// 緩いパーミッションで残っている既存ファイルも書き込みのたびに締め直す。
    #[cfg(unix)]
    #[tokio::test]
    async fn upsert_tightens_permissions_of_preexisting_file() {
        use std::os::unix::fs::PermissionsExt;

        let path = temp_path("existing_perms");
        tokio::fs::write(&path, "SAHAI_DNS_PROVIDER=cloudflare\n")
            .await
            .unwrap();
        tokio::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644))
            .await
            .unwrap();

        upsert(&path, &[("SAHAI_ACME_EMAIL", "a@b.com")])
            .await
            .unwrap();

        let metadata = tokio::fs::metadata(&path).await.unwrap();
        assert_eq!(metadata.permissions().mode() & 0o777, 0o600);

        let _ = tokio::fs::remove_file(&path).await;
    }
}
