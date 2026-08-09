//! プロジェクトディレクトリをtar.gz化する共通ロジック。`service create`・`service update`
//! で共有する(いずれもサーバー側でのビルド用にプロジェクト一式をアップロードする)。

use std::path::Path;

/// `context`配下をtar.gz化する。`.dockerignore`(無ければ`ignore`クレートの既定である
/// `.gitignore`・隠しファイル除外〈`.git`ディレクトリを含む〉)を尊重し、`.git`や
/// `node_modules`等を丸ごと送りつけないようにする(実docker CLIが自動でcontextを
/// `.dockerignore`ベースに絞り込む挙動に近づける)。
pub fn build_archive(context: &Path) -> Result<Vec<u8>, String> {
    let encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    let mut builder = tar::Builder::new(encoder);

    let walker = ignore::WalkBuilder::new(context)
        .add_custom_ignore_filename(".dockerignore")
        .build();

    for entry in walker {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        if path == context {
            continue;
        }
        let file_type = entry
            .file_type()
            .ok_or_else(|| format!("ファイル種別を判定できません: {}", path.display()))?;
        if !file_type.is_file() {
            continue;
        }
        let relative = path.strip_prefix(context).map_err(|e| e.to_string())?;
        builder
            .append_path_with_name(path, relative)
            .map_err(|e| e.to_string())?;
    }

    let encoder = builder.into_inner().map_err(|e| e.to_string())?;
    encoder.finish().map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn temp_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "sahai_cli_archive_test_{label}_{:x}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn build_archive_includes_regular_files() {
        let dir = temp_dir("ok");
        std::fs::write(dir.join("Dockerfile"), "FROM scratch\n").unwrap();

        let bytes = build_archive(&dir).unwrap();

        let decoder = flate2::read::GzDecoder::new(bytes.as_slice());
        let mut archive = tar::Archive::new(decoder);
        let names: Vec<String> = archive
            .entries()
            .unwrap()
            .map(|e| e.unwrap().path().unwrap().display().to_string())
            .collect();
        assert!(names.iter().any(|n| n.contains("Dockerfile")), "{names:?}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn build_archive_excludes_git_directory() {
        let dir = temp_dir("git");
        std::fs::create_dir_all(dir.join(".git")).unwrap();
        std::fs::write(dir.join(".git").join("HEAD"), "ref: refs/heads/main\n").unwrap();
        std::fs::write(dir.join("Dockerfile"), "FROM scratch\n").unwrap();

        let bytes = build_archive(&dir).unwrap();

        let decoder = flate2::read::GzDecoder::new(bytes.as_slice());
        let mut archive = tar::Archive::new(decoder);
        let names: Vec<String> = archive
            .entries()
            .unwrap()
            .map(|e| e.unwrap().path().unwrap().display().to_string())
            .collect();
        assert!(!names.iter().any(|n| n.contains(".git")), "{names:?}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn build_archive_excludes_files_matching_dockerignore() {
        let dir = temp_dir("dockerignore");
        std::fs::write(dir.join(".dockerignore"), "secret.txt\n").unwrap();
        std::fs::write(dir.join("secret.txt"), "shh").unwrap();
        std::fs::write(dir.join("Dockerfile"), "FROM scratch\n").unwrap();

        let bytes = build_archive(&dir).unwrap();

        let decoder = flate2::read::GzDecoder::new(bytes.as_slice());
        let mut archive = tar::Archive::new(decoder);
        let names: Vec<String> = archive
            .entries()
            .unwrap()
            .map(|e| e.unwrap().path().unwrap().display().to_string())
            .collect();
        assert!(names.iter().any(|n| n.contains("Dockerfile")), "{names:?}");
        assert!(!names.iter().any(|n| n.contains("secret.txt")), "{names:?}");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
