//! 全レイヤー共通のエラー型。HTTPステータスとエラーコードへの変換をここに集約し、
//! 各レイヤーがHTTPを意識しなくて済むようにする。

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::Serialize;

#[derive(Debug)]
pub struct FieldError {
    pub field: String,
    pub message: String,
}

#[derive(Debug)]
pub enum AppError {
    Unauthorized,
    NotFound(String),
    Validation(Vec<FieldError>),
    Conflict(String),
    Unprocessable(String),
    Internal(String),
    BuildFailed(String),
}

impl AppError {
    pub fn validation_single(field: impl Into<String>, message: impl Into<String>) -> Self {
        AppError::Validation(vec![FieldError {
            field: field.into(),
            message: message.into(),
        }])
    }

    fn code(&self) -> &'static str {
        match self {
            AppError::Unauthorized => "UNAUTHORIZED",
            AppError::NotFound(_) => "NOT_FOUND",
            AppError::Validation(_) => "VALIDATION_ERROR",
            AppError::Conflict(_) => "CONFLICT",
            AppError::Unprocessable(_) => "UNPROCESSABLE",
            AppError::Internal(_) => "INTERNAL_ERROR",
            AppError::BuildFailed(_) => "BUILD_FAILED",
        }
    }

    fn status(&self) -> StatusCode {
        match self {
            AppError::Unauthorized => StatusCode::UNAUTHORIZED,
            AppError::NotFound(_) => StatusCode::NOT_FOUND,
            AppError::Validation(_) => StatusCode::BAD_REQUEST,
            AppError::Conflict(_) => StatusCode::CONFLICT,
            AppError::Unprocessable(_) => StatusCode::UNPROCESSABLE_ENTITY,
            AppError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
            AppError::BuildFailed(_) => StatusCode::UNPROCESSABLE_ENTITY,
        }
    }

    fn message(&self) -> String {
        match self {
            AppError::Unauthorized => "認証に失敗しました".to_string(),
            AppError::NotFound(name) => format!("サービス '{name}' が見つかりません"),
            AppError::Validation(_) => "入力内容に誤りがあります".to_string(),
            AppError::Conflict(msg) => msg.clone(),
            AppError::Unprocessable(msg) => msg.clone(),
            AppError::Internal(msg) => msg.clone(),
            AppError::BuildFailed(msg) => format!("ビルドまたはpushに失敗しました: {msg}"),
        }
    }
}

#[derive(Serialize)]
struct ErrorFieldDto {
    field: String,
    message: String,
}

#[derive(Serialize)]
struct ErrorBodyDto {
    code: &'static str,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    fields: Option<Vec<ErrorFieldDto>>,
}

#[derive(Serialize)]
struct ErrorResponseDto {
    error: ErrorBodyDto,
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let status = self.status();
        let code = self.code();
        let message = self.message();
        let fields = if let AppError::Validation(fields) = &self {
            Some(
                fields
                    .iter()
                    .map(|f| ErrorFieldDto {
                        field: f.field.clone(),
                        message: f.message.clone(),
                    })
                    .collect(),
            )
        } else {
            None
        };

        (
            status,
            Json(ErrorResponseDto {
                error: ErrorBodyDto {
                    code,
                    message,
                    fields,
                },
            }),
        )
            .into_response()
    }
}

impl From<sqlx::Error> for AppError {
    fn from(err: sqlx::Error) -> Self {
        match &err {
            sqlx::Error::RowNotFound => AppError::NotFound("(id)".to_string()),
            sqlx::Error::Database(db_err) if db_err.is_unique_violation() => {
                AppError::Conflict(format!("一意制約違反: {db_err}"))
            }
            _ => AppError::Internal(err.to_string()),
        }
    }
}

impl From<sahai_core::CoreError> for AppError {
    fn from(err: sahai_core::CoreError) -> Self {
        AppError::validation_single("_", err.to_string())
    }
}

#[cfg(test)]
mod tests {
    use axum::body::to_bytes;

    use super::*;

    async fn body_json(err: AppError) -> (StatusCode, serde_json::Value) {
        let response = err.into_response();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        (status, json)
    }

    #[tokio::test]
    async fn unauthorized_maps_to_401() {
        let (status, json) = body_json(AppError::Unauthorized).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(json["error"]["code"], "UNAUTHORIZED");
        assert!(json["error"]["fields"].is_null());
    }

    #[tokio::test]
    async fn not_found_maps_to_404_and_includes_id_in_message() {
        let (status, json) = body_json(AppError::NotFound("myapp".to_string())).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(json["error"]["code"], "NOT_FOUND");
        assert!(json["error"]["message"].as_str().unwrap().contains("myapp"));
    }

    #[tokio::test]
    async fn validation_maps_to_400_and_includes_fields_array() {
        let (status, json) = body_json(AppError::Validation(vec![
            FieldError {
                field: "name".to_string(),
                message: "bad name".to_string(),
            },
            FieldError {
                field: "containers[].ports[].host_port".to_string(),
                message: "out of range".to_string(),
            },
        ]))
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(json["error"]["code"], "VALIDATION_ERROR");
        let fields = json["error"]["fields"].as_array().unwrap();
        assert_eq!(fields.len(), 2);
        assert_eq!(fields[0]["field"], "name");
        assert_eq!(fields[0]["message"], "bad name");
        assert_eq!(fields[1]["field"], "containers[].ports[].host_port");
    }

    #[tokio::test]
    async fn conflict_maps_to_409() {
        let (status, json) = body_json(AppError::Conflict("name重複".to_string())).await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(json["error"]["code"], "CONFLICT");
        assert_eq!(json["error"]["message"], "name重複");
    }

    #[tokio::test]
    async fn unprocessable_maps_to_422() {
        let (status, json) = body_json(AppError::Unprocessable("矛盾".to_string())).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(json["error"]["code"], "UNPROCESSABLE");
    }

    #[tokio::test]
    async fn internal_maps_to_500() {
        let (status, json) = body_json(AppError::Internal("boom".to_string())).await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(json["error"]["code"], "INTERNAL_ERROR");
    }

    #[tokio::test]
    async fn build_failed_maps_to_422_and_includes_docker_stderr() {
        let (status, json) = body_json(AppError::BuildFailed(
            "no such file: Dockerfile".to_string(),
        ))
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(json["error"]["code"], "BUILD_FAILED");
        assert!(json["error"]["message"]
            .as_str()
            .unwrap()
            .contains("no such file: Dockerfile"));
    }

    #[test]
    fn sqlx_row_not_found_becomes_not_found() {
        let err: AppError = sqlx::Error::RowNotFound.into();
        assert!(matches!(err, AppError::NotFound(_)));
    }

    #[test]
    fn core_validation_error_becomes_validation() {
        let core_err = sahai_core::CoreError::Validation("bad".to_string());
        let err: AppError = core_err.into();
        assert!(matches!(err, AppError::Validation(_)));
    }
}
