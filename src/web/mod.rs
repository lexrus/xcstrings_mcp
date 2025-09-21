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
    StoreError, SubstitutionUpdate, TranslationRecord, TranslationUpdate, TranslationValue,
    XcStringsStore, XcStringsStoreManager,
};

#[derive(Debug, Serialize)]
struct ErrorResponse {
    error: String,
}

#[derive(Debug, Deserialize)]
struct ListQuery {
    q: Option<String>,
    #[serde(default)]
    path: Option<String>,
}

#[derive(Debug, Serialize)]
struct TranslationsResponse {
    items: Vec<TranslationRecord>,
}

#[derive(Debug, Serialize)]
struct FileEntryResponse {
    path: String,
    label: String,
}

#[derive(Debug, Serialize)]
struct FilesResponse {
    files: Vec<FileEntryResponse>,
    default: Option<String>,
}

#[derive(Debug, Serialize)]
struct LanguagesResponse {
    languages: Vec<String>,
}

#[derive(Debug, Deserialize, Default)]
struct PathQuery {
    #[serde(default)]
    path: Option<String>,
}

#[derive(Debug, Deserialize)]
struct UpsertRequest {
    key: String,
    language: String,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    value: Option<Option<String>>,
    #[serde(default)]
    state: Option<Option<String>>,
    #[serde(default)]
    variations: Option<BTreeMap<String, BTreeMap<String, VariationUpdatePayload>>>,
    #[serde(default)]
    substitutions: Option<BTreeMap<String, Option<SubstitutionUpdatePayload>>>,
}

#[derive(Debug, Deserialize, Clone)]
struct VariationUpdatePayload {
    #[serde(default)]
    value: Option<Option<String>>,
    #[serde(default)]
    state: Option<Option<String>>,
    #[serde(default)]
    variations: Option<BTreeMap<String, BTreeMap<String, VariationUpdatePayload>>>,
    #[serde(default)]
    substitutions: Option<BTreeMap<String, Option<SubstitutionUpdatePayload>>>,
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
        if let Some(substitutions) = self.substitutions {
            update.substitutions = Some(
                substitutions
                    .into_iter()
                    .map(|(name, payload)| (name, payload.map(|value| value.into_update())))
                    .collect(),
            );
        }
        update
    }
}

#[derive(Debug, Deserialize, Clone)]
struct SubstitutionUpdatePayload {
    #[serde(default)]
    value: Option<Option<String>>,
    #[serde(default)]
    state: Option<Option<String>>,
    #[serde(rename = "argNum", default)]
    arg_num: Option<Option<i64>>,
    #[serde(rename = "formatSpecifier", default)]
    format_specifier: Option<Option<String>>,
    #[serde(default)]
    variations: Option<BTreeMap<String, BTreeMap<String, VariationUpdatePayload>>>,
}

impl SubstitutionUpdatePayload {
    fn into_update(self) -> SubstitutionUpdate {
        let mut update = SubstitutionUpdate::default();
        update.value = self.value;
        update.state = self.state;
        update.arg_num = self.arg_num;
        update.format_specifier = self.format_specifier;
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
        if let Some(substitutions) = self.substitutions {
            update.substitutions = Some(
                substitutions
                    .into_iter()
                    .map(|(name, payload)| (name, payload.map(|value| value.into_update())))
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
    #[serde(default)]
    path: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RenameKeyRequest {
    new_key: String,
    #[serde(default)]
    path: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ExtractionStateRequest {
    key: String,
    #[serde(rename = "extractionState", default)]
    extraction_state: Option<String>,
    #[serde(default)]
    path: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ShouldTranslateRequest {
    key: String,
    #[serde(rename = "shouldTranslate", default)]
    should_translate: Option<bool>,
    #[serde(default)]
    path: Option<String>,
}

pub fn router(manager: Arc<XcStringsStoreManager>) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/api/files", get(list_files))
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
        .route("/api/extraction-state", post(update_extraction_state))
        .route("/api/should-translate", post(update_should_translate))
        .route("/api/languages", get(list_languages))
        .layer(Extension(manager))
}

pub async fn serve(addr: SocketAddr, manager: Arc<XcStringsStoreManager>) -> anyhow::Result<()> {
    let app = router(manager);
    info!(%addr, "Starting web UI");
    let listener = TcpListener::bind(addr).await?;
    axum::serve(listener, app.into_make_service()).await?;
    Ok(())
}

fn path_token(manager: &XcStringsStoreManager, path: &std::path::Path) -> String {
    if let Ok(relative) = path.strip_prefix(manager.search_root()) {
        let display = relative.to_string_lossy();
        if !display.is_empty() {
            return display.replace('\\', "/");
        }
    }
    path.to_string_lossy().replace('\\', "/")
}

fn path_label(manager: &XcStringsStoreManager, path: &std::path::Path) -> String {
    if let Ok(relative) = path.strip_prefix(manager.search_root()) {
        let text = relative.to_string_lossy();
        if !text.is_empty() {
            return text.replace('\\', "/");
        }
    }
    path.file_name()
        .and_then(|value| value.to_str())
        .map(|value| value.to_string())
        .unwrap_or_else(|| path.to_string_lossy().replace('\\', "/"))
}

async fn resolve_store(
    manager: &XcStringsStoreManager,
    path: Option<&str>,
) -> Result<Arc<XcStringsStore>, ApiError> {
    manager.store_for(path).await.map_err(ApiError::from)
}

async fn index() -> Html<&'static str> {
    Html(INDEX_HTML)
}

async fn list_files(
    Extension(manager): Extension<Arc<XcStringsStoreManager>>,
) -> Result<Json<FilesResponse>, ApiError> {
    let paths = manager.refresh_discovered_paths().await?;
    let files = paths
        .iter()
        .map(|path| FileEntryResponse {
            path: path_token(manager.as_ref(), path),
            label: path_label(manager.as_ref(), path),
        })
        .collect();
    let default = manager
        .default_path()
        .as_ref()
        .map(|path| path_token(manager.as_ref(), path));

    Ok(Json(FilesResponse { files, default }))
}

async fn list_translations(
    Extension(manager): Extension<Arc<XcStringsStoreManager>>,
    Query(query): Query<ListQuery>,
) -> Result<Json<TranslationsResponse>, ApiError> {
    let store = resolve_store(manager.as_ref(), query.path.as_deref()).await?;
    let items = store.list_records(query.q.as_deref()).await;
    Ok(Json(TranslationsResponse { items }))
}

async fn list_languages(
    Extension(manager): Extension<Arc<XcStringsStoreManager>>,
    Query(query): Query<PathQuery>,
) -> Result<Json<LanguagesResponse>, ApiError> {
    let store = resolve_store(manager.as_ref(), query.path.as_deref()).await?;
    let languages = store.list_languages().await;
    Ok(Json(LanguagesResponse { languages }))
}

async fn upsert_translation(
    Extension(manager): Extension<Arc<XcStringsStoreManager>>,
    Json(payload): Json<UpsertRequest>,
) -> Result<Json<TranslationValue>, ApiError> {
    let path = payload.path.clone();
    let key = payload.key.clone();
    let language = payload.language.clone();
    let update = payload.into_update();
    let store = resolve_store(manager.as_ref(), path.as_deref()).await?;
    let value = store
        .upsert_translation(&key, &language, update)
        .await
        .map_err(ApiError::from)?;
    Ok(Json(value))
}

async fn delete_translation(
    Extension(manager): Extension<Arc<XcStringsStoreManager>>,
    Path((key, language)): Path<(String, String)>,
    Query(query): Query<PathQuery>,
) -> Result<StatusCode, ApiError> {
    let store = resolve_store(manager.as_ref(), query.path.as_deref()).await?;
    store
        .delete_translation(&key, &language)
        .await
        .map_err(ApiError::from)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn delete_key(
    Extension(manager): Extension<Arc<XcStringsStoreManager>>,
    Path(key): Path<String>,
    Query(query): Query<PathQuery>,
) -> Result<StatusCode, ApiError> {
    let store = resolve_store(manager.as_ref(), query.path.as_deref()).await?;
    store.delete_key(&key).await.map_err(ApiError::from)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn update_comment(
    Extension(manager): Extension<Arc<XcStringsStoreManager>>,
    Json(payload): Json<CommentRequest>,
) -> Result<StatusCode, ApiError> {
    let path = payload.path.clone();
    let store = resolve_store(manager.as_ref(), path.as_deref()).await?;
    store
        .set_comment(&payload.key, payload.comment.clone())
        .await
        .map_err(ApiError::from)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn update_extraction_state(
    Extension(manager): Extension<Arc<XcStringsStoreManager>>,
    Json(payload): Json<ExtractionStateRequest>,
) -> Result<StatusCode, ApiError> {
    let path = payload.path.clone();
    let store = resolve_store(manager.as_ref(), path.as_deref()).await?;
    store
        .set_extraction_state(&payload.key, payload.extraction_state.clone())
        .await
        .map_err(ApiError::from)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn update_should_translate(
    Extension(manager): Extension<Arc<XcStringsStoreManager>>,
    Json(payload): Json<ShouldTranslateRequest>,
) -> Result<StatusCode, ApiError> {
    let path = payload.path.clone();
    let store = resolve_store(manager.as_ref(), path.as_deref()).await?;
    store
        .set_should_translate(&payload.key, payload.should_translate)
        .await
        .map_err(ApiError::from)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn rename_key(
    Extension(manager): Extension<Arc<XcStringsStoreManager>>,
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

    let path = payload.path.clone();
    let store = resolve_store(manager.as_ref(), path.as_deref()).await?;

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
