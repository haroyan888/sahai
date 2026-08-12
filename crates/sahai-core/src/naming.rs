//! `svc-{id}`系の命名生成、レジストリタグ生成、ボリュームホストパス生成。
//! Dockerコンテナ名・composeプロジェクト名・サブドメイン・レジストリタグ・
//! ボリュームパスの組み立てを一箇所に集約する。ここを変えると既存環境の
//! コンテナ・ボリュームとの対応が切れるため、変更は移行手順とセットで行うこと。

use std::path::Path;

/// 土台の3コンテナとサービスのコンテナが共有するDockerネットワーク名。
/// compose.yamlで`name:`を指定して固定してある(既定ではプロジェクト名が
/// 前置され、クローン先のディレクトリ名に依存してしまうため)。
/// Traefikはこのネットワーク越しにコンテナ名を解決して直接転送する。
pub const SAHAI_NETWORK: &str = "sahai";

/// 実際のDockerコンテナ名: `svc-{ServiceContainer.id}`。
/// サービス名の変更やcompose_content編集による再作成の影響を受けない不変ID。
pub fn container_docker_name(container_id: i64) -> String {
    format!("svc-{container_id}")
}

/// docker composeのプロジェクト名: `svc-{Service.id}`。
pub fn compose_project_name(service_id: i64) -> String {
    format!("svc-{service_id}")
}

/// サービスのサブドメインを生成する: `{name}.{domain}`。
/// `domain`はコンテナ起動時に`SAHAI_DOMAIN`環境変数で変更可能(config.rs参照)。
/// SQLiteのGENERATED列は環境変数を参照できないため、Rust側で計算しDBの通常列へ
/// 保存する。
pub fn subdomain_for(name: &str, domain: &str) -> String {
    format!("{name}.{domain}")
}

/// レジストリタグの元となる名前部分を生成する。
/// image型相当(compose_service_name未指定)は `<service-name>`、
/// compose型の各サービスは `<service-name>-<composeサービス名>`。
pub fn registry_tag_name(service_name: &str, compose_service_name: Option<&str>) -> String {
    match compose_service_name {
        Some(compose_name) => format!("{service_name}-{compose_name}"),
        None => service_name.to_string(),
    }
}

/// コンテナ内パスをホスト側ディレクトリ名として正規化する。
/// 例: `/var/lib/mysql` -> `var-lib-mysql`。
pub fn normalize_container_path(container_path: &str) -> String {
    container_path.trim_matches('/').replace('/', "-")
}

/// 永続化ボリュームのホスト側パスを生成する。
/// `<host_data_root>/services/<service_id>/<正規化パス>/`。
/// `container_id`には依存しない。
///
/// 第1引数は**dockerdから見たデータルート**(`Config::host_data_root`)であり、
/// sahai-serverコンテナ内から見えるパス(`Config::sahai_data_root`)ではない。
/// 戻り値はDockerのbind-mount文字列(`docker run -v`やcompose overrideの
/// `volumes:`)に直接埋め込まれ、**dockerdがホスト側のパスとして解決する**ため。
/// sahai-server自身のローカルファイルI/Oにはこの関数を使わない。
///
/// 戻り値は`String`であり、常に`/`区切り。差配のDockerホストは
/// 常にLinuxのため、`PathBuf::join`/`Path::display`のような
/// 実行環境依存のパス区切り文字に頼らず、ここで明示的に`/`を使う
/// (Windows上での`cargo test`実行時に`\`が混入するのを防ぐ。実際に発生した不具合)。
pub fn volume_host_path(host_data_root: &Path, service_id: i64, container_path: &str) -> String {
    let root = host_data_root.display().to_string();
    let root = root.trim_end_matches('/');
    format!(
        "{root}/services/{service_id}/{}",
        normalize_container_path(container_path)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn container_and_project_names_use_numeric_id_only() {
        assert_eq!(container_docker_name(42), "svc-42");
        assert_eq!(compose_project_name(7), "svc-7");
    }

    #[test]
    fn subdomain_for_joins_name_and_domain() {
        assert_eq!(subdomain_for("myapp", "example.com"), "myapp.example.com");
        assert_eq!(subdomain_for("myapp", "example.com"), "myapp.example.com");
    }

    #[test]
    fn registry_tag_name_variants() {
        assert_eq!(registry_tag_name("myapp", None), "myapp");
        assert_eq!(registry_tag_name("myapp", Some("mysql")), "myapp-mysql");
    }

    #[test]
    fn normalize_container_path_strips_slashes() {
        assert_eq!(normalize_container_path("/var/lib/mysql"), "var-lib-mysql");
        assert_eq!(normalize_container_path("/data/"), "data");
        assert_eq!(normalize_container_path("data"), "data");
    }

    #[test]
    fn volume_host_path_ignores_container_id() {
        let root = Path::new("/var/sahai");
        let path = volume_host_path(root, 3, "/var/lib/mysql");
        // 常に`/`区切りの文字列であること(実行環境のパス区切り文字に依存しない)
        assert_eq!(path, "/var/sahai/services/3/var-lib-mysql");
    }
}
