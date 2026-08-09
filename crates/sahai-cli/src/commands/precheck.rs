//! アップロード前のローカル検証。`service create`・`service update`で共有する。
//!
//! ここでの検証は早期失敗のためだけのもので、正としての検証はサーバー側で必ず
//! 再実行される。アーカイブを作って送ってから弾かれるのを避けるのが目的。

use std::path::Path;

/// `context`直下のcomposeファイルを読み、`build:`を持つ各サービスについて
/// サービス名の文字種とレジストリタグ長を検証する。composeファイルが無ければ
/// image型プロジェクトとみなし、何も検証せず成功を返す。
pub fn validate_compose_build_targets(service_name: &str, context: &Path) -> Result<(), String> {
    let Some(compose_path) = sahai_core::compose::find_compose_file(context) else {
        return Ok(());
    };
    let content = std::fs::read_to_string(&compose_path).map_err(|e| e.to_string())?;
    let build_specs =
        sahai_core::compose::parse_build_specs(&content).map_err(|e| e.to_string())?;

    for compose_service_name in build_specs.keys() {
        sahai_core::validation::validate_compose_service_name(compose_service_name)
            .map_err(|e| format!("'{compose_service_name}': {e}"))?;
        let tag_name =
            sahai_core::naming::registry_tag_name(service_name, Some(compose_service_name));
        sahai_core::validation::validate_registry_tag_length(&tag_name)
            .map_err(|e| format!("'{compose_service_name}': {e}"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// テスト間で衝突しない一時ディレクトリを作る(archive.rsと同じ方式。
    /// テスト用の依存クレートを増やさないため自前で用意している)。
    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "sahai-precheck-{tag}-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_compose(dir: &Path, body: &str) {
        std::fs::write(dir.join("compose.yaml"), body).unwrap();
    }

    #[test]
    fn composeファイルが無ければ成功する() {
        let dir = temp_dir("nocompose");
        assert!(validate_compose_build_targets("myapp", &dir).is_ok());
    }

    #[test]
    fn build指定の無いサービスは検証対象外() {
        let dir = temp_dir("nobuild");
        write_compose(&dir, "services:\n  DB_BAD:\n    image: postgres:16\n");
        assert!(validate_compose_build_targets("myapp", &dir).is_ok());
    }

    #[test]
    fn 不正なcomposeサービス名を弾く() {
        let dir = temp_dir("badname");
        write_compose(&dir, "services:\n  Web_App:\n    build: .\n");
        let err = validate_compose_build_targets("myapp", &dir).unwrap_err();
        assert!(err.contains("Web_App"), "{err}");
    }

    #[test]
    fn タグ長超過を弾く() {
        let dir = temp_dir("longtag");
        write_compose(&dir, "services:\n  web:\n    build: .\n");
        let long_name = "a".repeat(200);
        let err = validate_compose_build_targets(&long_name, &dir).unwrap_err();
        assert!(err.contains("web"), "{err}");
    }

    #[test]
    fn 正当なcompose定義は通る() {
        let dir = temp_dir("ok");
        write_compose(
            &dir,
            "services:\n  web:\n    build: .\n  db:\n    image: postgres:16\n",
        );
        assert!(validate_compose_build_targets("myapp", &dir).is_ok());
    }
}
