//! 各エンドポイントのハンドラ関数。リクエスト⇄DTOの変換とservice層の呼び出しのみ。

use axum::extract::{Multipart, Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::Serialize;

use crate::api::dto::{
    CreateServiceRequest, DeleteQuery, UpdateServiceRequest, UpdateUploadMetadata,
    UploadServiceMetadata,
};
use crate::error::AppError;
use crate::service;
use crate::state::AppState;

#[derive(Serialize)]
struct ServiceListResponse {
    services: Vec<crate::domain::Service>,
}

pub async fn create(
    State(state): State<AppState>,
    Json(req): Json<CreateServiceRequest>,
) -> Result<impl IntoResponse, AppError> {
    let detail = service::registration::create(&state, req).await?;
    Ok((StatusCode::CREATED, Json(detail)))
}

/// `POST /api/services/upload`: `sahai service create`が送るmultipart/form-data
/// (`metadata`パート=JSON、`archive`パート=tar.gz)を受け取り、サーバー側でのビルド+push+
/// 新規登録(service::upload::create_from_archive)に委譲する。
pub async fn upload(
    State(state): State<AppState>,
    multipart: Multipart,
) -> Result<impl IntoResponse, AppError> {
    let (metadata_text, archive_bytes) = parse_upload_multipart(multipart).await?;
    let metadata: UploadServiceMetadata = serde_json::from_str(&metadata_text)
        .map_err(|e| AppError::validation_single("metadata", e.to_string()))?;

    let detail = service::upload::create_from_archive(&state, metadata, archive_bytes).await?;
    Ok((StatusCode::CREATED, Json(detail)))
}

/// `POST /api/services/{id_or_name}/upload`: `sahai service update`が送るmultipart/form-data
/// を受け取り、既存サービスのプロジェクトを現在の状態でビルド+push(service::upload::
/// update_from_archive)に委譲する。`upload`(新規登録)と異なり`metadata`に`name`は含まない
/// (対象サービスはパスの`id_or_name`で特定するため)。
pub async fn update_upload(
    State(state): State<AppState>,
    Path(id_or_name): Path<String>,
    multipart: Multipart,
) -> Result<impl IntoResponse, AppError> {
    let (metadata_text, archive_bytes) = parse_upload_multipart(multipart).await?;
    let metadata: UpdateUploadMetadata = serde_json::from_str(&metadata_text)
        .map_err(|e| AppError::validation_single("metadata", e.to_string()))?;

    let detail =
        service::upload::update_from_archive(&state, &id_or_name, metadata, archive_bytes).await?;
    Ok((StatusCode::OK, Json(detail)))
}

/// `metadata`パート(JSON文字列そのまま)+`archive`パート(tar.gzバイト列)のmultipart/form-data
/// を読み取る。metadataの実際の型は呼び出し元(用途ごとに異なるDTO)がパースする。
async fn parse_upload_multipart(mut multipart: Multipart) -> Result<(String, Vec<u8>), AppError> {
    let mut metadata_text: Option<String> = None;
    let mut archive_bytes: Option<Vec<u8>> = None;

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| AppError::Unprocessable(format!("multipartの解析に失敗しました: {e}")))?
    {
        match field.name() {
            Some("metadata") => {
                let text = field.text().await.map_err(|e| {
                    AppError::Unprocessable(format!("metadataパートの読み取りに失敗しました: {e}"))
                })?;
                metadata_text = Some(text);
            }
            Some("archive") => {
                let bytes = field.bytes().await.map_err(|e| {
                    AppError::Unprocessable(format!("archiveパートの読み取りに失敗しました: {e}"))
                })?;
                archive_bytes = Some(bytes.to_vec());
            }
            _ => {}
        }
    }

    let metadata_text = metadata_text
        .ok_or_else(|| AppError::validation_single("metadata", "metadataパートが必要です"))?;
    let archive_bytes = archive_bytes
        .ok_or_else(|| AppError::validation_single("archive", "archiveパートが必要です"))?;

    Ok((metadata_text, archive_bytes))
}

pub async fn list(State(state): State<AppState>) -> Result<impl IntoResponse, AppError> {
    let rows = crate::repo::services::list_all(state.db.pool()).await?;
    let mut services = Vec::with_capacity(rows.len());
    for row in rows {
        services.push(crate::domain::Service::try_from(row).map_err(AppError::Internal)?);
    }
    Ok(Json(ServiceListResponse { services }))
}

pub async fn get(
    State(state): State<AppState>,
    Path(id_or_name): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let detail = service::load_detail(&state, &id_or_name).await?;
    Ok(Json(detail))
}

pub async fn update(
    State(state): State<AppState>,
    Path(id_or_name): Path<String>,
    Json(req): Json<UpdateServiceRequest>,
) -> Result<impl IntoResponse, AppError> {
    let detail = service::update::update(&state, &id_or_name, req).await?;
    Ok(Json(detail))
}

pub async fn delete(
    State(state): State<AppState>,
    Path(id_or_name): Path<String>,
    Query(query): Query<DeleteQuery>,
) -> Result<impl IntoResponse, AppError> {
    service::deletion::delete(&state, &id_or_name, query.purge_volumes).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn start(
    State(state): State<AppState>,
    Path(id_or_name): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let detail = service::lifecycle::start(&state, &id_or_name).await?;
    Ok(Json(detail))
}

pub async fn stop(
    State(state): State<AppState>,
    Path(id_or_name): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let detail = service::lifecycle::stop(&state, &id_or_name).await?;
    Ok(Json(detail))
}

pub async fn restart(
    State(state): State<AppState>,
    Path(id_or_name): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let detail = service::lifecycle::restart(&state, &id_or_name).await?;
    Ok(Json(detail))
}

#[derive(Serialize)]
struct StatsResponse {
    containers: Vec<ContainerStatsDto>,
}

#[derive(Serialize)]
struct ContainerStatsDto {
    id: i64,
    name: String,
    cpu_percent: f64,
    memory_usage_bytes: u64,
    memory_limit_bytes: u64,
}

pub async fn stats(
    State(state): State<AppState>,
    Path(id_or_name): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let detail = service::load_detail(&state, &id_or_name).await?;
    if detail.service.status != crate::domain::ServiceStatus::Running {
        return Ok(Json(StatsResponse { containers: vec![] }));
    }

    let mut containers = Vec::with_capacity(detail.containers.len());
    for c in &detail.containers {
        let container_name = sahai_core::naming::container_docker_name(c.container.id);
        if let Ok(s) = state.docker.inspector.stats_once(&container_name).await {
            containers.push(ContainerStatsDto {
                id: c.container.id,
                name: c.container.name.clone(),
                cpu_percent: s.cpu_percent,
                memory_usage_bytes: s.memory_usage_bytes,
                memory_limit_bytes: s.memory_limit_bytes,
            });
        }
    }
    Ok(Json(StatsResponse { containers }))
}

/// ログ配信の既定行数と上限。上限を設けるのは、接続直後にDockerから
/// 大量の行を読み出してブラウザを固まらせないため。
const DEFAULT_LOG_TAIL: u32 = 200;
const MAX_LOG_TAIL: u32 = 5000;

#[derive(serde::Deserialize)]
pub struct LogsQuery {
    /// 対象の`ServiceContainer.id`。省略時はサービスの最初のコンテナ
    container: Option<i64>,
    tail: Option<u32>,
}

#[derive(Serialize)]
struct LogLineDto {
    stream: &'static str,
    timestamp: Option<String>,
    message: String,
}

/// `GET /api/services/{id_or_name}/logs`: コンテナログをSSEで流し続ける
/// (要件定義書9章)。サービス・コンテナの解決とtailの検証はここで済ませ、
/// 接続を確立してからは行を流すだけにする(確立後はJSONのエラーを返せないため)。
pub async fn logs(
    State(state): State<AppState>,
    Path(id_or_name): Path<String>,
    Query(query): Query<LogsQuery>,
) -> Result<impl IntoResponse, AppError> {
    use axum::response::sse::{Event, KeepAlive, Sse};
    use futures_util::StreamExt;

    let tail = query.tail.unwrap_or(DEFAULT_LOG_TAIL);
    if tail == 0 || tail > MAX_LOG_TAIL {
        return Err(AppError::validation_single(
            "tail",
            format!("1〜{MAX_LOG_TAIL}の範囲で指定してください"),
        ));
    }

    let detail = service::load_detail(&state, &id_or_name).await?;
    let target = match query.container {
        Some(container_id) => detail
            .containers
            .iter()
            .find(|c| c.container.id == container_id)
            .ok_or_else(|| {
                AppError::NotFound(format!(
                    "コンテナ {container_id} はサービス '{}' に属していません",
                    detail.service.name
                ))
            })?,
        None => detail.containers.first().ok_or_else(|| {
            AppError::NotFound(format!(
                "サービス '{}' にコンテナがありません",
                detail.service.name
            ))
        })?,
    };

    let container_name = sahai_core::naming::container_docker_name(target.container.id);
    let stream =
        crate::docker::log_stream::stream_logs(&state.docker.docker, &container_name, tail).map(
            |result| {
                let event = match result {
                    Ok(line) => Event::default().event("line").json_data(LogLineDto {
                        stream: line.stream.as_str(),
                        timestamp: line.timestamp,
                        message: line.message,
                    }),
                    // 読み出せなくなった理由を画面に出す。コンテナが未作成・削除済みの場合が
                    // 大半で、利用者にとっては「まだ起動していない」ことの手掛かりになる
                    Err(e) => Event::default()
                        .event("error")
                        .json_data(serde_json::json!({ "message": describe_log_error(&e) })),
                };
                event.map_err(|e| std::io::Error::other(e.to_string()))
            },
        );

    // Traefik経由でも切られないよう定期的にコメント行を送る。ログが何も出ない
    // コンテナでは無通信のまま時間が過ぎるため、これが無いと接続が落ちる
    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

/// ログ読み出しの失敗理由を利用者向けの文言にする。
///
/// コンテナが無い(404)のは異常ではなく、未起動か停止済みというだけ。しかも差配は
/// 停止時にコンテナごと削除するため(docker::image_runtime::stop参照)、停止した
/// サービスの過去のログは残らない。bollardの英語メッセージをそのまま出すと
/// この事情が伝わらない。
fn describe_log_error(error: &crate::docker::DockerError) -> String {
    let is_not_found = matches!(
        error,
        crate::docker::DockerError::Bollard(bollard::errors::Error::DockerResponseServerError {
            status_code: 404,
            ..
        })
    );
    if is_not_found {
        "コンテナがありません(未起動、または停止済み)。停止時にコンテナごと削除するため、停止したサービスの過去のログは残りません".to_string()
    } else {
        error.to_string()
    }
}

#[derive(Serialize)]
struct RegistryStatusResponse {
    containers: Vec<ContainerRegistryDto>,
}

#[derive(Serialize)]
struct ContainerRegistryDto {
    id: i64,
    name: String,
    image_tag: String,
    /// registry_urlのローカルイメージキャッシュ(Dockerホスト)に存在するか。
    /// レジストリのHTTP APIへは問い合わせない(inspector.rs::image_exists参照)
    image_present: bool,
}

/// コンテナが実際に起動時に参照するイメージタグを計算する(override_gen.rsの
/// generate_override_yamlと同じ命名規則。純粋関数なのでDocker daemon不要でテストできる)。
fn expected_image_tag(
    service: &crate::domain::Service,
    container_name: &str,
    registry_url: &str,
) -> String {
    match service.source_type {
        crate::domain::SourceType::Image => service.image.clone().unwrap_or_default(),
        crate::domain::SourceType::Compose => {
            let tag = sahai_core::naming::registry_tag_name(&service.name, Some(container_name));
            format!("{registry_url}/{tag}:latest")
        }
    }
}

pub async fn registry_status(
    State(state): State<AppState>,
    Path(id_or_name): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let detail = service::load_detail(&state, &id_or_name).await?;

    let registry_url = state.settings.read().await.registry_url.clone();
    let mut containers = Vec::with_capacity(detail.containers.len());
    for c in &detail.containers {
        let image_tag = expected_image_tag(&detail.service, &c.container.name, &registry_url);
        let image_present = state
            .docker
            .inspector
            .image_exists(&image_tag)
            .await
            .unwrap_or(false);
        containers.push(ContainerRegistryDto {
            id: c.container.id,
            name: c.container.name.clone(),
            image_tag,
            image_present,
        });
    }
    Ok(Json(RegistryStatusResponse { containers }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{Service, ServiceStatus, SourceType};

    fn service(source_type: SourceType, image: Option<&str>) -> Service {
        Service {
            id: 1,
            name: "myapp".to_string(),
            subdomain: "myapp.example.com".to_string(),
            source_type,
            image: image.map(str::to_string),
            compose_content: None,
            env_vars: serde_json::json!({}),
            status: ServiceStatus::Stopped,
            last_error: None,
            health_status: crate::domain::HealthStatus::Unknown,
            last_health_check_at: None,
            created_at: "2026-01-01T00:00:00.000Z".to_string(),
            updated_at: "2026-01-01T00:00:00.000Z".to_string(),
        }
    }

    #[test]
    fn expected_image_tag_uses_stored_image_field_for_image_type() {
        let svc = service(
            SourceType::Image,
            Some("registry.sahai.example.com/myapp:latest"),
        );
        assert_eq!(
            expected_image_tag(&svc, "myapp", "registry.sahai.example.com"),
            "registry.sahai.example.com/myapp:latest"
        );
    }

    #[test]
    fn expected_image_tag_derives_from_registry_url_for_compose_type() {
        let svc = service(SourceType::Compose, None);
        assert_eq!(
            expected_image_tag(&svc, "frontend", "registry.sahai.example.com"),
            "registry.sahai.example.com/myapp-frontend:latest"
        );
    }
}
