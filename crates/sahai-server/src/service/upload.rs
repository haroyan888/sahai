//! `POST /api/services/upload`のオーケストレーション: tar.gz展開→image/compose判定→
//! `docker::build_runtime::build_and_push`→`service::registration::create`への委譲。
//! ポート・env・ボリュームは扱わない(それらはWeb UIの既存登録フォームの責務のまま。
//! CLIは「サービスの追加」のみに専念するというユーザー確定方針)。

use std::path::{Path, PathBuf};

use sahai_core::validation;

use crate::api::dto::{
    ContainerInput, CreateServiceRequest, UpdateServiceRequest, UpdateUploadMetadata,
    UploadServiceMetadata,
};
use crate::docker::build_runtime;
use crate::error::AppError;
use crate::service::registration;
use crate::state::AppState;

/// `build.context`/`build.dockerfile`がアップロードされたプロジェクト内に実在することを確かめる。
///
/// CLIは`--context`配下だけをtar.gz化し、その際`.dockerignore`の除外も適用する。そのため
/// compose定義がプロジェクト外(`../shared`等)や、除外されて届いていないディレクトリを
/// 指していると、そのまま`docker build`へ渡しても原因の分かりにくい失敗になる。
/// ここで弾いて、何を直せばよいかが分かるメッセージを返す。
fn resolve_build_context(
    extract_dir: &Path,
    compose_service_name: &str,
    spec: &sahai_core::compose::BuildSpec,
) -> Result<PathBuf, AppError> {
    let field = format!("compose_content[{compose_service_name}]");
    let joined = extract_dir.join(&spec.context);

    // 実在確認が先。canonicalizeは存在しないパスに使えない
    if !joined.is_dir() {
        return Err(AppError::validation_single(
            field,
            format!(
                "build.context '{}' がアップロードされたプロジェクト内に見つかりません。                 プロジェクトルートからの相対パスで指定し、.dockerignoreで除外されていないか確認してください。",
                spec.context
            ),
        ));
    }

    let root = extract_dir
        .canonicalize()
        .map_err(|e| AppError::Internal(e.to_string()))?;
    let resolved = joined
        .canonicalize()
        .map_err(|e| AppError::Internal(e.to_string()))?;
    if !resolved.starts_with(&root) {
        return Err(AppError::validation_single(
            field,
            format!(
                "build.context '{}' がプロジェクトの外を指しています。アップロードされる範囲に含まれないため、                 プロジェクトルート配下の相対パスで指定してください。",
                spec.context
            ),
        ));
    }

    if let Some(dockerfile) = &spec.dockerfile {
        let df = resolved.join(dockerfile);
        if !df.is_file() {
            return Err(AppError::validation_single(
                field,
                format!("build.dockerfile '{dockerfile}' が build.context 内に見つかりません。"),
            ));
        }
        if !df
            .canonicalize()
            .map_err(|e| AppError::Internal(e.to_string()))?
            .starts_with(&root)
        {
            return Err(AppError::validation_single(
                field,
                format!("build.dockerfile '{dockerfile}' がプロジェクトの外を指しています。"),
            ));
        }
    }

    Ok(resolved)
}

/// 展開先ディレクトリを、成功・失敗・キャンセル(クライアント切断等によるFutureの破棄)の
/// いずれの経路でも確実に片付けるためのガード。async関数はキャンセルされると実行中の
/// `await`以降が実行されずFutureごと破棄されるため、単純に処理末尾で`remove_dir_all`する
/// だけでは掃除漏れが起こりうる(ビルドに数分かかる本処理では現実的に起こりうる)。
struct ExtractDirGuard {
    path: PathBuf,
}

impl ExtractDirGuard {
    fn new(path: PathBuf) -> Self {
        ExtractDirGuard { path }
    }
}

impl Drop for ExtractDirGuard {
    fn drop(&mut self) {
        let path = self.path.clone();
        tokio::spawn(async move {
            let _ = tokio::fs::remove_dir_all(&path).await;
        });
    }
}

pub async fn create_from_archive(
    state: &AppState,
    metadata: UploadServiceMetadata,
    archive_bytes: Vec<u8>,
) -> Result<crate::domain::ServiceDetail, AppError> {
    // 1. 安いチェックを先に済ませ、無駄なビルドを避ける
    validation::validate_service_name(&metadata.name)
        .map_err(|e| AppError::validation_single("name", e.to_string()))?;

    if crate::repo::services::find_by_id_or_name(state.db.pool(), &metadata.name)
        .await?
        .is_some()
    {
        return Err(AppError::Conflict(format!(
            "サービス '{}' は既に登録されています",
            metadata.name
        )));
    }

    let has_registry_credentials = {
        let settings = state.settings.read().await;
        settings.registry_username.is_some() && settings.registry_password.is_some()
    };
    if !has_registry_credentials {
        return Err(AppError::Unprocessable(
            "サーバーにレジストリ認証情報が設定されていません(設定画面の「レジストリ設定」から入力してください)"
                .to_string(),
        ));
    }

    // 2〜3. 展開先ディレクトリ(uuidで並行アップロード時の衝突を回避)へtar.gzを展開
    let (extract_dir, _guard) = extract_uploaded_archive(state, archive_bytes).await?;

    // 4. image/compose判定とビルド+push
    let build_args: Vec<(String, String)> = metadata
        .build_args
        .iter()
        .map(|b| (b.key.clone(), b.value.clone()))
        .collect();
    let registry_url = state.settings.read().await.registry_url.clone();
    let req = build_request(&metadata, &extract_dir, &registry_url, &build_args).await?;

    // 5. 既存のservice::registration::createへ委譲(トランザクション・rollbackパターンを
    //    そのまま再利用する。ビルドがここより前で失敗した場合はcreateが一度も
    //    呼ばれないため、それ自体が自然なロールバックになる)
    registration::create(state, req).await
    // extract_dirは_guardのDropで削除される(成功・失敗どちらの経路でも)
}

/// `POST /api/services/{id_or_name}/upload`: `sahai service update`が送るmultipart/form-data
/// を受け取り、既存サービスのプロジェクトを現在の状態でビルド+push(常に:latestを上書き)する。
/// image型はタグを上書きするだけ(常に`:latest`のため、DB上の`image`列は変更不要)。
/// compose型はビルド+push後、新しい`compose_content`を`service::update::update`経由で保存し、
/// 追加/削除されたcomposeサービスに応じて`ServiceContainer`を同期する(
/// 「compose_contentの編集」と同じdiffロジックを再利用する。既存コンテナのports/volumesは
/// 維持される)。ポート・env・ボリュームはWeb UIの責務のままここでは変更しない。
/// ビルドしたイメージの実際の反映(コンテナ再作成)には別途start/restartが必要
/// (他のメタデータ更新と同様、次回start/restart時に反映)。
pub async fn update_from_archive(
    state: &AppState,
    id_or_name: &str,
    metadata: UpdateUploadMetadata,
    archive_bytes: Vec<u8>,
) -> Result<crate::domain::ServiceDetail, AppError> {
    // 1. 対象サービスの存在確認(無ければload_detailがNotFoundを返す)
    let current = super::load_detail(state, id_or_name).await?;

    let has_registry_credentials = {
        let settings = state.settings.read().await;
        settings.registry_username.is_some() && settings.registry_password.is_some()
    };
    if !has_registry_credentials {
        return Err(AppError::Unprocessable(
            "サーバーにレジストリ認証情報が設定されていません(設定画面の「レジストリ設定」から入力してください)"
                .to_string(),
        ));
    }

    // 2. 展開先ディレクトリへtar.gzを展開
    let (extract_dir, _guard) = extract_uploaded_archive(state, archive_bytes).await?;

    // 3. アップロードされたプロジェクト構成が登録済みのsource_typeと一致するか確認
    //    (sahai-cli側の`container push`と同じ判定。ここでも再確認するのは、
    //    このAPIをCLI以外から直接叩かれた場合の取り違え防止のため)
    let resolved_compose =
        sahai_core::compose::resolve_compose_file(&extract_dir, metadata.compose_file.as_deref())
            .map_err(|e| AppError::validation_single("compose_file", e.to_string()))?;
    let is_compose_dir = resolved_compose.is_some();
    let is_compose_registered = current.service.source_type == crate::domain::SourceType::Compose;
    if is_compose_dir != is_compose_registered {
        return Err(AppError::Unprocessable(format!(
            "登録内容(source_type={})とアップロードされたプロジェクト構成が一致しません",
            current.service.source_type.as_str()
        )));
    }

    let build_args: Vec<(String, String)> = metadata
        .build_args
        .iter()
        .map(|b| (b.key.clone(), b.value.clone()))
        .collect();
    let registry_url = state.settings.read().await.registry_url.clone();

    match current.service.source_type {
        crate::domain::SourceType::Image => {
            let tag = format!("{registry_url}/{}:latest", current.service.name);
            build_runtime::build_and_push(
                &extract_dir,
                &tag,
                None,
                metadata.platform.as_deref(),
                &build_args,
            )
            .await
            .map_err(|e| AppError::BuildFailed(e.to_string()))?;
            super::load_detail_by_id(state, current.service.id).await
        }
        crate::domain::SourceType::Compose => {
            let compose_path = resolved_compose.expect("is_compose_dirで存在確認済み");
            let content = tokio::fs::read_to_string(&compose_path)
                .await
                .map_err(|e| AppError::Internal(e.to_string()))?;
            let build_specs = sahai_core::compose::parse_build_specs(&content)
                .map_err(|e| AppError::validation_single("compose_content", e.to_string()))?;

            // 全件を先に検証してから初めてビルドへ進む(1件でも不正なら1件もビルドしない)
            for compose_service_name in build_specs.keys() {
                validation::validate_compose_service_name(compose_service_name).map_err(|e| {
                    AppError::validation_single(
                        format!("compose_content[{compose_service_name}]"),
                        e.to_string(),
                    )
                })?;
                let tag_name = sahai_core::naming::registry_tag_name(
                    &current.service.name,
                    Some(compose_service_name),
                );
                validation::validate_registry_tag_length(&tag_name).map_err(|e| {
                    AppError::validation_single(
                        format!("compose_content[{compose_service_name}]"),
                        e.to_string(),
                    )
                })?;
            }

            for (compose_service_name, spec) in &build_specs {
                let tag_name = sahai_core::naming::registry_tag_name(
                    &current.service.name,
                    Some(compose_service_name),
                );
                let tag = format!("{registry_url}/{tag_name}:latest");
                let service_context =
                    resolve_build_context(&extract_dir, compose_service_name, spec)?;
                build_runtime::build_and_push(
                    &service_context,
                    &tag,
                    spec.dockerfile.as_deref(),
                    metadata.platform.as_deref(),
                    &build_args,
                )
                .await
                .map_err(|e| AppError::BuildFailed(e.to_string()))?;
            }

            // compose_contentの保存とServiceContainerの同期は既存のPUT処理を再利用する
            // (compose_content編集と同じdiffロジック。ports/volumesは維持され、
            // 追加/削除されたcomposeサービスのみ反映される)
            super::update::update(
                state,
                id_or_name,
                UpdateServiceRequest {
                    compose_content: Some(content),
                    ..Default::default()
                },
            )
            .await
        }
    }
    // extract_dirは_guardのDropで削除される(成功・失敗どちらの経路でも)
}

/// tar.gzをuuid名の一時ディレクトリへ展開する(`create_from_archive`・`update_from_archive`
/// 共通)。返り値の`ExtractDirGuard`がスコープを外れると展開先ディレクトリが自動削除される。
async fn extract_uploaded_archive(
    state: &AppState,
    archive_bytes: Vec<u8>,
) -> Result<(PathBuf, ExtractDirGuard), AppError> {
    let extract_dir = state
        .config
        .sahai_data_root
        .join("uploads")
        .join(uuid::Uuid::new_v4().to_string());
    tokio::fs::create_dir_all(&extract_dir)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;
    let guard = ExtractDirGuard::new(extract_dir.clone());

    let dir_for_extract = extract_dir.clone();
    tokio::task::spawn_blocking(move || extract_tar_gz(&archive_bytes, &dir_for_extract))
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?
        .map_err(AppError::Unprocessable)?;

    Ok((extract_dir, guard))
}

async fn build_request(
    metadata: &UploadServiceMetadata,
    extract_dir: &Path,
    registry_url: &str,
    build_args: &[(String, String)],
) -> Result<CreateServiceRequest, AppError> {
    let resolved =
        sahai_core::compose::resolve_compose_file(extract_dir, metadata.compose_file.as_deref())
            .map_err(|e| AppError::validation_single("compose_file", e.to_string()))?;
    match resolved {
        Some(compose_path) => {
            build_compose_request(
                metadata,
                extract_dir,
                &compose_path,
                registry_url,
                build_args,
            )
            .await
        }
        None => build_image_request(metadata, extract_dir, registry_url, build_args).await,
    }
}

async fn build_image_request(
    metadata: &UploadServiceMetadata,
    extract_dir: &Path,
    registry_url: &str,
    build_args: &[(String, String)],
) -> Result<CreateServiceRequest, AppError> {
    let tag = format!("{registry_url}/{}:latest", metadata.name);
    build_runtime::build_and_push(
        extract_dir,
        &tag,
        None,
        metadata.platform.as_deref(),
        build_args,
    )
    .await
    .map_err(|e| AppError::BuildFailed(e.to_string()))?;

    Ok(CreateServiceRequest {
        name: metadata.name.clone(),
        source_type: "image".to_string(),
        image: Some(tag),
        compose_content: None,
        env_vars: None,
        containers: vec![ContainerInput {
            name: metadata.name.clone(),
            ports: vec![],
            volumes: vec![],
        }],
    })
}

async fn build_compose_request(
    metadata: &UploadServiceMetadata,
    extract_dir: &Path,
    compose_path: &Path,
    registry_url: &str,
    build_args: &[(String, String)],
) -> Result<CreateServiceRequest, AppError> {
    let content = tokio::fs::read_to_string(compose_path)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;
    let build_specs = sahai_core::compose::parse_build_specs(&content)
        .map_err(|e| AppError::validation_single("compose_content", e.to_string()))?;

    // 全件を先に検証してから初めてビルドへ進む(1件でも不正なら1件もビルドしない)
    for compose_service_name in build_specs.keys() {
        validation::validate_compose_service_name(compose_service_name).map_err(|e| {
            AppError::validation_single(
                format!("compose_content[{compose_service_name}]"),
                e.to_string(),
            )
        })?;
        let tag_name =
            sahai_core::naming::registry_tag_name(&metadata.name, Some(compose_service_name));
        validation::validate_registry_tag_length(&tag_name).map_err(|e| {
            AppError::validation_single(
                format!("compose_content[{compose_service_name}]"),
                e.to_string(),
            )
        })?;
    }

    for (compose_service_name, spec) in &build_specs {
        let tag_name =
            sahai_core::naming::registry_tag_name(&metadata.name, Some(compose_service_name));
        let tag = format!("{registry_url}/{tag_name}:latest");
        let service_context = resolve_build_context(extract_dir, compose_service_name, spec)?;
        build_runtime::build_and_push(
            &service_context,
            &tag,
            spec.dockerfile.as_deref(),
            metadata.platform.as_deref(),
            build_args,
        )
        .await
        .map_err(|e| AppError::BuildFailed(e.to_string()))?;
    }

    Ok(CreateServiceRequest {
        name: metadata.name.clone(),
        source_type: "compose".to_string(),
        image: None,
        compose_content: Some(content),
        // containers[]に明示しなくても、registration::createのinsert_allが
        // compose_contentから検出した全サービスをports/volumes空で自動作成する
        // (registration.rs参照)
        env_vars: None,
        containers: vec![],
    })
}

/// tar.gzを`dest`配下へ展開する。同期処理のため呼び出し元は`spawn_blocking`経由で呼ぶこと。
/// 展開後サイズの上限(1GiB)。HTTPボディの上限は圧縮後のサイズにしか効かず、
/// gzipは最大1000:1程度まで圧縮できるため、これが無いと数MBのアーカイブで
/// データルートを埋め尽くせてしまう(DB・全サービスのボリュームが道連れになる)。
/// 実測でこのプロジェクト自体が展開後1.4MB程度なので、通常の利用で当たることはない。
const MAX_EXTRACTED_BYTES: u64 = 1024 * 1024 * 1024;

/// 読み取った累計バイト数が上限を超えたらエラーにする`Read`ラッパー。
/// tarヘッダのサイズ申告は信用できないため、実際に読めたバイト数で数える。
struct LimitedReader<R> {
    inner: R,
    total: u64,
    limit: u64,
}

impl<R: std::io::Read> std::io::Read for LimitedReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.total += n as u64;
        if self.total > self.limit {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "{SIZE_LIMIT_PREFIX}({})を超えました。不要なファイルを.dockerignoreで除外してください",
                    human_size(self.limit)
                ),
            ));
        }
        Ok(n)
    }
}

/// 上限超過メッセージの先頭。tarが包んだエラーから本文だけを取り出すのに使う。
const SIZE_LIMIT_PREFIX: &str = "アーカイブの展開サイズが上限";

fn human_size(bytes: u64) -> String {
    const GB: u64 = 1024 * 1024 * 1024;
    const MB: u64 = 1024 * 1024;
    if bytes >= GB {
        format!("{}GB", bytes / GB)
    } else if bytes >= MB {
        format!("{}MB", bytes / MB)
    } else {
        format!("{}KB", bytes / 1024)
    }
}

/// tarクレートはio::Errorを`failed to unpack <内部パス>`で二重に包むため、
/// そのままでは上限超過という本当の原因が利用者に見えない。
/// 上限超過だけは本文のみを返し、それ以外はsource()の連鎖を展開して繋ぐ。
fn describe_extract_error(e: &dyn std::error::Error) -> String {
    let chain = describe_error_chain(e);
    match chain.find(SIZE_LIMIT_PREFIX) {
        Some(pos) => chain[pos..].to_string(),
        None => chain,
    }
}

/// source()の連鎖を`: `で繋ぐ。
fn describe_error_chain(e: &dyn std::error::Error) -> String {
    let mut parts = vec![e.to_string()];
    let mut source = e.source();
    while let Some(s) = source {
        parts.push(s.to_string());
        source = s.source();
    }
    parts.join(": ")
}

fn extract_tar_gz(bytes: &[u8], dest: &Path) -> Result<(), String> {
    extract_tar_gz_with_limit(bytes, dest, MAX_EXTRACTED_BYTES)
}

fn extract_tar_gz_with_limit(bytes: &[u8], dest: &Path, limit: u64) -> Result<(), String> {
    let decoder = flate2::read::GzDecoder::new(bytes);
    let limited = LimitedReader {
        inner: decoder,
        total: 0,
        limit,
    };
    let mut archive = tar::Archive::new(limited);
    let entries = archive.entries().map_err(|e| describe_extract_error(&e))?;
    for entry in entries {
        let mut entry = entry.map_err(|e| describe_extract_error(&e))?;
        let path = entry
            .path()
            .map_err(|e| describe_extract_error(&e))?
            .into_owned();
        // tar crate自体もパストラバーサル対策を持つが、念のため明示的にも拒否する
        if path
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
        {
            return Err(format!("不正なアーカイブエントリです: {}", path.display()));
        }
        entry
            .unpack_in(dest)
            .map_err(|e| describe_extract_error(&e))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "sahai_server_upload_test_{label}_{:x}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn make_tar_gz(files: &[(&str, &str)]) -> Vec<u8> {
        let mut builder = tar::Builder::new(flate2::write::GzEncoder::new(
            Vec::new(),
            flate2::Compression::default(),
        ));
        for (name, content) in files {
            let mut header = tar::Header::new_gnu();
            header.set_size(content.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            builder
                .append_data(&mut header, name, content.as_bytes())
                .unwrap();
        }
        builder.into_inner().unwrap().finish().unwrap()
    }

    /// `tar::Builder::append_data`(内部で`Header::set_path`を呼ぶ)は`..`を含む
    /// パスをその場で拒否するため、正規のAPI経由では悪意あるアーカイブを作れない
    /// (tar crate自体の安全策)。それでも`extract_tar_gz`側の防御的チェックが
    /// 効くことを検証するため、ヘッダーの生バイト列に直接パスを書き込み
    /// `set_path`の検証を経由しない`Builder::append`(pathを引数に取らない版)で
    /// 不正なアーカイブを構築する。
    fn make_tar_gz_with_raw_path(evil_path: &[u8], content: &[u8]) -> Vec<u8> {
        let mut header = tar::Header::new_gnu();
        header.as_mut_bytes()[0..evil_path.len()].copy_from_slice(evil_path);
        header.set_size(content.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();

        let mut builder = tar::Builder::new(flate2::write::GzEncoder::new(
            Vec::new(),
            flate2::Compression::default(),
        ));
        builder.append(&header, content).unwrap();
        builder.into_inner().unwrap().finish().unwrap()
    }

    fn build_spec(context: &str, dockerfile: Option<&str>) -> sahai_core::compose::BuildSpec {
        sahai_core::compose::BuildSpec {
            context: context.to_string(),
            dockerfile: dockerfile.map(|s| s.to_string()),
        }
    }

    #[test]
    fn resolve_build_context_accepts_path_inside_project() {
        let root = temp_dir("ctx_ok");
        std::fs::create_dir_all(root.join("backend")).unwrap();

        let resolved = resolve_build_context(&root, "app", &build_spec("./backend", None)).unwrap();

        assert!(resolved.ends_with("backend"));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// `.dockerignore`で除外された等でアーカイブに含まれていないcontextは、
    /// docker buildへ渡す前に原因の分かるエラーにする。
    #[test]
    fn resolve_build_context_rejects_missing_directory() {
        let root = temp_dir("ctx_missing");
        std::fs::create_dir_all(&root).unwrap();

        let err = resolve_build_context(&root, "app", &build_spec("./backend", None)).unwrap_err();

        match err {
            AppError::Validation(fields) => {
                assert!(fields[0].message.contains("見つかりません"), "{:?}", fields);
                assert!(fields[0].message.contains(".dockerignore"), "{:?}", fields);
            }
            other => panic!("Validationを期待: {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    /// `../shared`のようにプロジェクト外を指すcontextはアップロード範囲に含まれない。
    #[test]
    fn resolve_build_context_rejects_path_outside_project() {
        let base = temp_dir("ctx_outside");
        let root = base.join("project");
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::create_dir_all(base.join("shared")).unwrap();

        let err = resolve_build_context(&root, "app", &build_spec("../shared", None)).unwrap_err();

        match err {
            AppError::Validation(fields) => {
                assert!(
                    fields[0].message.contains("プロジェクトの外"),
                    "{:?}",
                    fields
                );
            }
            other => panic!("Validationを期待: {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn resolve_build_context_rejects_missing_dockerfile() {
        let root = temp_dir("ctx_df_missing");
        std::fs::create_dir_all(&root).unwrap();

        let err = resolve_build_context(&root, "app", &build_spec(".", Some("Dockerfile.prod")))
            .unwrap_err();

        match err {
            AppError::Validation(fields) => {
                assert!(
                    fields[0].message.contains("Dockerfile.prod"),
                    "{:?}",
                    fields
                );
            }
            other => panic!("Validationを期待: {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn resolve_build_context_accepts_existing_dockerfile() {
        let root = temp_dir("ctx_df_ok");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("Dockerfile.prod"),
            "FROM scratch
",
        )
        .unwrap();

        let resolved =
            resolve_build_context(&root, "app", &build_spec(".", Some("Dockerfile.prod"))).unwrap();

        assert!(resolved.is_dir());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn extract_tar_gz_writes_files_into_dest() {
        let dest = temp_dir("ok");
        let archive = make_tar_gz(&[("Dockerfile", "FROM scratch\n")]);

        extract_tar_gz(&archive, &dest).unwrap();

        let content = std::fs::read_to_string(dest.join("Dockerfile")).unwrap();
        assert_eq!(content, "FROM scratch\n");
        let _ = std::fs::remove_dir_all(&dest);
    }

    /// 圧縮爆弾対策。小さなアーカイブでも展開後が上限を超えたら止める。
    #[test]
    fn extract_tar_gz_rejects_archive_exceeding_size_limit() {
        let dest = temp_dir("bomb");
        // ゼロ埋め64KBはgzipで数百バイトに縮む(圧縮後は小さいが展開後は大きい)
        let big = "0".repeat(64 * 1024);
        let archive = make_tar_gz(&[("big.txt", &big)]);
        assert!(
            archive.len() < 4096,
            "圧縮後は十分小さいこと: {}",
            archive.len()
        );

        let result = extract_tar_gz_with_limit(&archive, &dest, 8 * 1024);

        let err = result.expect_err("上限超過は拒否されるべき");
        assert!(err.contains("展開サイズ"), "{err}");
        assert!(err.contains(".dockerignore"), "{err}");
        let _ = std::fs::remove_dir_all(&dest);
    }

    /// 上限超過時に利用者へ届くメッセージ全文を固定する(tarの包み込みで
    /// 原因が隠れる回帰を防ぐ)。
    #[test]
    fn size_limit_error_message_reaches_the_user() {
        let dest = temp_dir("bomb_msg");
        let big = "0".repeat(64 * 1024);
        let archive = make_tar_gz(&[("big.txt", &big)]);

        let err = extract_tar_gz_with_limit(&archive, &dest, 8 * 1024).unwrap_err();

        println!("利用者に届くメッセージ: {err}");
        assert!(
            err.starts_with("アーカイブの展開サイズが上限"),
            "本文だけが返るべき: {err}"
        );
        assert!(
            !err.contains("failed to unpack"),
            "tarの内部メッセージは出さない: {err}"
        );
        let _ = std::fs::remove_dir_all(&dest);
    }

    /// 上限内であれば通常どおり展開できる(境界の回帰確認)。
    #[test]
    fn extract_tar_gz_accepts_archive_within_size_limit() {
        let dest = temp_dir("within_limit");
        let content = "x".repeat(1024);
        let archive = make_tar_gz(&[("small.txt", &content)]);

        extract_tar_gz_with_limit(&archive, &dest, 1024 * 1024).unwrap();

        assert_eq!(
            std::fs::read_to_string(dest.join("small.txt"))
                .unwrap()
                .len(),
            1024
        );
        let _ = std::fs::remove_dir_all(&dest);
    }

    #[test]
    fn extract_tar_gz_rejects_parent_dir_traversal() {
        let dest = temp_dir("traversal");
        let archive = make_tar_gz_with_raw_path(b"../escape.txt", b"evil");

        let result = extract_tar_gz(&archive, &dest);

        assert!(result.is_err(), "'..'を含むエントリは拒否されるべき");
        let _ = std::fs::remove_dir_all(&dest);
    }

    // update_from_archiveの早期チェック(docker build/pushに到達する前に弾かれる経路)。
    // 実docker CLIを必要としないため、build_and_push到達前に失敗することを検証する。
    mod update_from_archive_tests {
        use super::*;
        use crate::api::dto::{ContainerInput, CreateServiceRequest, UpdateUploadMetadata};
        use crate::service::{registration, test_support::test_state};

        fn empty_metadata() -> UpdateUploadMetadata {
            UpdateUploadMetadata {
                build_args: vec![],
                platform: None,
                compose_file: None,
            }
        }

        async fn register_image_service(state: &AppState, name: &str, host_port: Option<i64>) {
            registration::create(
                state,
                CreateServiceRequest {
                    name: name.to_string(),
                    source_type: "image".to_string(),
                    image: Some("x:latest".to_string()),
                    compose_content: None,
                    env_vars: None,
                    containers: vec![ContainerInput {
                        name: name.to_string(),
                        ports: vec![crate::api::dto::PortInput {
                            container_port: 80,
                            host_port,
                            protocol: "tcp".to_string(),
                            is_http: true,
                        }],
                        volumes: vec![],
                    }],
                },
            )
            .await
            .unwrap();
        }

        #[tokio::test]
        async fn rejects_unknown_service_with_not_found() {
            let state = test_state().await;
            let archive = make_tar_gz(&[("Dockerfile", "FROM scratch\n")]);

            let result =
                update_from_archive(&state, "doesnotexist", empty_metadata(), archive).await;

            assert!(matches!(result, Err(AppError::NotFound(_))), "{result:?}");
        }

        #[tokio::test]
        async fn rejects_when_registry_credentials_are_missing() {
            let state = test_state().await;
            register_image_service(&state, "myapp", Some(21100)).await;
            let archive = make_tar_gz(&[("Dockerfile", "FROM scratch\n")]);

            let result = update_from_archive(&state, "myapp", empty_metadata(), archive).await;

            assert!(
                matches!(result, Err(AppError::Unprocessable(_))),
                "{result:?}"
            );
        }

        #[tokio::test]
        async fn rejects_when_uploaded_project_shape_does_not_match_registered_source_type() {
            let state = test_state().await;
            register_image_service(&state, "myapp", Some(21101)).await;
            {
                let mut settings = state.settings.write().await;
                settings.registry_username = Some("u".to_string());
                settings.registry_password = Some("p".to_string());
            }
            // image型で登録済みなのにcompose.ymlを含むアーカイブをアップロードする
            let archive = make_tar_gz(&[("compose.yml", "services:\n  app:\n    build: .\n")]);

            let result = update_from_archive(&state, "myapp", empty_metadata(), archive).await;

            match result {
                Err(AppError::Unprocessable(msg)) => {
                    assert!(msg.contains("source_type=image"), "{msg}");
                }
                other => panic!("Unprocessableを期待: {other:?}"),
            }
        }
    }
}
