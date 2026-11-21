//! HTTP API router for the VRTc Search Engine.
//!
//! The router is designed to be embedded in an operator-owned server. It does
//! not start a listener, provision credentials, or register providers itself.

use std::sync::Arc;

use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Serialize;

use vrtc_core::{ErrorCategory, SearchRequest, SearchResponse, VrtCError};
use vrtc_search::SearchEngine;

#[derive(Clone)]
struct ApiState {
    search: Arc<SearchEngine>,
}

/// Build the VRTc HTTP router around a configured Search Engine.
pub fn router(search: Arc<SearchEngine>) -> Router {
    Router::new()
        .route("/v1/search", post(search_endpoint))
        .route("/v1/providers", get(providers_endpoint))
        .route("/v1/health", get(health_endpoint))
        .with_state(ApiState { search })
}

/// Search request endpoint. Provider failures are returned inside a successful
/// response when at least the orchestration itself completed.
async fn search_endpoint(
    State(state): State<ApiState>,
    Json(request): Json<SearchRequest>,
) -> Result<Json<SearchResponse>, ApiError> {
    state.search.search(request).await.map(Json).map_err(ApiError)
}

async fn providers_endpoint(State(state): State<ApiState>) -> Json<ProvidersResponse> {
    Json(ProvidersResponse {
        providers: state.search.provider_ids(),
    })
}

async fn health_endpoint() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        service: "vrtc-search",
    })
}

#[derive(Debug, Serialize)]
struct ProvidersResponse {
    providers: Vec<String>,
}

#[derive(Debug, Serialize)]
struct HealthResponse {
    status: &'static str,
    service: &'static str,
}

struct ApiError(VrtCError);

impl axum::response::IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        let status = match self.0.category {
            ErrorCategory::Validation => StatusCode::BAD_REQUEST,
            ErrorCategory::NotRegistered => StatusCode::SERVICE_UNAVAILABLE,
            ErrorCategory::ResourceExhausted => StatusCode::TOO_MANY_REQUESTS,
            ErrorCategory::Timeout => StatusCode::GATEWAY_TIMEOUT,
            ErrorCategory::Disabled | ErrorCategory::PolicyDenied => StatusCode::FORBIDDEN,
            ErrorCategory::NotFound => StatusCode::NOT_FOUND,
            ErrorCategory::DuplicateId | ErrorCategory::ContractIncompatible => {
                StatusCode::CONFLICT
            }
            ErrorCategory::LifecycleInvalid => StatusCode::UNPROCESSABLE_ENTITY,
            ErrorCategory::HealthCheckFailed | ErrorCategory::Internal => {
                StatusCode::INTERNAL_SERVER_ERROR
            }
            ErrorCategory::Cancelled => StatusCode::REQUEST_TIMEOUT,
        };

        (status, Json(self.0)).into_response()
    }
}
