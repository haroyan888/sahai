//! `docker`サブコマンドに渡す引数列の組み立て。
//!
//! ビルドはCLI(利用者のマシン)とサーバー(`sahai service create`の代行ビルド)の
//! 両方で行うため、引数の並びがずれると片方だけ挙動が変わる。純粋関数として
//! ここに置き、サブプロセス実行はそれぞれの呼び出し側が持つ。

use std::path::Path;

/// `docker build`の引数列を組み立てる。
/// `dockerfile`はcompose定義の`build.dockerfile`と同じくcontext基準の相対パス。
pub fn build_args(
    context: &Path,
    tag: &str,
    dockerfile: Option<&str>,
    platform: Option<&str>,
    build_args: &[(String, String)],
) -> Vec<String> {
    let mut args = vec!["build".to_string(), "-t".to_string(), tag.to_string()];
    for (k, v) in build_args {
        args.push("--build-arg".to_string());
        args.push(format!("{k}={v}"));
    }
    if let Some(p) = platform {
        args.push("--platform".to_string());
        args.push(p.to_string());
    }
    if let Some(f) = dockerfile {
        args.push("-f".to_string());
        args.push(context.join(f).display().to_string());
    }
    args.push(context.display().to_string());
    args
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_argsとplatformを並べる() {
        let args = build_args(
            Path::new("./ctx"),
            "registry.sahai.example.com/myapp:latest",
            None,
            Some("linux/amd64"),
            &[("FOO".to_string(), "bar".to_string())],
        );
        assert_eq!(
            args,
            vec![
                "build".to_string(),
                "-t".to_string(),
                "registry.sahai.example.com/myapp:latest".to_string(),
                "--build-arg".to_string(),
                "FOO=bar".to_string(),
                "--platform".to_string(),
                "linux/amd64".to_string(),
                "./ctx".to_string(),
            ]
        );
    }

    #[test]
    fn dockerfileはcontext基準で解決する() {
        let args = build_args(
            Path::new("./frontend"),
            "registry.sahai.example.com/myapp-frontend:latest",
            Some("Dockerfile.prod"),
            None,
            &[],
        );
        assert_eq!(
            args,
            vec![
                "build".to_string(),
                "-t".to_string(),
                "registry.sahai.example.com/myapp-frontend:latest".to_string(),
                "-f".to_string(),
                Path::new("./frontend")
                    .join("Dockerfile.prod")
                    .display()
                    .to_string(),
                "./frontend".to_string(),
            ]
        );
    }

    #[test]
    fn 未指定のplatformとdockerfileは省略される() {
        let args = build_args(Path::new("."), "x:latest", None, None, &[]);
        assert_eq!(args, vec!["build", "-t", "x:latest", "."]);
    }
}
