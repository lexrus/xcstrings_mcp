use std::{collections::BTreeMap, net::SocketAddr, sync::Arc};

use axum::{
    extract::{Path, Query},
    http::StatusCode,
    response::{Html, IntoResponse},
    routing::{delete, get, post},
    Extension, Json, Router,
};
use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;
use tracing::info;

use crate::store::{
    StoreError, TranslationRecord, TranslationUpdate, TranslationValue, XcStringsStore,
};

#[derive(Debug, Serialize)]
struct ErrorResponse {
    error: String,
}

#[derive(Debug, Deserialize)]
struct ListQuery {
    q: Option<String>,
}

#[derive(Debug, Serialize)]
struct TranslationsResponse {
    items: Vec<TranslationRecord>,
}

#[derive(Debug, Serialize)]
struct LanguagesResponse {
    languages: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct UpsertRequest {
    key: String,
    language: String,
    #[serde(default)]
    value: Option<Option<String>>,
    #[serde(default)]
    state: Option<Option<String>>,
    #[serde(default)]
    variations: Option<BTreeMap<String, BTreeMap<String, VariationUpdatePayload>>>,
}

#[derive(Debug, Deserialize, Clone)]
struct VariationUpdatePayload {
    #[serde(default)]
    value: Option<Option<String>>,
    #[serde(default)]
    state: Option<Option<String>>,
    #[serde(default)]
    variations: Option<BTreeMap<String, BTreeMap<String, VariationUpdatePayload>>>,
}

impl VariationUpdatePayload {
    fn into_update(self) -> TranslationUpdate {
        let mut update = TranslationUpdate::default();
        update.value = self.value;
        update.state = self.state;
        if let Some(variations) = self.variations {
            update.variations = Some(
                variations
                    .into_iter()
                    .map(|(selector, cases)| {
                        let cases = cases
                            .into_iter()
                            .map(|(case, nested)| (case, nested.into_update()))
                            .collect();
                        (selector, cases)
                    })
                    .collect(),
            );
        }
        update
    }
}

impl UpsertRequest {
    fn into_update(self) -> TranslationUpdate {
        let mut update = TranslationUpdate::default();
        update.value = self.value;
        update.state = self.state;
        if let Some(variations) = self.variations {
            update.variations = Some(
                variations
                    .into_iter()
                    .map(|(selector, cases)| {
                        let cases = cases
                            .into_iter()
                            .map(|(case, nested)| (case, nested.into_update()))
                            .collect();
                        (selector, cases)
                    })
                    .collect(),
            );
        }
        update
    }
}

#[derive(Debug, Deserialize)]
struct CommentRequest {
    key: String,
    comment: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RenameKeyRequest {
    new_key: String,
}

pub fn router(store: Arc<XcStringsStore>) -> Router {
    Router::new()
        .route("/", get(index))
        .route(
            "/api/translations",
            get(list_translations).put(upsert_translation),
        )
        .route(
            "/api/translations/:key/:language",
            delete(delete_translation),
        )
        .route("/api/keys/:key", delete(delete_key).put(rename_key))
        .route("/api/comments", post(update_comment))
        .route("/api/languages", get(list_languages))
        .layer(Extension(store))
}

pub async fn serve(addr: SocketAddr, store: Arc<XcStringsStore>) -> anyhow::Result<()> {
    let app = router(store);
    info!(%addr, "Starting web UI");
    let listener = TcpListener::bind(addr).await?;
    axum::serve(listener, app.into_make_service()).await?;
    Ok(())
}

async fn index() -> Html<&'static str> {
    Html(INDEX_HTML)
}

async fn list_translations(
    Extension(store): Extension<Arc<XcStringsStore>>,
    Query(query): Query<ListQuery>,
) -> Result<Json<TranslationsResponse>, ApiError> {
    let items = store.list_records(query.q.as_deref()).await;
    Ok(Json(TranslationsResponse { items }))
}

async fn list_languages(
    Extension(store): Extension<Arc<XcStringsStore>>,
) -> Result<Json<LanguagesResponse>, ApiError> {
    let languages = store.list_languages().await;
    Ok(Json(LanguagesResponse { languages }))
}

async fn upsert_translation(
    Extension(store): Extension<Arc<XcStringsStore>>,
    Json(payload): Json<UpsertRequest>,
) -> Result<Json<TranslationValue>, ApiError> {
    let key = payload.key.clone();
    let language = payload.language.clone();
    let update = payload.into_update();
    let value = store
        .upsert_translation(&key, &language, update)
        .await
        .map_err(ApiError::from)?;
    Ok(Json(value))
}

async fn delete_translation(
    Extension(store): Extension<Arc<XcStringsStore>>,
    Path((key, language)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    store
        .delete_translation(&key, &language)
        .await
        .map_err(ApiError::from)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn delete_key(
    Extension(store): Extension<Arc<XcStringsStore>>,
    Path(key): Path<String>,
) -> Result<StatusCode, ApiError> {
    store.delete_key(&key).await.map_err(ApiError::from)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn update_comment(
    Extension(store): Extension<Arc<XcStringsStore>>,
    Json(payload): Json<CommentRequest>,
) -> Result<StatusCode, ApiError> {
    store
        .set_comment(&payload.key, payload.comment.clone())
        .await
        .map_err(ApiError::from)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn rename_key(
    Extension(store): Extension<Arc<XcStringsStore>>,
    Path(old_key): Path<String>,
    Json(payload): Json<RenameKeyRequest>,
) -> Result<StatusCode, ApiError> {
    let new_key = payload.new_key.trim();
    if new_key.is_empty() {
        return Err(ApiError {
            status: StatusCode::BAD_REQUEST,
            message: "New key must not be empty".to_string(),
        });
    }

    store
        .rename_key(&old_key, new_key)
        .await
        .map_err(ApiError::from)?;

    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug)]
struct ApiError {
    status: StatusCode,
    message: String,
}

impl From<StoreError> for ApiError {
    fn from(value: StoreError) -> Self {
        let status = match value {
            StoreError::TranslationMissing { .. } => StatusCode::NOT_FOUND,
            StoreError::KeyMissing(_) => StatusCode::NOT_FOUND,
            StoreError::KeyExists(_) => StatusCode::CONFLICT,
            StoreError::SerdeFailed(_) | StoreError::ReadFailed(_) => {
                StatusCode::INTERNAL_SERVER_ERROR
            }
            StoreError::PathRequired => StatusCode::BAD_REQUEST,
        };
        ApiError {
            status,
            message: value.to_string(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        let status = self.status;
        let body = Json(ErrorResponse {
            error: self.message,
        });
        (status, body).into_response()
    }
}

const INDEX_HTML: &str = include_str!("index.html");
