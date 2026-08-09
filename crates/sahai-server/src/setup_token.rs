//! 初回セットアップ用のワンタイムトークン。
//!
//! `POST /api/setup`はトークンがまだ存在しない段階で叩く必要があるため認証層の外側にあり、
//! そのままでは初期設定が完了するまでの間、ネットワーク越しに誰でも初期設定を先取りできる。
//! これを防ぐため、起動時にデータルートへトークンを書き出し、提示を必須にする。
//!
//! ファイルを唯一の正とする(メモリに持たない)。攻撃者はHTTPS越しにしか到達できず
//! このファイルを読めない一方、セットアップスクリプトは`docker compose exec`で
//! コンテナ内部からAPIを叩いており、同じ経路でファイルも読めるため運用上支障がない。

use std::path::{Path, PathBuf};

use subtle::ConstantTimeEq;
use uuid::Uuid;

/// リクエストヘッダー名。
pub const HEADER: &str = "X-Sahai-Setup-Token";

fn token_path(data_root: &Path) -> PathBuf {
    data_root.join("setup-token")
}

/// トークンを生成してデータルート直下へ書き出す。既存ファイルは上書きする
/// (未設定のまま再起動した場合は作り直す)。
pub async fn issue(data_root: &Path) -> std::io::Result<String> {
    // 既存依存のuuid v4(getrandom由来)を2つ連結し、64桁の16進とする。
    // 乱数生成のためだけに`rand`を追加せずに十分な強度を得る
    let token = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());

    tokio::fs::create_dir_all(data_root).await?;
    let path = token_path(data_root);
    tokio::fs::write(&path, &token).await?;
    crate::fs_perms::secure_file(&path).await?;
    Ok(token)
}

/// 提示されたトークンが発行済みのものと一致するか。
/// ファイルが無い(発行前・失効後)場合と空トークンは常に不一致とする。
pub async fn verify(data_root: &Path, presented: &str) -> bool {
    if presented.is_empty() {
        return false;
    }
    let Ok(expected) = tokio::fs::read_to_string(token_path(data_root)).await else {
        return false;
    };
    let expected = expected.trim();
    if expected.is_empty() {
        return false;
    }
    presented.as_bytes().ct_eq(expected.as_bytes()).into()
}

/// トークンを失効させる(初期設定の完了時、および設定済みでの起動時に呼ぶ)。
pub async fn revoke(data_root: &Path) -> std::io::Result<()> {
    match tokio::fs::remove_file(token_path(data_root)).await {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "sahai_setup_token_{label}_{:x}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[tokio::test]
    async fn issued_token_verifies() {
        let root = temp_root("ok");
        let token = issue(&root).await.unwrap();

        assert!(verify(&root, &token).await);

        let _ = tokio::fs::remove_dir_all(&root).await;
    }

    #[tokio::test]
    async fn wrong_token_is_rejected() {
        let root = temp_root("wrong");
        issue(&root).await.unwrap();

        assert!(!verify(&root, "not-the-token").await);

        let _ = tokio::fs::remove_dir_all(&root).await;
    }

    /// 発行前(ファイルが無い)は何を出しても通らない。
    #[tokio::test]
    async fn verify_fails_when_not_issued() {
        let root = temp_root("missing");
        assert!(!verify(&root, "anything").await);
    }

    #[tokio::test]
    async fn empty_token_is_rejected() {
        let root = temp_root("empty");
        issue(&root).await.unwrap();

        assert!(!verify(&root, "").await);

        let _ = tokio::fs::remove_dir_all(&root).await;
    }

    #[tokio::test]
    async fn revoked_token_no_longer_verifies() {
        let root = temp_root("revoke");
        let token = issue(&root).await.unwrap();
        assert!(verify(&root, &token).await);

        revoke(&root).await.unwrap();

        assert!(!verify(&root, &token).await);

        let _ = tokio::fs::remove_dir_all(&root).await;
    }

    /// 失効済み・未発行のディレクトリに対してrevokeしてもエラーにしない。
    #[tokio::test]
    async fn revoke_is_idempotent() {
        let root = temp_root("revoke_twice");
        issue(&root).await.unwrap();
        revoke(&root).await.unwrap();

        assert!(revoke(&root).await.is_ok());

        let _ = tokio::fs::remove_dir_all(&root).await;
    }

    /// 未設定のまま再起動した場合、前のトークンは無効になる。
    #[tokio::test]
    async fn reissue_invalidates_previous_token() {
        let root = temp_root("reissue");
        let first = issue(&root).await.unwrap();
        let second = issue(&root).await.unwrap();

        assert_ne!(first, second);
        assert!(!verify(&root, &first).await);
        assert!(verify(&root, &second).await);

        let _ = tokio::fs::remove_dir_all(&root).await;
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn issued_file_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;

        let root = temp_root("perms");
        issue(&root).await.unwrap();

        let mode = tokio::fs::metadata(token_path(&root))
            .await
            .unwrap()
            .permissions()
            .mode();

        let _ = tokio::fs::remove_dir_all(&root).await;
        assert_eq!(mode & 0o777, 0o600);
    }
}
