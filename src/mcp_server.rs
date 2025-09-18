use std::{collections::BTreeMap, future::Future, sync::Arc};

use rmcp::{
    handler::server::{
        router::Router,
        tool::{Parameters, ToolRouter},
    },
    model::{CallToolResult, Content, ServerCapabilities, ServerInfo},
    tool, tool_handler, tool_router, ErrorData as McpError,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json;

use crate::store::{
    StoreError, TranslationRecord, TranslationSummary, TranslationUpdate, TranslationValue,
    XcStringsStore, XcStringsStoreManager,
};

#[derive(Clone)]
pub struct XcStringsMcpServer {
    stores: Arc<XcStringsStoreManager>,
    tool_router: ToolRouter<Self>,
}

const DEFAULT_LIST_LIMIT: usize = 100;

impl XcStringsMcpServer {
    pub fn new(stores: Arc<XcStringsStoreManager>) -> Self {
        Self {
            stores,
            tool_router: Self::tool_router(),
        }
    }

    pub fn router(&self) -> Router<Self> {
        Router::new(self.clone()).with_tools(self.tool_router.clone())
    }

    fn error_to_mcp(err: StoreError) -> McpError {
        match err {
            StoreError::TranslationMissing { key, language } => McpError::resource_not_found(
                format!("Translation '{key}' ({language}) not found"),
                None,
            ),
            StoreError::KeyMissing(key) => {
                McpError::resource_not_found(format!("Key '{key}' not found"), None)
            }
            StoreError::KeyExists(key) => {
                McpError::invalid_params(format!("Key '{key}' already exists"), None)
            }
            StoreError::PathRequired => McpError::invalid_params(
                "xcstrings path must be provided via tool arguments".to_string(),
                None,
            ),
            other => McpError::internal_error(other.to_string(), None),
        }
    }

    async fn store_for(&self, path: Option<&str>) -> Result<Arc<XcStringsStore>, McpError> {
        self.stores
            .store_for(path)
            .await
            .map_err(Self::error_to_mcp)
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ListTranslationsParams {
    pub path: String,
    /// Optional case-insensitive search query
    pub query: Option<String>,
    /// Optional maximum number of items to return (defaults to 100)
    #[serde(default)]
    pub limit: Option<u32>,
    /// Include full translation payloads; defaults to false for compact summaries
    #[serde(default)]
    pub include_values: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct GetTranslationParams {
    pub path: String,
    pub key: String,
    pub language: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct UpsertTranslationParams {
    pub path: String,
    pub key: String,
    pub language: String,
    #[serde(default)]
    pub value: Option<Option<String>>,
    #[serde(default)]
    pub state: Option<Option<String>>,
    #[serde(default)]
    pub variations: Option<BTreeMap<String, BTreeMap<String, VariationUpdateParam>>>,
}

#[derive(Debug, Deserialize, JsonSchema, Clone)]
struct VariationUpdateParam {
    #[serde(default)]
    pub value: Option<Option<String>>,
    #[serde(default)]
    pub state: Option<Option<String>>,
    #[serde(default)]
    pub variations: Option<BTreeMap<String, BTreeMap<String, VariationUpdateParam>>>,
}

impl VariationUpdateParam {
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

impl UpsertTranslationParams {
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

#[derive(Debug, Deserialize, JsonSchema)]
struct DeleteTranslationParams {
    pub path: String,
    pub key: String,
    pub language: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct DeleteKeyParams {
    pub path: String,
    pub key: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct SetCommentParams {
    pub path: String,
    pub key: String,
    pub comment: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ListLanguagesParams {
    pub path: String,
}

fn to_json_text<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|err| {
        serde_json::json!({
            "error": format!("Failed to serialize response: {err}"),
        })
        .to_string()
    })
}

#[derive(Debug, Serialize)]
struct TranslationListResponse<T> {
    items: Vec<T>,
    total: usize,
    returned: usize,
    truncated: bool,
}

fn render_json<T: serde::Serialize>(value: &T) -> CallToolResult {
    CallToolResult::success(vec![Content::text(to_json_text(value))])
}

fn render_translation_value(value: Option<TranslationValue>) -> CallToolResult {
    render_json(&value)
}

fn render_languages(languages: Vec<String>) -> CallToolResult {
    render_json(&serde_json::json!({ "languages": languages }))
}

fn render_ok_message(message: &str) -> CallToolResult {
    CallToolResult::success(vec![Content::text(message.to_string())])
}

#[tool_router]
impl XcStringsMcpServer {
    #[tool(description = "List translation entries, optionally filtered by a search query")]
    async fn list_translations(
        &self,
        params: Parameters<ListTranslationsParams>,
    ) -> Result<CallToolResult, McpError> {
        let params = params.0;
        let query = params.query.as_deref();
        let store = self.store_for(Some(params.path.as_str())).await?;
        let limit = params
            .limit
            .map(|value| value as usize)
            .unwrap_or(DEFAULT_LIST_LIMIT);
        let limit = if limit == 0 { usize::MAX } else { limit };
        let include_values = params.include_values.unwrap_or(false);

        if include_values {
            let records = store.list_records(query).await;
            let total = records.len();
            let items: Vec<TranslationRecord> = records.into_iter().take(limit).collect();
            let truncated = total > items.len();
            let response = TranslationListResponse {
                returned: items.len(),
                total,
                truncated,
                items,
            };
            Ok(render_json(&response))
        } else {
            let summaries = store.list_summaries(query).await;
            let total = summaries.len();
            let items: Vec<TranslationSummary> = summaries.into_iter().take(limit).collect();
            let truncated = total > items.len();
            let response = TranslationListResponse {
                returned: items.len(),
                total,
                truncated,
                items,
            };
            Ok(render_json(&response))
        }
    }

    #[tool(description = "Fetch a single translation by key and language")]
    async fn get_translation(
        &self,
        params: Parameters<GetTranslationParams>,
    ) -> Result<CallToolResult, McpError> {
        let params = params.0;
        let store = self.store_for(Some(params.path.as_str())).await?;
        let value = store
            .get_translation(&params.key, &params.language)
            .await
            .map_err(Self::error_to_mcp)?;
        Ok(render_translation_value(value))
    }

    #[tool(description = "Create or update a translation")]
    async fn upsert_translation(
        &self,
        params: Parameters<UpsertTranslationParams>,
    ) -> Result<CallToolResult, McpError> {
        let params = params.0;
        let path = params.path.clone();
        let key = params.key.clone();
        let language = params.language.clone();
        let update = params.into_update();
        let store = self.store_for(Some(path.as_str())).await?;
        let updated = store
            .upsert_translation(&key, &language, update)
            .await
            .map_err(Self::error_to_mcp)?;
        Ok(render_translation_value(Some(updated)))
    }

    #[tool(description = "Delete a translation for a given language")]
    async fn delete_translation(
        &self,
        params: Parameters<DeleteTranslationParams>,
    ) -> Result<CallToolResult, McpError> {
        let params = params.0;
        let store = self.store_for(Some(params.path.as_str())).await?;
        store
            .delete_translation(&params.key, &params.language)
            .await
            .map_err(Self::error_to_mcp)?;
        Ok(render_ok_message("Translation deleted"))
    }

    #[tool(description = "Delete an entire translation key across all languages")]
    async fn delete_key(
        &self,
        params: Parameters<DeleteKeyParams>,
    ) -> Result<CallToolResult, McpError> {
        let params = params.0;
        let store = self.store_for(Some(params.path.as_str())).await?;
        store
            .delete_key(&params.key)
            .await
            .map_err(Self::error_to_mcp)?;
        Ok(render_ok_message("Key deleted"))
    }

    #[tool(description = "Set or clear the developer comment for a translation key")]
    async fn set_comment(
        &self,
        params: Parameters<SetCommentParams>,
    ) -> Result<CallToolResult, McpError> {
        let params = params.0;
        let store = self.store_for(Some(params.path.as_str())).await?;
        store
            .set_comment(&params.key, params.comment.clone())
            .await
            .map_err(Self::error_to_mcp)?;
        Ok(render_ok_message("Comment updated"))
    }

    #[tool(description = "List all languages present in the xcstrings file")]
    async fn list_languages(
        &self,
        params: Parameters<ListLanguagesParams>,
    ) -> Result<CallToolResult, McpError> {
        let params = params.0;
        let store = self.store_for(Some(params.path.as_str())).await?;
        let languages = store.list_languages().await;
        Ok(render_languages(languages))
    }
}

impl From<StoreError> for McpError {
    fn from(value: StoreError) -> Self {
        XcStringsMcpServer::error_to_mcp(value)
    }
}

#[tool_handler(router = self.tool_router)]
impl rmcp::ServerHandler for XcStringsMcpServer {
    fn get_info(&self) -> ServerInfo {
        let mut info = ServerInfo::default();
        info.instructions = Some(
            "Manage translations in Localizable.xcstrings using the provided MCP tools.".into(),
        );
        info.capabilities = ServerCapabilities::builder().enable_tools().build();
        info
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{TranslationUpdate, XcStringsStoreManager};
    use std::{
        collections::BTreeMap,
        path::PathBuf,
        sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        },
        time::{SystemTime, UNIX_EPOCH},
    };

    static NEXT_ID: AtomicUsize = AtomicUsize::new(0);

    fn fresh_store_path(label: &str) -> PathBuf {
        let mut dir = std::env::temp_dir();
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        dir.push(format!("xcstrings_mcp_server_{label}_{nanos}_{id}"));
        std::fs::create_dir_all(&dir).expect("create dir");
        dir.join("Localizable.xcstrings")
    }

    fn parse_json(result: &CallToolResult) -> serde_json::Value {
        let text = result
            .content
            .as_ref()
            .expect("content available")
            .first()
            .expect("content entry")
            .as_text()
            .expect("text content")
            .text
            .clone();
        serde_json::from_str(&text).expect("valid json payload")
    }

    #[tokio::test]
    async fn list_translations_tool_returns_records() {
        let path = fresh_store_path("list_translations");
        let path_str = path.to_str().unwrap().to_string();
        let manager = Arc::new(
            XcStringsStoreManager::new(None)
                .await
                .expect("create manager"),
        );
        let store = manager
            .store_for(Some(path_str.as_str()))
            .await
            .expect("load store");
        store
            .upsert_translation(
                "greeting",
                "en",
                TranslationUpdate::from_value_state(Some("Hello".into()), None),
            )
            .await
            .expect("save translation");
        let server = XcStringsMcpServer::new(manager.clone());

        let result = server
            .list_translations(Parameters(ListTranslationsParams {
                path: path_str.clone(),
                query: None,
                limit: None,
                include_values: None,
            }))
            .await
            .expect("tool success");

        let payload = parse_json(&result);
        assert_eq!(payload.get("total").and_then(|v| v.as_u64()), Some(1));
        assert_eq!(payload.get("returned").and_then(|v| v.as_u64()), Some(1));
        let items = payload
            .get("items")
            .and_then(|v| v.as_array())
            .expect("array payload");
        assert_eq!(items.len(), 1);
        let item = &items[0];
        assert_eq!(item.get("key").and_then(|v| v.as_str()), Some("greeting"));
        assert!(item.get("translations").is_none());
        let languages = item
            .get("languages")
            .and_then(|v| v.as_array())
            .expect("languages array");
        assert_eq!(languages.len(), 1);
        assert_eq!(languages[0].as_str(), Some("en"));
        assert_eq!(
            item.get("hasVariations").and_then(|v| v.as_bool()),
            Some(false)
        );
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[tokio::test]
    async fn list_translations_tool_can_include_values() {
        let path = fresh_store_path("list_translations_full");
        let path_str = path.to_str().unwrap().to_string();
        let manager = Arc::new(
            XcStringsStoreManager::new(None)
                .await
                .expect("create manager"),
        );
        let store = manager
            .store_for(Some(path_str.as_str()))
            .await
            .expect("load store");
        store
            .upsert_translation(
                "greeting",
                "en",
                TranslationUpdate::from_value_state(Some("Hello".into()), None),
            )
            .await
            .expect("save translation");
        let server = XcStringsMcpServer::new(manager.clone());

        let result = server
            .list_translations(Parameters(ListTranslationsParams {
                path: path_str.clone(),
                query: None,
                limit: None,
                include_values: Some(true),
            }))
            .await
            .expect("tool success");

        let payload = parse_json(&result);
        let items = payload
            .get("items")
            .and_then(|v| v.as_array())
            .expect("array payload");
        assert_eq!(items.len(), 1);
        let translations = items[0]
            .get("translations")
            .and_then(|v| v.as_object())
            .expect("translations map");
        assert!(translations.contains_key("en"));
        let greeting = translations
            .get("en")
            .and_then(|v| v.get("value"))
            .and_then(|v| v.as_str());
        assert_eq!(greeting, Some("Hello"));
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[tokio::test]
    async fn list_languages_tool_reports_unique_entries() {
        let path = fresh_store_path("list_languages");
        let path_str = path.to_str().unwrap().to_string();
        let manager = Arc::new(
            XcStringsStoreManager::new(None)
                .await
                .expect("create manager"),
        );
        let store = manager
            .store_for(Some(path_str.as_str()))
            .await
            .expect("load store");
        store
            .upsert_translation(
                "greeting",
                "en",
                TranslationUpdate::from_value_state(Some("Hello".into()), None),
            )
            .await
            .expect("save translation");
        store
            .upsert_translation(
                "greeting",
                "fr",
                TranslationUpdate::from_value_state(Some("Bonjour".into()), None),
            )
            .await
            .expect("save translation");
        let server = XcStringsMcpServer::new(manager.clone());

        let result = server
            .list_languages(Parameters(ListLanguagesParams {
                path: path_str.clone(),
            }))
            .await
            .expect("tool success");
        let payload = parse_json(&result);
        let languages = payload
            .get("languages")
            .and_then(|v| v.as_array())
            .expect("languages array");
        assert!(languages.iter().any(|v| v.as_str() == Some("en")));
        assert!(languages.iter().any(|v| v.as_str() == Some("fr")));
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[tokio::test]
    async fn upsert_translation_tool_supports_plural_variations() {
        let path = fresh_store_path("upsert_plural");
        let path_str = path.to_str().unwrap().to_string();
        let manager = Arc::new(
            XcStringsStoreManager::new(None)
                .await
                .expect("create manager"),
        );
        let store = manager
            .store_for(Some(path_str.as_str()))
            .await
            .expect("load store");
        let server = XcStringsMcpServer::new(manager.clone());

        let mut plural_cases = BTreeMap::new();
        plural_cases.insert(
            "one".to_string(),
            VariationUpdateParam {
                value: Some(Some("One".into())),
                state: None,
                variations: None,
            },
        );
        plural_cases.insert(
            "other".to_string(),
            VariationUpdateParam {
                value: Some(Some("Many".into())),
                state: None,
                variations: None,
            },
        );

        let mut variations = BTreeMap::new();
        variations.insert("plural".to_string(), plural_cases);

        server
            .upsert_translation(Parameters(UpsertTranslationParams {
                path: path_str.clone(),
                key: "items".into(),
                language: "en".into(),
                value: None,
                state: None,
                variations: Some(variations),
            }))
            .await
            .expect("tool success");

        let translation = store
            .get_translation("items", "en")
            .await
            .expect("fetch translation")
            .expect("translation exists");

        let plural = translation
            .variations
            .get("plural")
            .expect("plural selector present");
        assert_eq!(
            plural.get("one").and_then(|entry| entry.value.as_deref()),
            Some("One"),
        );
        assert_eq!(
            plural.get("other").and_then(|entry| entry.value.as_deref()),
            Some("Many"),
        );

        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }
}
