use axum::{
    Json,
    http::{HeaderName, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
};
use forgequeue_core::ProblemDetails;
use thiserror::Error;
use uuid::Uuid;

tokio::task_local! {
    pub static REQUEST_ID: String;
}

#[derive(Debug, Error)]
pub enum AppError {
    #[error("{0}")]
    Unauthorized(String),
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    Conflict(String),
    #[error("{0}")]
    Validation(String),
    #[error("{0}")]
    RateLimited(String),
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    #[error(transparent)]
    Storage(#[from] object_store::Error),
    #[error(transparent)]
    Internal(#[from] anyhow::Error),
}

impl AppError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Unauthorized(_) => "unauthorized",
            Self::NotFound(_) => "not_found",
            Self::Conflict(_) => "conflict",
            Self::Validation(_) => "validation_error",
            Self::RateLimited(_) => "rate_limited",
            Self::Database(_) => "database_error",
            Self::Storage(_) => "storage_error",
            Self::Internal(_) => "internal_error",
        }
    }

    fn status(&self) -> StatusCode {
        match self {
            Self::Unauthorized(_) => StatusCode::UNAUTHORIZED,
            Self::NotFound(_) => StatusCode::NOT_FOUND,
            Self::Conflict(_) => StatusCode::CONFLICT,
            Self::Validation(_) => StatusCode::UNPROCESSABLE_ENTITY,
            Self::RateLimited(_) => StatusCode::TOO_MANY_REQUESTS,
            Self::Database(_) | Self::Storage(_) | Self::Internal(_) => {
                StatusCode::INTERNAL_SERVER_ERROR
            }
        }
    }

    fn title(&self) -> &'static str {
        match self {
            Self::Unauthorized(_) => "Autenticación requerida",
            Self::NotFound(_) => "Recurso no encontrado",
            Self::Conflict(_) => "Conflicto de estado",
            Self::Validation(_) => "Entrada inválida",
            Self::RateLimited(_) => "Límite de uso alcanzado",
            Self::Database(_) | Self::Storage(_) | Self::Internal(_) => "Error interno",
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let status = self.status();
        let code = self.code();
        let detail = if status.is_server_error() {
            tracing::error!(error = %self, code, "request failed");
            "El servidor no pudo completar la solicitud.".to_owned()
        } else {
            self.to_string()
        };
        let request_id = REQUEST_ID
            .try_with(Clone::clone)
            .unwrap_or_else(|_| Uuid::new_v4().to_string());
        let problem = ProblemDetails {
            problem_type: format!("https://forgequeue.dev/problems/{code}"),
            title: self.title().to_owned(),
            status: status.as_u16(),
            code: code.to_owned(),
            detail,
            request_id: request_id.clone(),
        };

        let mut response = (status, Json(problem)).into_response();
        response.headers_mut().insert(
            HeaderName::from_static("x-request-id"),
            HeaderValue::from_str(&request_id).expect("UUID request id is a valid header"),
        );
        response
    }
}

pub type AppResult<T> = Result<T, AppError>;

#[cfg(test)]
mod tests {
    use axum::{body::to_bytes, response::IntoResponse};
    use forgequeue_core::ProblemDetails;

    use super::{AppError, REQUEST_ID};

    #[tokio::test]
    async fn problem_details_and_header_share_the_request_id() {
        REQUEST_ID
            .scope("request-test-123".to_owned(), async {
                let response = AppError::Validation("entrada inválida".to_owned()).into_response();
                assert_eq!(
                    response.headers().get("x-request-id").unwrap(),
                    "request-test-123"
                );
                let problem: ProblemDetails = serde_json::from_slice(
                    &to_bytes(response.into_body(), 16 * 1024).await.unwrap(),
                )
                .unwrap();
                assert_eq!(problem.request_id, "request-test-123");
                assert_eq!(problem.code, "validation_error");
            })
            .await;
    }
}
