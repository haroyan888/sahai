//! docker-compose YAMLのパースと、ServiceContainerの新旧差分算出。
//! sahai-serverとsahai-cliの両方から使うため、I/Oを持たない純粋ロジックとして
//! このクレートに置く(片方だけ直すとタグ名がずれて事故になる)。

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::error::CoreError;

/// ディレクトリ直下のdocker-compose定義ファイルを探す。
/// CLI(`container push`/`service create`)とサーバー(アップロードによる新規登録)の
/// 両方がここを経由してimage型/compose型を判定することで、判定ロジックの重複を避ける。
pub fn find_compose_file(dir: &Path) -> Option<PathBuf> {
    for name in [
        "docker-compose.yml",
        "docker-compose.yaml",
        "compose.yml",
        "compose.yaml",
    ] {
        let path = dir.join(name);
        if path.exists() {
            return Some(path);
        }
    }
    None
}

#[derive(Debug, Deserialize)]
struct ComposeFile {
    #[serde(default)]
    services: BTreeMap<String, ComposeServiceDef>,
}

#[derive(Debug, Default, Deserialize)]
struct ComposeServiceDef {
    #[serde(default)]
    build: Option<BuildValue>,
}

/// composeの`build:`キーは短縮形(文字列でcontextを直接指定)と、
/// context/dockerfileを個別に指定するマッピング形式のどちらも取りうる。
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum BuildValue {
    Shorthand(String),
    Detailed {
        #[serde(default)]
        context: Option<String>,
        #[serde(default)]
        dockerfile: Option<String>,
    },
}

/// サービスごとのビルド設定。
/// `context`はcompose_contentのあるディレクトリからの相対パス。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildSpec {
    pub context: String,
    pub dockerfile: Option<String>,
}

/// compose_content内の全サービス名を取得する。
pub fn parse_service_names(compose_content: &str) -> Result<Vec<String>, CoreError> {
    let file: ComposeFile = serde_yaml::from_str(compose_content)
        .map_err(|e| CoreError::ComposeParse(e.to_string()))?;
    Ok(file.services.into_keys().collect())
}

/// `build:`キーを持つサービス名のみを取得する(既製イメージのサービスはビルド対象外)。
pub fn parse_build_service_names(compose_content: &str) -> Result<Vec<String>, CoreError> {
    let file: ComposeFile = serde_yaml::from_str(compose_content)
        .map_err(|e| CoreError::ComposeParse(e.to_string()))?;
    Ok(file
        .services
        .into_iter()
        .filter(|(_, def)| def.build.is_some())
        .map(|(name, _)| name)
        .collect())
}

/// `build:`キーを持つ各サービスの`context`/`dockerfile`を取得する。
/// サービスごとにビルドコンテキストが異なりうる(例: フロントエンド/バックエンドを
/// 別ディレクトリでビルドするcompose構成)ため、`sahai container push`はここで得た
/// サービスごとのcontextを使って個別に`docker build`する必要がある
/// (以前は全サービスをCLI起動時の単一contextで一律ビルドしており、
/// context違いのサービスが誤った内容でビルドされていた)。
/// `context`未指定時は`.`(compose_contentのあるディレクトリ)を既定値とする。
pub fn parse_build_specs(compose_content: &str) -> Result<BTreeMap<String, BuildSpec>, CoreError> {
    let file: ComposeFile = serde_yaml::from_str(compose_content)
        .map_err(|e| CoreError::ComposeParse(e.to_string()))?;
    Ok(file
        .services
        .into_iter()
        .filter_map(|(name, def)| {
            def.build.map(|build| {
                let spec = match build {
                    BuildValue::Shorthand(context) => BuildSpec {
                        context,
                        dockerfile: None,
                    },
                    BuildValue::Detailed {
                        context,
                        dockerfile,
                    } => BuildSpec {
                        context: context.unwrap_or_else(|| ".".to_string()),
                        dockerfile,
                    },
                };
                (name, spec)
            })
        })
        .collect())
}

/// 既存の`ServiceContainer`名集合と、新しい`compose_content`から得られたサービス名集合を
/// 突き合わせた結果。
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ContainerDiff {
    /// 新しくServiceContainerを作る必要があるサービス名
    pub added: Vec<String>,
    /// ServiceContainerを削除する必要があるサービス名
    pub removed: Vec<String>,
    /// 変更不要(ServiceContainer.idを維持する)サービス名
    pub kept: Vec<String>,
}

/// 使用するcomposeファイルを決める。
///
/// `explicit`が与えられればそれを使い、無ければ`find_compose_file`で既定の名前を探す。
/// `compose.prod.yaml`のような既定名以外のファイルや、1つのプロジェクトに複数の
/// composeファイルがある場合に明示できるようにするための関数。
///
/// 明示されたパスは`dir`配下に収まることを検証する。ルート外を指すとアップロードした
/// アーカイブに含まれておらず、サーバー側で必ず失敗するため、ここで弾く
/// (`build.context`に対する検証と同じ考え方)。
pub fn resolve_compose_file(
    dir: &Path,
    explicit: Option<&str>,
) -> Result<Option<PathBuf>, CoreError> {
    let Some(explicit) = explicit else {
        return Ok(find_compose_file(dir));
    };

    let joined = dir.join(explicit);
    if !joined.is_file() {
        return Err(CoreError::Validation(format!(
            "composeファイルが見つかりません: {explicit}"
        )));
    }

    // シンボリックリンクや`..`でルート外へ出ていないかを実体で確かめる
    let root = dir
        .canonicalize()
        .map_err(|e| CoreError::Validation(format!("プロジェクトルートを解決できません: {e}")))?;
    let resolved = joined
        .canonicalize()
        .map_err(|e| CoreError::Validation(format!("composeファイルを解決できません: {e}")))?;
    if !resolved.starts_with(&root) {
        return Err(CoreError::Validation(format!(
            "composeファイルはプロジェクトルート配下を指定してください: {explicit}"
        )));
    }
    Ok(Some(resolved))
}

/// 差配が一元管理するため、利用者の記述を使わないcomposeのキー。
///
/// どちらもdocker composeが置き換えではなく**合算**するキーであり、overrideでの
/// 上書きでは打ち消せない(`image:`はスカラーなので置き換えになり、overrideで
/// 無効化できるのとは事情が違う)。そのためbase側の記述自体を落とす。
///
/// - `ports`: 利用者の公開設定が残ると意図しないポートがホストに開く。
///   衝突検証はDBを見るだけなので、この経路で開いたポートはすり抜ける
/// - `env_file`: 参照先は利用者のプロジェクト内の相対パスであることが多いが、
///   起動時のカレントにはbase.yml・override.yml・`.env`しか無く、存在しない
///   ファイルを指したまま起動しようとして失敗する
///
/// `volumes`は含めない。マウント先(target)単位でマージされ差配の設定が勝つため、
/// 利用者が別targetへ足した分をそのまま活かせる。
const SAHAI_MANAGED_SERVICE_KEYS: [&str; 2] = ["ports", "env_file"];

/// compose定義から差配が管理するキーを取り除いたYAMLを返す。
pub fn strip_managed_keys(compose_content: &str) -> Result<String, CoreError> {
    let mut doc: serde_yaml::Value = serde_yaml::from_str(compose_content)
        .map_err(|e| CoreError::Validation(format!("composeファイルの解析に失敗しました: {e}")))?;

    if let Some(services) = doc
        .get_mut("services")
        .and_then(serde_yaml::Value::as_mapping_mut)
    {
        for (_, service) in services.iter_mut() {
            if let Some(map) = service.as_mapping_mut() {
                for key in SAHAI_MANAGED_SERVICE_KEYS {
                    map.remove(serde_yaml::Value::String(key.to_string()));
                }
            }
        }
    }

    serde_yaml::to_string(&doc)
        .map_err(|e| CoreError::Validation(format!("composeファイルの再構成に失敗しました: {e}")))
}

pub fn diff_container_names(existing: &[String], desired: &[String]) -> ContainerDiff {
    let existing_set: BTreeSet<&str> = existing.iter().map(String::as_str).collect();
    let desired_set: BTreeSet<&str> = desired.iter().map(String::as_str).collect();

    ContainerDiff {
        added: desired_set
            .difference(&existing_set)
            .map(|s| s.to_string())
            .collect(),
        removed: existing_set
            .difference(&desired_set)
            .map(|s| s.to_string())
            .collect(),
        kept: existing_set
            .intersection(&desired_set)
            .map(|s| s.to_string())
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
services:
  app:
    build: .
  mysql:
    image: mysql:8
"#;

    fn temp_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "sahai_core_test_{label}_{:x}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn find_compose_file_returns_none_when_no_compose_file_exists() {
        let dir = temp_dir("none");
        assert_eq!(find_compose_file(&dir), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn find_compose_file_finds_docker_compose_yml() {
        let dir = temp_dir("dc_yml");
        std::fs::write(dir.join("docker-compose.yml"), "services: {}").unwrap();
        assert_eq!(
            find_compose_file(&dir),
            Some(dir.join("docker-compose.yml"))
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn find_compose_file_finds_compose_yaml_variant() {
        let dir = temp_dir("compose_yaml");
        std::fs::write(dir.join("compose.yaml"), "services: {}").unwrap();
        assert_eq!(find_compose_file(&dir), Some(dir.join("compose.yaml")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn parses_all_service_names() {
        let mut names = parse_service_names(SAMPLE).unwrap();
        names.sort();
        assert_eq!(names, vec!["app".to_string(), "mysql".to_string()]);
    }

    #[test]
    fn parses_only_build_services() {
        let names = parse_build_service_names(SAMPLE).unwrap();
        assert_eq!(names, vec!["app".to_string()]);
    }

    #[test]
    fn invalid_yaml_is_reported() {
        assert!(parse_service_names("not: [valid").is_err());
    }

    #[test]
    fn parse_build_specs_resolves_shorthand_context() {
        let specs = parse_build_specs(SAMPLE).unwrap();
        assert_eq!(
            specs.get("app"),
            Some(&BuildSpec {
                context: ".".to_string(),
                dockerfile: None
            })
        );
        assert_eq!(specs.get("mysql"), None);
    }

    #[test]
    fn parse_build_specs_resolves_detailed_context_and_dockerfile() {
        const COMPOSE: &str = r#"
services:
  frontend:
    build:
      context: ./frontend
      dockerfile: Dockerfile
  backend:
    build:
      context: ./backend
"#;
        let specs = parse_build_specs(COMPOSE).unwrap();
        assert_eq!(
            specs.get("frontend"),
            Some(&BuildSpec {
                context: "./frontend".to_string(),
                dockerfile: Some("Dockerfile".to_string()),
            })
        );
        assert_eq!(
            specs.get("backend"),
            Some(&BuildSpec {
                context: "./backend".to_string(),
                dockerfile: None
            })
        );
    }

    #[test]
    fn parse_build_specs_defaults_context_when_mapping_omits_it() {
        const COMPOSE: &str = r#"
services:
  app:
    build:
      dockerfile: Dockerfile.prod
"#;
        let specs = parse_build_specs(COMPOSE).unwrap();
        assert_eq!(
            specs.get("app"),
            Some(&BuildSpec {
                context: ".".to_string(),
                dockerfile: Some("Dockerfile.prod".to_string()),
            })
        );
    }

    #[test]
    fn diff_detects_added_removed_kept() {
        let existing = vec!["app".to_string(), "mysql".to_string()];
        let desired = vec!["app".to_string(), "redis".to_string()];
        let diff = diff_container_names(&existing, &desired);
        assert_eq!(diff.added, vec!["redis".to_string()]);
        assert_eq!(diff.removed, vec!["mysql".to_string()]);
        assert_eq!(diff.kept, vec!["app".to_string()]);
    }

    #[test]
    fn diff_of_identical_sets_is_all_kept() {
        let names = vec!["app".to_string(), "mysql".to_string()];
        let diff = diff_container_names(&names, &names);
        assert!(diff.added.is_empty());
        assert!(diff.removed.is_empty());
        assert_eq!(diff.kept.len(), 2);
    }

    #[test]
    fn diff_rename_is_delete_plus_add() {
        // compose_contentの編集章: サービスキーの変更は削除+新規追加として扱われる
        let existing = vec!["web".to_string()];
        let desired = vec!["app".to_string()];
        let diff = diff_container_names(&existing, &desired);
        assert_eq!(diff.added, vec!["app".to_string()]);
        assert_eq!(diff.removed, vec!["web".to_string()]);
        assert!(diff.kept.is_empty());
    }

    #[test]
    fn strip_managed_keys_removes_ports_from_every_service() {
        let yaml = "services:
  web:
    image: nginx
    ports:
      - \"3000:3000\"
  db:
    image: postgres
    ports:
      - \"5432:5432\"
";
        let out = strip_managed_keys(yaml).unwrap();
        assert!(!out.contains("ports"), "{out}");
        // サービス自体は残る
        assert!(out.contains("web"));
        assert!(out.contains("db"));
        assert!(out.contains("nginx"));
    }

    #[test]
    fn strip_managed_keys_keeps_other_keys() {
        let yaml = "services:
  web:
    image: nginx
    ports:
      - \"80:80\"
    volumes:
      - myvol:/cache
    environment:
      FOO: bar
    expose:
      - \"8080\"
volumes:
  myvol:
";
        let out = strip_managed_keys(yaml).unwrap();
        assert!(!out.contains("\"80:80\""), "{out}");
        // volumesはtarget単位でマージされ差配の設定が勝つため残す
        assert!(out.contains("myvol"), "{out}");
        assert!(out.contains("FOO"), "{out}");
        // exposeはホストに公開しないため残す
        assert!(out.contains("expose"), "{out}");
    }

    #[test]
    fn strip_managed_keys_removes_env_file_from_every_service() {
        let yaml = "services:
  web:
    image: nginx
    env_file:
      - ./web.env
  db:
    image: postgres
    env_file: ./db.env
";
        let out = strip_managed_keys(yaml).unwrap();
        assert!(!out.contains("env_file"), "{out}");
        assert!(!out.contains("web.env"), "{out}");
        // 単一文字列で書かれた形式も落とせること
        assert!(!out.contains("db.env"), "{out}");
        assert!(out.contains("postgres"), "{out}");
    }

    /// environment: は差配の管理外なので残す。env_fileと違いマッピングであり、
    /// overrideとマージされても両方の値が生きる。
    #[test]
    fn strip_managed_keys_keeps_inline_environment() {
        let yaml = "services:
  web:
    image: nginx
    env_file:
      - ./web.env
    environment:
      FOO: bar
";
        let out = strip_managed_keys(yaml).unwrap();
        assert!(!out.contains("env_file"), "{out}");
        assert!(out.contains("FOO"), "{out}");
        assert!(out.contains("bar"), "{out}");
    }

    #[test]
    fn strip_managed_keys_is_a_no_op_without_ports() {
        let yaml = "services:
  web:
    image: nginx
";
        let out = strip_managed_keys(yaml).unwrap();
        assert!(out.contains("nginx"));
    }

    #[test]
    fn strip_managed_keys_rejects_broken_yaml() {
        assert!(strip_managed_keys(
            "services:
  - ["
        )
        .is_err());
    }

    #[test]
    fn resolve_compose_file_falls_back_to_discovery() {
        let dir = temp_dir("resolve_default");
        std::fs::write(dir.join("compose.yaml"), "services: {}").unwrap();
        assert_eq!(
            resolve_compose_file(&dir, None).unwrap(),
            Some(dir.join("compose.yaml"))
        );
    }

    /// 既定名以外を明示できることが本題。compose.prod.yamlは自動探索では見つからない。
    #[test]
    fn resolve_compose_file_accepts_non_default_name() {
        let dir = temp_dir("resolve_explicit");
        std::fs::write(dir.join("compose.prod.yaml"), "services: {}").unwrap();
        let resolved = resolve_compose_file(&dir, Some("compose.prod.yaml")).unwrap();
        assert!(resolved.is_some());
        assert!(resolved.unwrap().ends_with("compose.prod.yaml"));
    }

    /// 明示した場合は既定名があってもそちらを使わない
    #[test]
    fn resolve_compose_file_prefers_the_explicit_one() {
        let dir = temp_dir("resolve_prefer");
        std::fs::write(dir.join("compose.yaml"), "services: {}").unwrap();
        std::fs::write(dir.join("compose.prod.yaml"), "services: {}").unwrap();
        let resolved = resolve_compose_file(&dir, Some("compose.prod.yaml"))
            .unwrap()
            .unwrap();
        assert!(resolved.ends_with("compose.prod.yaml"));
    }

    #[test]
    fn resolve_compose_file_rejects_missing_file() {
        let dir = temp_dir("resolve_missing");
        let err = resolve_compose_file(&dir, Some("nosuch.yaml")).unwrap_err();
        assert!(err.to_string().contains("見つかりません"), "{err}");
    }

    /// ルート外はアーカイブに含まれず、サーバー側で必ず失敗する
    #[test]
    fn resolve_compose_file_rejects_paths_outside_the_project_root() {
        let dir = temp_dir("resolve_escape");
        let outside = dir.parent().unwrap().join("outside-compose.yaml");
        std::fs::write(&outside, "services: {}").unwrap();
        let err = resolve_compose_file(&dir, Some("../outside-compose.yaml")).unwrap_err();
        assert!(err.to_string().contains("プロジェクトルート配下"), "{err}");
        let _ = std::fs::remove_file(&outside);
    }
}
