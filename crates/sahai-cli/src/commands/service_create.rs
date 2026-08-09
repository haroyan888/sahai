//! `sahai service create <name>`: プロジェクトをサーバーへアップロードし、
//! サーバー側でビルド+push+サービスの新規登録までを一括で行う。
//! ポート・env・ボリュームは扱わない。それらは引き続きWeb UIで設定する
//! (CLIは「サービスの追加」のみに専念する)。

use std::path::PathBuf;

use serde::Serialize;
use serde_json::Value;

use crate::api_client::ApiClient;
use crate::commands::archive::build_archive;
use crate::commands::precheck::validate_compose_build_targets;

pub struct CreateArgs {
    pub name: String,
    pub context: PathBuf,
    pub build_args: Vec<(String, String)>,
    pub platform: Option<String>,
}

#[derive(Serialize)]
struct BuildArgDto {
    key: String,
    value: String,
}

#[derive(Serialize)]
struct UploadMetadata {
    name: String,
    build_args: Vec<BuildArgDto>,
    platform: Option<String>,
}

pub async fn run(client: &ApiClient, args: CreateArgs) -> Result<(), String> {
    sahai_core::validation::validate_service_name(&args.name).map_err(|e| e.to_string())?;

    validate_compose_build_targets(&args.name, &args.context)?;

    println!("アーカイブを作成中...");
    let archive_bytes = build_archive(&args.context)?;

    let metadata = UploadMetadata {
        name: args.name.clone(),
        build_args: args
            .build_args
            .iter()
            .map(|(k, v)| BuildArgDto {
                key: k.clone(),
                value: v.clone(),
            })
            .collect(),
        platform: args.platform.clone(),
    };
    let metadata_json = serde_json::to_string(&metadata).map_err(|e| e.to_string())?;

    println!("登録中です。サーバー側でビルドしています(数分かかる場合があります)...");
    let detail: Value = client
        .post_multipart("/api/services/upload", metadata_json, archive_bytes)
        .await?;

    // ServiceDetailは`#[serde(flatten)]`でServiceのフィールドをトップレベルに
    // 展開する(サーバー側`domain::ServiceDetail`参照)。ネストした"service"キーは無い
    let name = detail["name"].as_str().unwrap_or(&args.name);
    let subdomain = detail["subdomain"].as_str().unwrap_or("");
    println!("サービス '{name}' を登録しました(サブドメイン: {subdomain})。");
    println!("Web UIでポート・env・ボリュームを設定してから起動してください。");

    Ok(())
}
