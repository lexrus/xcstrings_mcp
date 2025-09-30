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
    StoreError, SubstitutionUpdate, TranslationSummary, TranslationUpdate, TranslationValue,
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
            StoreError::LanguageMissing(language) => {
                McpError::resource_not_found(format!("Language '{language}' not found"), None)
            }
            StoreError::LanguageExists(language) => {
                McpError::invalid_params(format!("Language '{language}' already exists"), None)
            }
            StoreError::InvalidLanguage(msg) => {
                McpError::invalid_params(format!("Invalid language: {msg}"), None)
            }
            StoreError::CannotRemoveSourceLanguage(language) => McpError::invalid_params(
                format!("Cannot remove source language '{language}'"),
                None,
            ),
            StoreError::CannotRenameSourceLanguage(language) => McpError::invalid_params(
                format!("Cannot rename source language '{language}'"),
                None,
            ),
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
    #[serde(default)]
    pub substitutions: Option<BTreeMap<String, Option<SubstitutionUpdateParam>>>,
}

#[derive(Debug, Deserialize, JsonSchema, Clone)]
struct VariationUpdateParam {
    #[serde(default)]
    pub value: Option<Option<String>>,
    #[serde(default)]
    pub state: Option<Option<String>>,
    #[serde(default)]
    pub variations: Option<BTreeMap<String, BTreeMap<String, VariationUpdateParam>>>,
    #[serde(default)]
    pub substitutions: Option<BTreeMap<String, Option<SubstitutionUpdateParam>>>,
}

impl VariationUpdateParam {
    fn into_update(self) -> TranslationUpdate {
        let mut update = TranslationUpdate::default();
        update.state = self.state;
        update.value = self.value;
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

#[derive(Debug, Deserialize, JsonSchema, Clone)]
struct SubstitutionUpdateParam {
    #[serde(default)]
    pub value: Option<Option<String>>,
    #[serde(default)]
    pub state: Option<Option<String>>,
    #[serde(rename = "argNum", default)]
    pub arg_num: Option<Option<i64>>,
    #[serde(rename = "formatSpecifier", default)]
    pub format_specifier: Option<Option<String>>,
    #[serde(default)]
    pub variations: Option<BTreeMap<String, BTreeMap<String, VariationUpdateParam>>>,
}

impl SubstitutionUpdateParam {
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

impl UpsertTranslationParams {
    fn into_update(self) -> TranslationUpdate {
        let mut update = TranslationUpdate::default();
        update.state = self.state;
        update.value = self.value;
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
struct SetExtractionStateParams {
    pub path: String,
    pub key: String,
    #[serde(rename = "extractionState")]
    pub extraction_state: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ListKeysParams {
    pub path: String,
    /// Optional case-insensitive search query
    pub query: Option<String>,
    /// Optional maximum number of items to return (defaults to 100)
    #[serde(default)]
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ListLanguagesParams {
    pub path: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct AddLanguageParams {
    pub path: String,
    pub language: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct RemoveLanguageParams {
    pub path: String,
    pub language: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct UpdateLanguageParams {
    pub path: String,
    #[serde(rename = "oldLanguage")]
    pub old_language: String,
    #[serde(rename = "newLanguage")]
    pub new_language: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ListUntranslatedParams {
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

    #[tool(description = "List translation keys only, optionally filtered by a search query")]
    async fn list_keys(
        &self,
        params: Parameters<ListKeysParams>,
    ) -> Result<CallToolResult, McpError> {
        let params = params.0;
        let query = params.query.as_deref();
        let store = self.store_for(Some(params.path.as_str())).await?;
        let limit = params
            .limit
            .map(|value| value as usize)
            .unwrap_or(DEFAULT_LIST_LIMIT);
        let limit = if limit == 0 { usize::MAX } else { limit };

        let summaries = store.list_summaries(query).await;
        let total = summaries.len();
        let keys: Vec<String> = summaries.into_iter().take(limit).map(|s| s.key).collect();
        let truncated = total > keys.len();
        let response = serde_json::json!({
            "keys": keys,
            "total": total,
            "returned": keys.len(),
            "truncated": truncated
        });
        Ok(render_json(&response))
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

    #[tool(description = "Set or clear the extraction state for a string key")]
    async fn set_extraction_state(
        &self,
        params: Parameters<SetExtractionStateParams>,
    ) -> Result<CallToolResult, McpError> {
        let params = params.0;
        let store = self.store_for(Some(params.path.as_str())).await?;
        store
            .set_extraction_state(&params.key, params.extraction_state.clone())
            .await
            .map_err(Self::error_to_mcp)?;
        Ok(render_ok_message("Extraction state updated"))
    }

    #[tool(description = "List all languages present in the xcstrings file")]
    async fn list_languages(
        &self,
        params: Parameters<ListLanguagesParams>,
    ) -> Result<CallToolResult, McpError> {
        let params = params.0;
        let store = self.store_for(Some(params.path.as_str())).await?;
        store.reload().await.expect("reload store");
        let languages = store.list_languages().await;
        Ok(render_languages(languages))
    }

    #[tool(description = "Add a new language to the xcstrings file")]
    async fn add_language(
        &self,
        params: Parameters<AddLanguageParams>,
    ) -> Result<CallToolResult, McpError> {
        let params = params.0;
        let store = self.store_for(Some(params.path.as_str())).await?;
        store
            .add_language(&params.language)
            .await
            .map_err(Self::error_to_mcp)?;
        Ok(render_ok_message(&format!(
            "Language '{}' added successfully",
            params.language
        )))
    }

    #[tool(description = "Remove a language from the xcstrings file")]
    async fn remove_language(
        &self,
        params: Parameters<RemoveLanguageParams>,
    ) -> Result<CallToolResult, McpError> {
        let params = params.0;
        let store = self.store_for(Some(params.path.as_str())).await?;
        store
            .remove_language(&params.language)
            .await
            .map_err(Self::error_to_mcp)?;
        Ok(render_ok_message(&format!(
            "Language '{}' removed successfully",
            params.language
        )))
    }

    #[tool(description = "Update/rename a language in the xcstrings file")]
    async fn update_language(
        &self,
        params: Parameters<UpdateLanguageParams>,
    ) -> Result<CallToolResult, McpError> {
        let params = params.0;
        let store = self.store_for(Some(params.path.as_str())).await?;
        store
            .update_language(&params.old_language, &params.new_language)
            .await
            .map_err(Self::error_to_mcp)?;
        Ok(render_ok_message(&format!(
            "Language '{}' renamed to '{}' successfully",
            params.old_language, params.new_language
        )))
    }

    #[tool(
        description = "List untranslated keys per language (empty values or duplicates across languages)"
    )]
    async fn list_untranslated(
        &self,
        params: Parameters<ListUntranslatedParams>,
    ) -> Result<CallToolResult, McpError> {
        let params = params.0;
        let store = self.store_for(Some(params.path.as_str())).await?;
        let untranslated = store.list_untranslated().await;
        Ok(render_json(&untranslated))
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
    async fn list_keys_tool_returns_matching_keys() {
        let path = fresh_store_path("list_keys_tool");
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
            .expect("save greeting");

        store
            .upsert_translation(
                "farewell",
                "en",
                TranslationUpdate::from_value_state(Some("Bye".into()), None),
            )
            .await
            .expect("save farewell");

        let server = XcStringsMcpServer::new(manager.clone());

        // Fetch all keys
        let result = server
            .list_keys(Parameters(ListKeysParams {
                path: path_str.clone(),
                query: None,
                limit: None,
            }))
            .await
            .expect("tool success");

        let payload = parse_json(&result);
        let keys = payload
            .get("keys")
            .and_then(|v| v.as_array())
            .expect("keys array");
        let key_values: Vec<&str> = keys
            .iter()
            .map(|v| v.as_str().expect("string key"))
            .collect();
        assert_eq!(keys.len(), 2);
        assert!(key_values.contains(&"greeting"));
        assert!(key_values.contains(&"farewell"));

        // Query should filter down to a single key
        let result = server
            .list_keys(Parameters(ListKeysParams {
                path: path_str.clone(),
                query: Some("well".to_string()),
                limit: None,
            }))
            .await
            .expect("filtered success");
        let payload = parse_json(&result);
        let keys = payload
            .get("keys")
            .and_then(|v| v.as_array())
            .expect("keys array");
        assert_eq!(keys.len(), 1);
        assert_eq!(keys[0].as_str(), Some("farewell"));

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
                substitutions: None,
            },
        );
        plural_cases.insert(
            "other".to_string(),
            VariationUpdateParam {
                value: Some(Some("Many".into())),
                state: None,
                variations: None,
                substitutions: None,
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
                substitutions: None,
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

    #[tokio::test]
    async fn set_extraction_state_tool_updates_entry() {
        let path = fresh_store_path("set_extraction_state");
        let path_str = path.to_str().unwrap().to_string();
        let manager = Arc::new(
            XcStringsStoreManager::new(None)
                .await
                .expect("create manager"),
        );
        let server = XcStringsMcpServer::new(manager.clone());

        let store = manager
            .store_for(Some(path_str.as_str()))
            .await
            .expect("seed store");

        store
            .upsert_translation(
                "message",
                "en",
                TranslationUpdate::from_value_state(Some("Hello".into()), None),
            )
            .await
            .expect("seed translation");

        server
            .set_extraction_state(Parameters(SetExtractionStateParams {
                path: path_str.clone(),
                key: "message".into(),
                extraction_state: Some("manual".into()),
            }))
            .await
            .expect("tool success");

        let store = manager
            .store_for(Some(path_str.as_str()))
            .await
            .expect("load store");
        let records = store.list_records(None).await;
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].extraction_state.as_deref(), Some("manual"));

        server
            .set_extraction_state(Parameters(SetExtractionStateParams {
                path: path_str.clone(),
                key: "message".into(),
                extraction_state: None,
            }))
            .await
            .expect("tool success");
        let records = store.list_records(None).await;
        assert!(records[0].extraction_state.is_none());

        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[tokio::test]
    async fn add_language_tool_creates_placeholder_localizations() {
        let path = fresh_store_path("add_language_tool");
        let path_str = path.to_str().unwrap().to_string();
        let manager = Arc::new(
            XcStringsStoreManager::new(None)
                .await
                .expect("create manager"),
        );
        let server = XcStringsMcpServer::new(manager.clone());

        let store = manager
            .store_for(Some(path_str.as_str()))
            .await
            .expect("load store");

        // Add some initial translations
        store
            .upsert_translation(
                "greeting",
                "en",
                TranslationUpdate::from_value_state(Some("Hello".into()), None),
            )
            .await
            .expect("save translation");

        // Add French language via MCP tool
        let result = server
            .add_language(Parameters(AddLanguageParams {
                path: path_str.clone(),
                language: "fr".to_string(),
            }))
            .await
            .expect("tool success");

        // Verify success message
        let content = result.content.as_ref().expect("content available");
        let text = content
            .first()
            .expect("content entry")
            .as_text()
            .expect("text content");
        assert!(text.text.contains("Language 'fr' added successfully"));

        store.reload().await.expect("reload store");
        let languages = store.list_languages().await;
        assert!(languages.contains(&"fr".to_string()));

        // Placeholder should exist with needs-translation state
        let placeholder = store
            .get_translation("greeting", "fr")
            .await
            .expect("lookup succeeds")
            .expect("placeholder created");
        assert_eq!(placeholder.state.as_deref(), Some("needs-translation"));
        assert!(placeholder.value.is_none());

        // But we can add translations for this language
        store
            .upsert_translation(
                "greeting",
                "fr",
                TranslationUpdate::from_value_state(Some("Bonjour".into()), None),
            )
            .await
            .expect("save fr translation");

        // Now the language still appears and has a translated value
        let languages = store.list_languages().await;
        assert!(languages.contains(&"fr".to_string()));

        let greeting_fr = store.get_translation("greeting", "fr").await.unwrap();
        let greeting_fr = greeting_fr.expect("translation exists");
        assert_eq!(greeting_fr.value.as_deref(), Some("Bonjour"));
        assert_eq!(greeting_fr.state.as_deref(), Some("translated"));

        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[tokio::test]
    async fn add_language_tool_fails_if_exists() {
        let path = fresh_store_path("add_language_tool_exists");
        let path_str = path.to_str().unwrap().to_string();
        let manager = Arc::new(
            XcStringsStoreManager::new(None)
                .await
                .expect("create manager"),
        );
        let server = XcStringsMcpServer::new(manager.clone());

        // Try to add English (source language)
        let result = server
            .add_language(Parameters(AddLanguageParams {
                path: path_str.clone(),
                language: "en".to_string(),
            }))
            .await;

        assert!(result.is_err());
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[tokio::test]
    async fn remove_language_tool_deletes_localizations() {
        let path = fresh_store_path("remove_language_tool");
        let path_str = path.to_str().unwrap().to_string();
        let manager = Arc::new(
            XcStringsStoreManager::new(None)
                .await
                .expect("create manager"),
        );
        let server = XcStringsMcpServer::new(manager.clone());

        let store = manager
            .store_for(Some(path_str.as_str()))
            .await
            .expect("load store");

        // Add translations in multiple languages
        store
            .upsert_translation(
                "greeting",
                "en",
                TranslationUpdate::from_value_state(Some("Hello".into()), None),
            )
            .await
            .expect("save en translation");

        store
            .upsert_translation(
                "greeting",
                "fr",
                TranslationUpdate::from_value_state(Some("Bonjour".into()), None),
            )
            .await
            .expect("save fr translation");

        // Remove French via MCP tool
        let result = server
            .remove_language(Parameters(RemoveLanguageParams {
                path: path_str.clone(),
                language: "fr".to_string(),
            }))
            .await
            .expect("tool success");

        // Verify success message
        let content = result.content.as_ref().expect("content available");
        let text = content
            .first()
            .expect("content entry")
            .as_text()
            .expect("text content");
        assert!(text.text.contains("Language 'fr' removed successfully"));

        // Explicitly reload the store to ensure we see the changes
        store.reload().await.expect("reload store");

        // Verify French was removed
        let languages = store.list_languages().await;
        assert!(!languages.contains(&"fr".to_string()));

        let greeting_fr = store.get_translation("greeting", "fr").await.unwrap();
        assert!(greeting_fr.is_none());

        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[tokio::test]
    async fn remove_language_tool_fails_if_source_language() {
        let path = fresh_store_path("remove_language_tool_source");
        let path_str = path.to_str().unwrap().to_string();
        let manager = Arc::new(
            XcStringsStoreManager::new(None)
                .await
                .expect("create manager"),
        );
        let server = XcStringsMcpServer::new(manager.clone());

        // Try to remove English (source language)
        let result = server
            .remove_language(Parameters(RemoveLanguageParams {
                path: path_str.clone(),
                language: "en".to_string(),
            }))
            .await;

        assert!(result.is_err());
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[tokio::test]
    async fn update_language_tool_renames_successfully() {
        let path = fresh_store_path("update_language_tool");
        let path_str = path.to_str().unwrap().to_string();
        let manager = Arc::new(
            XcStringsStoreManager::new(None)
                .await
                .expect("create manager"),
        );
        let server = XcStringsMcpServer::new(manager.clone());

        let store = manager
            .store_for(Some(path_str.as_str()))
            .await
            .expect("load store");

        // Add translations
        store
            .upsert_translation(
                "greeting",
                "en",
                TranslationUpdate::from_value_state(Some("Hello".into()), None),
            )
            .await
            .expect("save en translation");

        store
            .upsert_translation(
                "greeting",
                "fr",
                TranslationUpdate::from_value_state(Some("Bonjour".into()), None),
            )
            .await
            .expect("save fr translation");

        // Rename French to French-France via MCP tool
        let result = server
            .update_language(Parameters(UpdateLanguageParams {
                path: path_str.clone(),
                old_language: "fr".to_string(),
                new_language: "fr-FR".to_string(),
            }))
            .await
            .expect("tool success");

        // Verify success message
        let content = result.content.as_ref().expect("content available");
        let text = content
            .first()
            .expect("content entry")
            .as_text()
            .expect("text content");
        assert!(text
            .text
            .contains("Language 'fr' renamed to 'fr-FR' successfully"));

        // Explicitly reload the store to ensure we see the changes
        store.reload().await.expect("reload store");

        // Verify the rename
        let languages = store.list_languages().await;
        assert!(!languages.contains(&"fr".to_string()));
        assert!(languages.contains(&"fr-FR".to_string()));

        let greeting_fr_fr = store.get_translation("greeting", "fr-FR").await.unwrap();
        assert!(greeting_fr_fr.is_some());
        assert_eq!(greeting_fr_fr.unwrap().value.as_deref(), Some("Bonjour"));

        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[tokio::test]
    async fn update_language_tool_fails_if_source_language() {
        let path = fresh_store_path("update_language_tool_source");
        let path_str = path.to_str().unwrap().to_string();
        let manager = Arc::new(
            XcStringsStoreManager::new(None)
                .await
                .expect("create manager"),
        );
        let server = XcStringsMcpServer::new(manager.clone());

        // Try to rename English (source language)
        let result = server
            .update_language(Parameters(UpdateLanguageParams {
                path: path_str.clone(),
                old_language: "en".to_string(),
                new_language: "en-US".to_string(),
            }))
            .await;

        assert!(result.is_err());
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[tokio::test]
    async fn delete_translation_tool_removes_existing_translation() {
        let path = fresh_store_path("delete_translation_tool");
        let path_str = path.to_str().unwrap().to_string();
        let manager = Arc::new(
            XcStringsStoreManager::new(None)
                .await
                .expect("create manager"),
        );
        let server = XcStringsMcpServer::new(manager.clone());

        let store = manager
            .store_for(Some(path_str.as_str()))
            .await
            .expect("load store");

        // Add a translation
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
            .expect("save fr translation");

        // Delete the English translation via MCP tool
        let result = server
            .delete_translation(Parameters(DeleteTranslationParams {
                path: path_str.clone(),
                key: "greeting".to_string(),
                language: "en".to_string(),
            }))
            .await
            .expect("tool success");

        // Verify success message
        let content = result.content.as_ref().expect("content available");
        let text = content
            .first()
            .expect("content entry")
            .as_text()
            .expect("text content");
        assert_eq!(text.text, "Translation deleted");

        // Reload the store to see the changes
        store.reload().await.expect("reload store");

        // Verify the translation was deleted
        let greeting_en = store.get_translation("greeting", "en").await.unwrap();
        assert!(greeting_en.is_none());

        // Verify the French translation still exists
        let greeting_fr = store.get_translation("greeting", "fr").await.unwrap();
        assert!(greeting_fr.is_some());
        assert_eq!(greeting_fr.unwrap().value.as_deref(), Some("Bonjour"));

        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[tokio::test]
    async fn delete_translation_tool_fails_for_nonexistent_key() {
        let path = fresh_store_path("delete_translation_tool_no_key");
        let path_str = path.to_str().unwrap().to_string();
        let manager = Arc::new(
            XcStringsStoreManager::new(None)
                .await
                .expect("create manager"),
        );
        let server = XcStringsMcpServer::new(manager.clone());

        // Try to delete a translation for a key that doesn't exist
        let result = server
            .delete_translation(Parameters(DeleteTranslationParams {
                path: path_str.clone(),
                key: "nonexistent_key".to_string(),
                language: "en".to_string(),
            }))
            .await;

        assert!(result.is_err());
        let error = result.unwrap_err();
        assert!(error
            .to_string()
            .contains("Translation 'nonexistent_key' (en) not found"));

        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[tokio::test]
    async fn delete_translation_tool_fails_for_nonexistent_language() {
        let path = fresh_store_path("delete_translation_tool_no_lang");
        let path_str = path.to_str().unwrap().to_string();
        let manager = Arc::new(
            XcStringsStoreManager::new(None)
                .await
                .expect("create manager"),
        );
        let server = XcStringsMcpServer::new(manager.clone());

        let store = manager
            .store_for(Some(path_str.as_str()))
            .await
            .expect("load store");

        // Add a translation in English only
        store
            .upsert_translation(
                "greeting",
                "en",
                TranslationUpdate::from_value_state(Some("Hello".into()), None),
            )
            .await
            .expect("save translation");

        // Try to delete a translation for a language that doesn't exist for this key
        let result = server
            .delete_translation(Parameters(DeleteTranslationParams {
                path: path_str.clone(),
                key: "greeting".to_string(),
                language: "fr".to_string(),
            }))
            .await;

        assert!(result.is_err());
        let error = result.unwrap_err();
        assert!(error
            .to_string()
            .contains("Translation 'greeting' (fr) not found"));

        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[tokio::test]
    async fn delete_translation_tool_handles_format_specifiers() {
        let path = fresh_store_path("delete_translation_tool_format");
        let path_str = path.to_str().unwrap().to_string();
        let manager = Arc::new(
            XcStringsStoreManager::new(None)
                .await
                .expect("create manager"),
        );
        let server = XcStringsMcpServer::new(manager.clone());

        let store = manager
            .store_for(Some(path_str.as_str()))
            .await
            .expect("load store");

        // Add a translation with format specifiers (like the one that caused the error)
        let key_with_format = "paywall_badge_savings %lld";
        store
            .upsert_translation(
                key_with_format,
                "en",
                TranslationUpdate::from_value_state(Some("Save %lld%".into()), None),
            )
            .await
            .expect("save translation");

        // Delete the translation via MCP tool
        let result = server
            .delete_translation(Parameters(DeleteTranslationParams {
                path: path_str.clone(),
                key: key_with_format.to_string(),
                language: "en".to_string(),
            }))
            .await
            .expect("tool success");

        // Verify success message
        let content = result.content.as_ref().expect("content available");
        let text = content
            .first()
            .expect("content entry")
            .as_text()
            .expect("text content");
        assert_eq!(text.text, "Translation deleted");

        // Reload the store to see the changes
        store.reload().await.expect("reload store");

        // Verify the translation was deleted
        let translation = store.get_translation(key_with_format, "en").await.unwrap();
        assert!(translation.is_none());

        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[tokio::test]
    async fn delete_translation_tool_handles_special_characters() {
        let path = fresh_store_path("delete_translation_tool_special");
        let path_str = path.to_str().unwrap().to_string();
        let manager = Arc::new(
            XcStringsStoreManager::new(None)
                .await
                .expect("create manager"),
        );
        let server = XcStringsMcpServer::new(manager.clone());

        let store = manager
            .store_for(Some(path_str.as_str()))
            .await
            .expect("load store");

        // Test various special characters that might cause issues
        let special_keys = vec![
            "key with spaces",
            "key.with.dots",
            "key-with-dashes",
            "key_with_underscores",
            "key/with/slashes",
            "key@with@symbols",
            "key#with#hash",
            "key$with$dollar",
            "key%with%percent",
            "key^with^caret",
            "key&with&ampersand",
            "key*with*asterisk",
            "key(with)parentheses",
            "key[with]brackets",
            "key{with}braces",
            "key|with|pipes",
            "key\\with\\backslashes",
            "key\"with\"quotes",
            "key'with'apostrophes",
            "key`with`backticks",
            "key~with~tildes",
            "key!with!exclamation",
            "key?with?question",
            "key<with>angles",
            "key=with=equals",
            "key+with+plus",
            "key,with,commas",
            "key;with;semicolons",
            "key:with:colons",
        ];

        for key in &special_keys {
            // Add translation
            store
                .upsert_translation(
                    key,
                    "en",
                    TranslationUpdate::from_value_state(Some(format!("Value for {}", key)), None),
                )
                .await
                .expect("save translation");

            // Delete translation via MCP tool
            let result = server
                .delete_translation(Parameters(DeleteTranslationParams {
                    path: path_str.clone(),
                    key: key.to_string(),
                    language: "en".to_string(),
                }))
                .await
                .expect("tool success");

            // Verify success message
            let content = result.content.as_ref().expect("content available");
            let text = content
                .first()
                .expect("content entry")
                .as_text()
                .expect("text content");
            assert_eq!(text.text, "Translation deleted");

            // Reload the store to see the changes
            store.reload().await.expect("reload store");

            // Verify the translation was deleted
            let translation = store.get_translation(key, "en").await.unwrap();
            assert!(
                translation.is_none(),
                "Translation should be deleted for key: {}",
                key
            );
        }

        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[tokio::test]
    async fn delete_translation_tool_removes_key_when_last_translation() {
        let path = fresh_store_path("delete_translation_tool_last");
        let path_str = path.to_str().unwrap().to_string();
        let manager = Arc::new(
            XcStringsStoreManager::new(None)
                .await
                .expect("create manager"),
        );
        let server = XcStringsMcpServer::new(manager.clone());

        let store = manager
            .store_for(Some(path_str.as_str()))
            .await
            .expect("load store");

        // Add a translation with only one language
        store
            .upsert_translation(
                "single_lang_key",
                "en",
                TranslationUpdate::from_value_state(Some("Only English".into()), None),
            )
            .await
            .expect("save translation");

        // Verify the key exists
        let records_before = store.list_records(None).await;
        assert_eq!(records_before.len(), 1);
        assert_eq!(records_before[0].key, "single_lang_key");

        // Delete the only translation via MCP tool
        let result = server
            .delete_translation(Parameters(DeleteTranslationParams {
                path: path_str.clone(),
                key: "single_lang_key".to_string(),
                language: "en".to_string(),
            }))
            .await
            .expect("tool success");

        // Verify success message
        let content = result.content.as_ref().expect("content available");
        let text = content
            .first()
            .expect("content entry")
            .as_text()
            .expect("text content");
        assert_eq!(text.text, "Translation deleted");

        // Reload the store to see the changes
        store.reload().await.expect("reload store");

        // Verify the entire key was removed (since it has no translations left)
        let records_after = store.list_records(None).await;
        assert_eq!(records_after.len(), 0);

        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[tokio::test]
    async fn delete_translation_tool_handles_unicode_characters() {
        let path = fresh_store_path("delete_translation_tool_unicode");
        let path_str = path.to_str().unwrap().to_string();
        let manager = Arc::new(
            XcStringsStoreManager::new(None)
                .await
                .expect("create manager"),
        );
        let server = XcStringsMcpServer::new(manager.clone());

        let store = manager
            .store_for(Some(path_str.as_str()))
            .await
            .expect("load store");

        // Test Unicode characters in keys and values
        let unicode_key = "greeting_emoji_🌍_世界_مرحبا";
        let unicode_value = "Hello World! 🌍 世界 مرحبا بالعالم";

        store
            .upsert_translation(
                unicode_key,
                "en",
                TranslationUpdate::from_value_state(Some(unicode_value.into()), None),
            )
            .await
            .expect("save unicode translation");

        // Delete the translation via MCP tool
        let result = server
            .delete_translation(Parameters(DeleteTranslationParams {
                path: path_str.clone(),
                key: unicode_key.to_string(),
                language: "en".to_string(),
            }))
            .await
            .expect("tool success");

        // Verify success message
        let content = result.content.as_ref().expect("content available");
        let text = content
            .first()
            .expect("content entry")
            .as_text()
            .expect("text content");
        assert_eq!(text.text, "Translation deleted");

        // Reload the store to see the changes
        store.reload().await.expect("reload store");

        // Verify the translation was deleted
        let translation = store.get_translation(unicode_key, "en").await.unwrap();
        assert!(translation.is_none());

        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[tokio::test]
    async fn delete_translation_tool_handles_empty_and_whitespace_keys() {
        let path = fresh_store_path("delete_translation_tool_empty");
        let path_str = path.to_str().unwrap().to_string();
        let manager = Arc::new(
            XcStringsStoreManager::new(None)
                .await
                .expect("create manager"),
        );
        let server = XcStringsMcpServer::new(manager.clone());

        let store = manager
            .store_for(Some(path_str.as_str()))
            .await
            .expect("load store");

        // Test whitespace-only keys
        let whitespace_keys = vec![
            " ",        // single space
            "  ",       // multiple spaces
            "\t",       // tab
            "\n",       // newline
            "\r",       // carriage return
            " \t\n\r ", // mixed whitespace
        ];

        for key in &whitespace_keys {
            // Add translation
            store
                .upsert_translation(
                    key,
                    "en",
                    TranslationUpdate::from_value_state(Some("Whitespace key".into()), None),
                )
                .await
                .expect("save translation");

            // Delete translation via MCP tool
            let result = server
                .delete_translation(Parameters(DeleteTranslationParams {
                    path: path_str.clone(),
                    key: key.to_string(),
                    language: "en".to_string(),
                }))
                .await
                .expect("tool success");

            // Verify success message
            let content = result.content.as_ref().expect("content available");
            let text = content
                .first()
                .expect("content entry")
                .as_text()
                .expect("text content");
            assert_eq!(text.text, "Translation deleted");

            // Reload the store to see the changes
            store.reload().await.expect("reload store");

            // Verify the translation was deleted
            let translation = store.get_translation(key, "en").await.unwrap();
            assert!(
                translation.is_none(),
                "Translation should be deleted for whitespace key: {:?}",
                key
            );
        }

        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[tokio::test]
    async fn delete_translation_tool_handles_variations() {
        let path = fresh_store_path("delete_translation_tool_variations");
        let path_str = path.to_str().unwrap().to_string();
        let manager = Arc::new(
            XcStringsStoreManager::new(None)
                .await
                .expect("create manager"),
        );
        let server = XcStringsMcpServer::new(manager.clone());

        let store = manager
            .store_for(Some(path_str.as_str()))
            .await
            .expect("load store");

        // Create a translation with plural variations
        let mut plural_cases = BTreeMap::new();
        plural_cases.insert(
            "one".to_string(),
            VariationUpdateParam {
                value: Some(Some("One item".into())),
                state: None,
                variations: None,
                substitutions: None,
            },
        );
        plural_cases.insert(
            "other".to_string(),
            VariationUpdateParam {
                value: Some(Some("Many items".into())),
                state: None,
                variations: None,
                substitutions: None,
            },
        );

        let mut variations = BTreeMap::new();
        variations.insert("plural".to_string(), plural_cases);

        // Add translation with variations via MCP tool
        server
            .upsert_translation(Parameters(UpsertTranslationParams {
                path: path_str.clone(),
                key: "item_count".into(),
                language: "en".into(),
                value: None,
                state: None,
                variations: Some(variations),
                substitutions: None,
            }))
            .await
            .expect("upsert with variations");

        // Verify the translation with variations exists
        let translation = store.get_translation("item_count", "en").await.unwrap();
        assert!(translation.is_some());
        let translation = translation.unwrap();
        assert!(translation.variations.contains_key("plural"));

        // Delete the translation via MCP tool
        let result = server
            .delete_translation(Parameters(DeleteTranslationParams {
                path: path_str.clone(),
                key: "item_count".to_string(),
                language: "en".to_string(),
            }))
            .await
            .expect("tool success");

        // Verify success message
        let content = result.content.as_ref().expect("content available");
        let text = content
            .first()
            .expect("content entry")
            .as_text()
            .expect("text content");
        assert_eq!(text.text, "Translation deleted");

        // Reload the store to see the changes
        store.reload().await.expect("reload store");

        // Verify the translation was deleted
        let translation = store.get_translation("item_count", "en").await.unwrap();
        assert!(translation.is_none());

        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[tokio::test]
    async fn delete_translation_tool_handles_substitutions() {
        let path = fresh_store_path("delete_translation_tool_substitutions");
        let path_str = path.to_str().unwrap().to_string();
        let manager = Arc::new(
            XcStringsStoreManager::new(None)
                .await
                .expect("create manager"),
        );
        let server = XcStringsMcpServer::new(manager.clone());

        let store = manager
            .store_for(Some(path_str.as_str()))
            .await
            .expect("load store");

        // Create a translation with substitutions
        let mut substitutions = BTreeMap::new();
        substitutions.insert(
            "count".to_string(),
            Some(SubstitutionUpdateParam {
                value: Some(Some("%lld".into())),
                state: None,
                arg_num: Some(Some(1)),
                format_specifier: Some(Some("lld".into())),
                variations: None,
            }),
        );

        // Add translation with substitutions via MCP tool
        server
            .upsert_translation(Parameters(UpsertTranslationParams {
                path: path_str.clone(),
                key: "download_progress".into(),
                language: "en".into(),
                value: Some(Some("Downloaded %lld files".into())),
                state: None,
                variations: None,
                substitutions: Some(substitutions),
            }))
            .await
            .expect("upsert with substitutions");

        // Verify the translation with substitutions exists
        let translation = store
            .get_translation("download_progress", "en")
            .await
            .unwrap();
        assert!(translation.is_some());
        let translation = translation.unwrap();
        assert!(translation.substitutions.contains_key("count"));

        // Delete the translation via MCP tool
        let result = server
            .delete_translation(Parameters(DeleteTranslationParams {
                path: path_str.clone(),
                key: "download_progress".to_string(),
                language: "en".to_string(),
            }))
            .await
            .expect("tool success");

        // Verify success message
        let content = result.content.as_ref().expect("content available");
        let text = content
            .first()
            .expect("content entry")
            .as_text()
            .expect("text content");
        assert_eq!(text.text, "Translation deleted");

        // Reload the store to see the changes
        store.reload().await.expect("reload store");

        // Verify the translation was deleted
        let translation = store
            .get_translation("download_progress", "en")
            .await
            .unwrap();
        assert!(translation.is_none());

        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[tokio::test]
    async fn delete_translation_tool_handles_complex_variations_and_substitutions() {
        let path = fresh_store_path("delete_translation_tool_complex");
        let path_str = path.to_str().unwrap().to_string();
        let manager = Arc::new(
            XcStringsStoreManager::new(None)
                .await
                .expect("create manager"),
        );
        let server = XcStringsMcpServer::new(manager.clone());

        let store = manager
            .store_for(Some(path_str.as_str()))
            .await
            .expect("load store");

        // Create complex nested variations with substitutions
        let mut substitutions = BTreeMap::new();
        substitutions.insert(
            "count".to_string(),
            Some(SubstitutionUpdateParam {
                value: Some(Some("%lld".into())),
                state: None,
                arg_num: Some(Some(1)),
                format_specifier: Some(Some("lld".into())),
                variations: None,
            }),
        );

        let mut plural_cases = BTreeMap::new();
        plural_cases.insert(
            "one".to_string(),
            VariationUpdateParam {
                value: Some(Some("Downloaded %lld file".into())),
                state: None,
                variations: None,
                substitutions: Some(substitutions.clone()),
            },
        );
        plural_cases.insert(
            "other".to_string(),
            VariationUpdateParam {
                value: Some(Some("Downloaded %lld files".into())),
                state: None,
                variations: None,
                substitutions: Some(substitutions.clone()),
            },
        );

        let mut variations = BTreeMap::new();
        variations.insert("plural".to_string(), plural_cases);

        // Add complex translation via MCP tool
        server
            .upsert_translation(Parameters(UpsertTranslationParams {
                path: path_str.clone(),
                key: "complex_download_status".into(),
                language: "en".into(),
                value: None,
                state: None,
                variations: Some(variations),
                substitutions: Some(substitutions),
            }))
            .await
            .expect("upsert complex translation");

        // Verify the complex translation exists
        let translation = store
            .get_translation("complex_download_status", "en")
            .await
            .unwrap();
        assert!(translation.is_some());
        let translation = translation.unwrap();
        assert!(translation.variations.contains_key("plural"));
        assert!(translation.substitutions.contains_key("count"));

        // Delete the translation via MCP tool
        let result = server
            .delete_translation(Parameters(DeleteTranslationParams {
                path: path_str.clone(),
                key: "complex_download_status".to_string(),
                language: "en".to_string(),
            }))
            .await
            .expect("tool success");

        // Verify success message
        let content = result.content.as_ref().expect("content available");
        let text = content
            .first()
            .expect("content entry")
            .as_text()
            .expect("text content");
        assert_eq!(text.text, "Translation deleted");

        // Reload the store to see the changes
        store.reload().await.expect("reload store");

        // Verify the translation was deleted
        let translation = store
            .get_translation("complex_download_status", "en")
            .await
            .unwrap();
        assert!(translation.is_none());

        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[tokio::test]
    async fn set_extraction_state_tool_creates_key_if_not_exists() {
        let path = fresh_store_path("set_extraction_state_no_key");
        let path_str = path.to_str().unwrap().to_string();
        let manager = Arc::new(
            XcStringsStoreManager::new(None)
                .await
                .expect("create manager"),
        );
        let server = XcStringsMcpServer::new(manager.clone());

        let store = manager
            .store_for(Some(path_str.as_str()))
            .await
            .expect("load store");

        // Set extraction state for a key that doesn't exist yet
        let result = server
            .set_extraction_state(Parameters(SetExtractionStateParams {
                path: path_str.clone(),
                key: "new_key".to_string(),
                extraction_state: Some("manual".to_string()),
            }))
            .await
            .expect("tool success");

        // Verify success message
        let content = result.content.as_ref().expect("content available");
        let text = content
            .first()
            .expect("content entry")
            .as_text()
            .expect("text content");
        assert_eq!(text.text, "Extraction state updated");

        // Reload the store to see the changes
        store.reload().await.expect("reload store");

        // Verify the key was created with extraction state
        let records = store.list_records(None).await;
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].key, "new_key");
        assert_eq!(records[0].extraction_state.as_deref(), Some("manual"));

        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[tokio::test]
    async fn set_extraction_state_tool_handles_special_characters() {
        let path = fresh_store_path("set_extraction_state_special");
        let path_str = path.to_str().unwrap().to_string();
        let manager = Arc::new(
            XcStringsStoreManager::new(None)
                .await
                .expect("create manager"),
        );
        let server = XcStringsMcpServer::new(manager.clone());

        let store = manager
            .store_for(Some(path_str.as_str()))
            .await
            .expect("load store");

        // Test key with format specifiers (like the one that might cause issues)
        let key_with_format = "paywall_badge_savings %lld";
        store
            .upsert_translation(
                key_with_format,
                "en",
                TranslationUpdate::from_value_state(Some("Save %lld%".into()), None),
            )
            .await
            .expect("save translation");

        // Set extraction state via MCP tool
        let result = server
            .set_extraction_state(Parameters(SetExtractionStateParams {
                path: path_str.clone(),
                key: key_with_format.to_string(),
                extraction_state: Some("manual".to_string()),
            }))
            .await
            .expect("tool success");

        // Verify success message
        let content = result.content.as_ref().expect("content available");
        let text = content
            .first()
            .expect("content entry")
            .as_text()
            .expect("text content");
        assert_eq!(text.text, "Extraction state updated");

        // Reload the store to see the changes
        store.reload().await.expect("reload store");

        // Verify the extraction state was set
        let records = store.list_records(None).await;
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].key, key_with_format);
        assert_eq!(records[0].extraction_state.as_deref(), Some("manual"));

        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[tokio::test]
    async fn set_extraction_state_tool_clears_state() {
        let path = fresh_store_path("set_extraction_state_clear");
        let path_str = path.to_str().unwrap().to_string();
        let manager = Arc::new(
            XcStringsStoreManager::new(None)
                .await
                .expect("create manager"),
        );
        let server = XcStringsMcpServer::new(manager.clone());

        let store = manager
            .store_for(Some(path_str.as_str()))
            .await
            .expect("load store");

        // Add a translation
        store
            .upsert_translation(
                "test_key",
                "en",
                TranslationUpdate::from_value_state(Some("Test value".into()), None),
            )
            .await
            .expect("save translation");

        // Set extraction state first
        server
            .set_extraction_state(Parameters(SetExtractionStateParams {
                path: path_str.clone(),
                key: "test_key".to_string(),
                extraction_state: Some("manual".to_string()),
            }))
            .await
            .expect("set extraction state");

        // Reload and verify it was set
        store.reload().await.expect("reload store");
        let records = store.list_records(None).await;
        assert_eq!(records[0].extraction_state.as_deref(), Some("manual"));

        // Clear extraction state via MCP tool
        let result = server
            .set_extraction_state(Parameters(SetExtractionStateParams {
                path: path_str.clone(),
                key: "test_key".to_string(),
                extraction_state: None,
            }))
            .await
            .expect("tool success");

        // Verify success message
        let content = result.content.as_ref().expect("content available");
        let text = content
            .first()
            .expect("content entry")
            .as_text()
            .expect("text content");
        assert_eq!(text.text, "Extraction state updated");

        // Reload and verify it was cleared
        store.reload().await.expect("reload store");
        let records = store.list_records(None).await;
        assert!(records[0].extraction_state.is_none());

        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[tokio::test]
    async fn list_untranslated_tool_returns_untranslated_keys() {
        let path = fresh_store_path("list_untranslated_tool");
        let path_str = path.to_str().unwrap().to_string();
        let manager = Arc::new(
            XcStringsStoreManager::new(None)
                .await
                .expect("create manager"),
        );
        let server = XcStringsMcpServer::new(manager.clone());

        let store = manager
            .store_for(Some(path_str.as_str()))
            .await
            .expect("load store");

        // Add some translations with various states
        store
            .upsert_translation(
                "key1",
                "en",
                TranslationUpdate::from_value_state(Some("Hello".into()), None),
            )
            .await
            .expect("save en translation");

        store
            .upsert_translation(
                "key1",
                "fr",
                TranslationUpdate::from_value_state(Some("Bonjour".into()), None),
            )
            .await
            .expect("save fr translation");

        store
            .upsert_translation(
                "key2",
                "en",
                TranslationUpdate::from_value_state(Some("World".into()), None),
            )
            .await
            .expect("save en translation");

        // key2: no French translation (will be missing)

        store
            .upsert_translation(
                "key3",
                "en",
                TranslationUpdate::from_value_state(Some("Foo".into()), None),
            )
            .await
            .expect("save en translation");

        store
            .upsert_translation(
                "key3",
                "fr",
                TranslationUpdate::from_value_state(Some("Foo".into()), None), // Duplicate - now OK
            )
            .await
            .expect("save fr translation");

        // Call the MCP tool
        let result = server
            .list_untranslated(Parameters(ListUntranslatedParams {
                path: path_str.clone(),
            }))
            .await
            .expect("tool success");

        // Parse the JSON response
        let payload = parse_json(&result);

        // French should have only key2 (missing)
        let fr_untranslated = payload
            .get("fr")
            .and_then(|v| v.as_array())
            .expect("fr array");
        assert_eq!(fr_untranslated.len(), 1);
        assert!(fr_untranslated.iter().any(|v| v.as_str() == Some("key2")));

        // English should have no untranslated keys
        let en_untranslated = payload.get("en").and_then(|v| v.as_array());
        if let Some(keys) = en_untranslated {
            assert!(keys.is_empty());
        }

        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[tokio::test]
    async fn list_untranslated_tool_handles_empty_store() {
        let path = fresh_store_path("list_untranslated_empty_tool");
        let path_str = path.to_str().unwrap().to_string();
        let manager = Arc::new(
            XcStringsStoreManager::new(None)
                .await
                .expect("create manager"),
        );
        let server = XcStringsMcpServer::new(manager.clone());

        // Call the MCP tool on empty store
        let result = server
            .list_untranslated(Parameters(ListUntranslatedParams {
                path: path_str.clone(),
            }))
            .await
            .expect("tool success");

        // Parse the JSON response
        let payload = parse_json(&result);

        // Should be an empty object or have only source language with empty array
        if let Some(en_untranslated) = payload.get("en").and_then(|v| v.as_array()) {
            assert!(en_untranslated.is_empty());
        }

        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[tokio::test]
    async fn list_untranslated_tool_handles_fully_translated() {
        let path = fresh_store_path("list_untranslated_complete_tool");
        let path_str = path.to_str().unwrap().to_string();
        let manager = Arc::new(
            XcStringsStoreManager::new(None)
                .await
                .expect("create manager"),
        );
        let server = XcStringsMcpServer::new(manager.clone());

        let store = manager
            .store_for(Some(path_str.as_str()))
            .await
            .expect("load store");

        // Add fully translated keys
        store
            .upsert_translation(
                "key1",
                "en",
                TranslationUpdate::from_value_state(Some("Hello".into()), None),
            )
            .await
            .expect("save en translation");

        store
            .upsert_translation(
                "key1",
                "fr",
                TranslationUpdate::from_value_state(Some("Bonjour".into()), None),
            )
            .await
            .expect("save fr translation");

        store
            .upsert_translation(
                "key2",
                "en",
                TranslationUpdate::from_value_state(Some("World".into()), None),
            )
            .await
            .expect("save en translation");

        store
            .upsert_translation(
                "key2",
                "fr",
                TranslationUpdate::from_value_state(Some("Monde".into()), None),
            )
            .await
            .expect("save fr translation");

        // Call the MCP tool
        let result = server
            .list_untranslated(Parameters(ListUntranslatedParams {
                path: path_str.clone(),
            }))
            .await
            .expect("tool success");

        // Parse the JSON response
        let payload = parse_json(&result);

        // All languages should have empty arrays
        if let Some(en_untranslated) = payload.get("en").and_then(|v| v.as_array()) {
            assert!(en_untranslated.is_empty());
        }
        if let Some(fr_untranslated) = payload.get("fr").and_then(|v| v.as_array()) {
            assert!(fr_untranslated.is_empty());
        }

        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }
}
