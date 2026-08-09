//! `sahai service update <name>`: 登録済みサービスのプロジェクトを現在のディレクトリの
//! 状態でサーバーへ再アップロードし、サーバー側でビルド+push(常に:latestを上書き)を
//! 行う。`service create`の更新版。名前・source_typeの変更はできず(既存サービスと
//! ディレクトリ構成が一致しない場合はサーバー側で拒否される)、ポート・env・ボリュームも
//! ここでは変更しない(引き続きWeb UIの責務)。compose型の場合のみ、compose_content内の
//! サービス追加/削除がServiceContainerへ自動反映される(6章「compose_contentの編集」)。
//! ビルドしたイメージの実際の反映には`--deploy`(`container push`と同じ挙動)が必要。

use std::path::PathBuf;

use serde::Serialize;
use serde_json::Value;

use crate::api_client::ApiClient;
use crate::commands::archive::build_archive;
use crate::commands::precheck::validate_compose_build_targets;

pub struct UpdateArgs {
    pub name: String,
    pub context: PathBuf,
    /// 使用するcomposeファイル。省略時は既定の名前から探す
    pub compose_file: Option<String>,
    pub build_args: Vec<(String, String)>,
    pub platform: Option<String>,
    pub deploy: bool,
}

#[derive(Serialize)]
struct BuildArgDto {
    key: String,
    value: String,
}

#[derive(Serialize)]
struct UpdateUploadMetadata {
    build_args: Vec<BuildArgDto>,
    platform: Option<String>,
    /// 使用するcomposeファイル。省略時はサーバー側が既定の名前を探す
    compose_file: Option<String>,
}

pub async fn run(client: &ApiClient, args: UpdateArgs) -> Result<(), String> {
    validate_compose_build_targets(&args.name, &args.context, args.compose_file.as_deref())?;

    println!("アーカイブを作成中...");
    let archive_bytes = build_archive(&args.context)?;

    let metadata = UpdateUploadMetadata {
        build_args: args
            .build_args
            .iter()
            .map(|(k, v)| BuildArgDto {
                key: k.clone(),
                value: v.clone(),
            })
            .collect(),
        platform: args.platform.clone(),
        compose_file: args.compose_file.clone(),
    };
    let metadata_json = serde_json::to_string(&metadata).map_err(|e| e.to_string())?;

    println!("更新中です。サーバー側でビルドしています(数分かかる場合があります)...");
    let detail: Value = client
        .post_multipart(
            &format!("/api/services/{}/upload", args.name),
            metadata_json,
            archive_bytes,
        )
        .await?;

    let name = detail["name"].as_str().unwrap_or(&args.name);
    println!("サービス '{name}' を現在のプロジェクト状態に更新しました。");

    if args.deploy {
        let _: Value = client
            .post_empty(&format!("/api/services/{}/restart", args.name))
            .await?;
        println!("再デプロイしました。");
    } else {
        println!(
            "反映するには `sahai service restart {name}` を実行するか、Web UIから再起動してください。"
        );
    }

    Ok(())
}
