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
        .route("/api/keys/:key", delete(delete_key))
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

const INDEX_HTML: &str = r#"
<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="utf-8" />
    <title>xcstrings Translations</title>
    <style>
        body { font-family: Arial, sans-serif; margin: 2rem; background: #f4f4f9; color: #222; }
        h1 { margin-bottom: 1rem; }
        table { border-collapse: collapse; width: 100%; margin-top: 1.5rem; background: #fff; }
        th, td { border: 1px solid #ccc; padding: 0.5rem; text-align: left; }
        th { background: #eee; }
        input[type="text"], select { padding: 0.5rem; margin-right: 0.5rem; }
        .toolbar { display: flex; flex-wrap: wrap; gap: 0.5rem; align-items: center; }
        .status { margin-top: 1rem; color: #0a7; }
        .error { color: #c00; }
        textarea { width: 100%; }
        button { padding: 0.4rem 0.8rem; }
        details.plural-container { margin-top: 0.5rem; background: #f8f8fb; padding: 0.5rem; border: 1px solid #ddd; border-radius: 4px; }
        details.plural-container > summary { cursor: pointer; font-weight: 600; }
        .plural-row { display: flex; gap: 0.5rem; align-items: center; margin-top: 0.25rem; }
        .plural-row label { min-width: 4rem; font-weight: 600; }
        .plural-row textarea { flex: 1; }
        .plural-add { display: flex; gap: 0.5rem; align-items: center; margin-top: 0.5rem; flex-wrap: wrap; }
        .plural-add input { width: 6rem; padding: 0.4rem; }
        .plural-empty { font-style: italic; color: #555; margin-top: 0.25rem; }
    </style>
</head>
<body>
    <h1>Localizable.xcstrings Browser</h1>
    <div class="toolbar">
        <input type="text" id="search" placeholder="Search keys or translations" />
        <button id="refresh">Refresh</button>
        <select id="language-select"></select>
        <input type="text" id="new-key" placeholder="New key" />
        <textarea id="new-value" rows="1" placeholder="New translation"></textarea>
        <button id="create">Create / Update</button>
    </div>
    <div id="status" class="status"></div>
    <table>
        <thead>
            <tr>
                <th>Key</th>
                <th>Comment</th>
                <th id="language-header">Translations</th>
                <th>Actions</th>
            </tr>
        </thead>
        <tbody id="translations-body"></tbody>
    </table>

    <script>
        const state = {
            languages: [],
            currentLanguage: null,
            items: [],
        };

        async function fetchLanguages() {
            const res = await fetch('/api/languages');
            const data = await res.json();
            state.languages = data.languages;
            const select = document.getElementById('language-select');
            select.innerHTML = '';
            state.languages.forEach(lang => {
                const option = document.createElement('option');
                option.value = lang;
                option.textContent = lang;
                select.appendChild(option);
            });
            if (!state.currentLanguage && state.languages.length) {
                state.currentLanguage = state.languages[0];
            }
            select.value = state.currentLanguage;
            document.getElementById('language-header').textContent = `Translation (${state.currentLanguage})`;
        }

        async function fetchTranslations(query = '') {
            const params = query ? `?q=${encodeURIComponent(query)}` : '';
            const res = await fetch(`/api/translations${params}`);
            const data = await res.json();
            state.items = data.items;
            renderTable();
        }

        function renderTable() {
            const tbody = document.getElementById('translations-body');
            tbody.innerHTML = '';
            state.items.forEach(item => {
                const tr = document.createElement('tr');

                const keyCell = document.createElement('td');
                keyCell.textContent = item.key;
                tr.appendChild(keyCell);

                const commentCell = document.createElement('td');
                const commentInput = document.createElement('input');
                commentInput.type = 'text';
                commentInput.value = item.comment || '';
                commentInput.addEventListener('change', async (event) => {
                    await fetch('/api/comments', {
                        method: 'POST',
                        headers: { 'Content-Type': 'application/json' },
                        body: JSON.stringify({ key: item.key, comment: event.target.value || null })
                    });
                    setStatus('Comment saved');
                });
                commentCell.appendChild(commentInput);
                tr.appendChild(commentCell);

                const translationCell = document.createElement('td');
                const translation = item.translations[state.currentLanguage] || {};
                const textarea = document.createElement('textarea');
                textarea.rows = 1;
                textarea.value = translation.value || '';
                textarea.addEventListener('change', async (event) => {
                    try {
                        await upsertTranslation({
                            key: item.key,
                            language: state.currentLanguage,
                            value: event.target.value
                        });
                    } catch (error) {
                        setStatus(error.message, true);
                    }
                });
                translationCell.appendChild(textarea);

                const pluralDetails = document.createElement('details');
                pluralDetails.className = 'plural-container';
                const pluralSummary = document.createElement('summary');
                pluralSummary.textContent = 'Plural forms';
                pluralDetails.appendChild(pluralSummary);

                const pluralContent = document.createElement('div');
                const pluralVariations = (translation.variations && translation.variations.plural) || {};
                const pluralKeys = Object.keys(pluralVariations);
                if (pluralKeys.length === 0) {
                    const empty = document.createElement('div');
                    empty.className = 'plural-empty';
                    empty.textContent = 'No plural forms yet.';
                    pluralContent.appendChild(empty);
                }

                pluralKeys.forEach((caseKey) => {
                    const entry = pluralVariations[caseKey] || {};
                    const row = document.createElement('div');
                    row.className = 'plural-row';

                    const label = document.createElement('label');
                    label.textContent = caseKey;
                    row.appendChild(label);

                    const variantTextarea = document.createElement('textarea');
                    variantTextarea.rows = 1;
                    variantTextarea.value = entry.value || '';
                    variantTextarea.addEventListener('change', async (event) => {
                        const updates = { plural: {} };
                        updates.plural[caseKey] = { value: event.target.value };
                        try {
                            await upsertTranslation({
                                key: item.key,
                                language: state.currentLanguage,
                                variations: updates
                            }, `Plural '${caseKey}' saved`);
                        } catch (error) {
                            setStatus(error.message, true);
                        }
                    });
                    row.appendChild(variantTextarea);

                    pluralContent.appendChild(row);
                });

                pluralDetails.appendChild(pluralContent);

                const pluralAdd = document.createElement('div');
                pluralAdd.className = 'plural-add';
                const caseInput = document.createElement('input');
                caseInput.type = 'text';
                caseInput.placeholder = 'Case id';
                const valueInput = document.createElement('textarea');
                valueInput.rows = 1;
                valueInput.placeholder = 'Value';
                const addBtn = document.createElement('button');
                addBtn.type = 'button';
                addBtn.textContent = 'Add plural form';
                addBtn.addEventListener('click', async () => {
                    const caseKey = caseInput.value.trim();
                    if (!caseKey) {
                        setStatus('Plural case id is required', true);
                        return;
                    }
                    const updates = { plural: {} };
                    updates.plural[caseKey] = { value: valueInput.value };
                    try {
                        await upsertTranslation({
                            key: item.key,
                            language: state.currentLanguage,
                            variations: updates
                        }, `Plural '${caseKey}' saved`);
                        caseInput.value = '';
                        valueInput.value = '';
                        fetchTranslations(document.getElementById('search').value);
                    } catch (error) {
                        setStatus(error.message, true);
                    }
                });

                pluralAdd.appendChild(caseInput);
                pluralAdd.appendChild(valueInput);
                pluralAdd.appendChild(addBtn);
                pluralDetails.appendChild(pluralAdd);

                if (pluralKeys.length > 0) {
                    pluralDetails.open = true;
                }

                translationCell.appendChild(pluralDetails);
                tr.appendChild(translationCell);

                const actionsCell = document.createElement('td');
                const deleteTranslationBtn = document.createElement('button');
                deleteTranslationBtn.textContent = 'Delete translation';
                deleteTranslationBtn.addEventListener('click', async () => {
                    await fetch(`/api/translations/${encodeURIComponent(item.key)}/${encodeURIComponent(state.currentLanguage)}`, {
                        method: 'DELETE'
                    });
                    setStatus('Translation deleted');
                    fetchTranslations(document.getElementById('search').value);
                });
                const deleteKeyBtn = document.createElement('button');
                deleteKeyBtn.textContent = 'Delete key';
                deleteKeyBtn.addEventListener('click', async () => {
                    await fetch(`/api/keys/${encodeURIComponent(item.key)}`, { method: 'DELETE' });
                    setStatus('Key deleted');
                    fetchTranslations(document.getElementById('search').value);
                });
                actionsCell.appendChild(deleteTranslationBtn);
                actionsCell.appendChild(deleteKeyBtn);
                tr.appendChild(actionsCell);

                tbody.appendChild(tr);
            });
        }

        async function upsertTranslation(payload, successMessage = 'Translation saved') {
            const { key, language } = payload;
            const body = { key, language };
            if (payload.value !== undefined) {
                body.value = payload.value;
            }
            if (payload.state !== undefined) {
                body.state = payload.state;
            }
            if (payload.variations !== undefined) {
                body.variations = payload.variations;
            }

            const res = await fetch('/api/translations', {
                method: 'PUT',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify(body)
            });

            if (!res.ok) {
                let message = `Failed to save translation (${res.status})`;
                try {
                    const errorBody = await res.json();
                    if (errorBody?.error) {
                        message = errorBody.error;
                    }
                } catch (_) {
                    // ignore JSON parse errors
                }
                throw new Error(message);
            }

            setStatus(successMessage);
        }

        function setStatus(message, isError = false) {
            const status = document.getElementById('status');
            status.textContent = message;
            status.className = isError ? 'status error' : 'status';
            setTimeout(() => { status.textContent = ''; }, 3000);
        }

        document.getElementById('refresh').addEventListener('click', () => {
            const query = document.getElementById('search').value;
            fetchTranslations(query);
        });

        document.getElementById('search').addEventListener('input', (event) => {
            const query = event.target.value;
            fetchTranslations(query);
        });

        document.getElementById('language-select').addEventListener('change', (event) => {
            state.currentLanguage = event.target.value;
            document.getElementById('language-header').textContent = `Translation (${state.currentLanguage})`;
            renderTable();
        });

        document.getElementById('create').addEventListener('click', async () => {
            const key = document.getElementById('new-key').value.trim();
            const value = document.getElementById('new-value').value;
            const language = document.getElementById('language-select').value;
            if (!key) {
                setStatus('Key is required', true);
                return;
            }
            try {
                await upsertTranslation({ key, language, value });
                document.getElementById('new-key').value = '';
                document.getElementById('new-value').value = '';
                fetchTranslations(document.getElementById('search').value);
            } catch (error) {
                setStatus(error.message, true);
            }
        });

        (async function init() {
            await fetchLanguages();
            await fetchTranslations();
        })();
    </script>
</body>
</html>
"#;
