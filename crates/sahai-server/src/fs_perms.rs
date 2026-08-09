//! 秘匿値を含むファイル・ディレクトリのパーミッションを絞るユーティリティ。
//! 平文で保存する以上、OSのパーミッションが唯一の防御線になる。
//!
//! unix以外には対応するパーミッションモデルがないためno-op(本番環境はLinuxホストのみを想定)。

use std::path::Path;

/// 所有者のみ読み書き可能(0600)にする。書き出しのたびに呼ぶ想定で、既存ファイルの
/// パーミッションも締め直す。
#[cfg(unix)]
pub async fn secure_file(path: &Path) -> std::io::Result<()> {
    set_mode(path, 0o600).await
}

#[cfg(not(unix))]
pub async fn secure_file(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

/// 所有者のみアクセス可能(0700)にする。ファイル本体が読めない場合でも、
/// サービス名・ID・ボリューム構成がディレクトリ列挙から漏れることを防ぐ。
#[cfg(unix)]
pub async fn secure_dir(path: &Path) -> std::io::Result<()> {
    set_mode(path, 0o700).await
}

#[cfg(not(unix))]
pub async fn secure_dir(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

#[cfg(unix)]
async fn set_mode(path: &Path, mode: u32) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let metadata = tokio::fs::metadata(path).await?;
    let mut perms = metadata.permissions();
    perms.set_mode(mode);
    tokio::fs::set_permissions(path, perms).await
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn temp_path(label: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "sahai_fs_perms_test_{label}_{:x}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[tokio::test]
    async fn secure_file_tightens_existing_permissive_file() {
        let path = temp_path("file");
        tokio::fs::write(&path, "SECRET=1\n").await.unwrap();
        tokio::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644))
            .await
            .unwrap();

        secure_file(&path).await.unwrap();

        let mode = tokio::fs::metadata(&path)
            .await
            .unwrap()
            .permissions()
            .mode();
        let _ = tokio::fs::remove_file(&path).await;
        assert_eq!(mode & 0o777, 0o600);
    }

    #[tokio::test]
    async fn secure_dir_tightens_existing_permissive_dir() {
        let path = temp_path("dir");
        tokio::fs::create_dir_all(&path).await.unwrap();
        tokio::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
            .await
            .unwrap();

        secure_dir(&path).await.unwrap();

        let mode = tokio::fs::metadata(&path)
            .await
            .unwrap()
            .permissions()
            .mode();
        let _ = tokio::fs::remove_dir_all(&path).await;
        assert_eq!(mode & 0o777, 0o700);
    }
}
