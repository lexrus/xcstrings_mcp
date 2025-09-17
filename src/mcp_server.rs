use std::{future::Future, sync::Arc};

use rmcp::{
    handler::server::{
        router::Router,
        tool::{Parameters, ToolRouter},
    },
    model::{CallToolResult, Content, ServerCapabilities, ServerInfo},
    tool, tool_handler, tool_router, ErrorData as McpError,
};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json;

use crate::store::{StoreError, TranslationRecord, TranslationValue, XcStringsStore};

#[derive(Clone)]
pub struct XcStringsMcpServer {
    store: Arc<XcStringsStore>,
    tool_router: ToolRouter<Self>,
}

impl XcStringsMcpServer {
    pub fn new(store: Arc<XcStringsStore>) -> Self {
        Self {
            store,
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
            other => McpError::internal_error(other.to_string(), None),
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ListTranslationsParams {
    /// Optional case-insensitive search query
    pub query: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct GetTranslationParams {
    pub key: String,
    pub language: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct UpsertTranslationParams {
    pub key: String,
    pub language: String,
    pub value: Option<String>,
    pub state: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct DeleteTranslationParams {
    pub key: String,
    pub language: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct DeleteKeyParams {
    pub key: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct SetCommentParams {
    pub key: String,
    pub comment: Option<String>,
}

fn to_json_text<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|err| {
        serde_json::json!({
            "error": format!("Failed to serialize response: {err}"),
        })
        .to_string()
    })
}

fn render_translation_records(records: Vec<TranslationRecord>) -> CallToolResult {
    CallToolResult::success(vec![Content::text(to_json_text(&records))])
}

fn render_translation_value(value: Option<TranslationValue>) -> CallToolResult {
    CallToolResult::success(vec![Content::text(to_json_text(&value))])
}

fn render_languages(languages: Vec<String>) -> CallToolResult {
    CallToolResult::success(vec![Content::text(to_json_text(&serde_json::json!({
        "languages": languages,
    })))])
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
        let records = self.store.list_records(query).await;
        Ok(render_translation_records(records))
    }

    #[tool(description = "Fetch a single translation by key and language")]
    async fn get_translation(
        &self,
        params: Parameters<GetTranslationParams>,
    ) -> Result<CallToolResult, McpError> {
        let params = params.0;
        let value = self
            .store
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
        let updated = self
            .store
            .upsert_translation(
                &params.key,
                &params.language,
                params.value.clone(),
                params.state.clone(),
            )
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
        self.store
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
        self.store
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
        self.store
            .set_comment(&params.key, params.comment.clone())
            .await
            .map_err(Self::error_to_mcp)?;
        Ok(render_ok_message("Comment updated"))
    }

    #[tool(description = "List all languages present in the xcstrings file")]
    async fn list_languages(&self) -> Result<CallToolResult, McpError> {
        let languages = self.store.list_languages().await;
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
    use crate::store::XcStringsStore;
    use std::{
        path::PathBuf,
        sync::atomic::{AtomicUsize, Ordering},
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
        let store = Arc::new(
            XcStringsStore::load_or_create(&path)
                .await
                .expect("load store"),
        );
        store
            .upsert_translation("greeting", "en", Some("Hello".into()), None)
            .await
            .expect("save translation");
        let server = XcStringsMcpServer::new(store);

        let result = server
            .list_translations(Parameters(ListTranslationsParams { query: None }))
            .await
            .expect("tool success");

        let payload = parse_json(&result);
        let items = payload.as_array().expect("array payload");
        assert_eq!(items.len(), 1);
        assert_eq!(
            items[0].get("key").and_then(|v| v.as_str()),
            Some("greeting")
        );
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[tokio::test]
    async fn list_languages_tool_reports_unique_entries() {
        let path = fresh_store_path("list_languages");
        let store = Arc::new(
            XcStringsStore::load_or_create(&path)
                .await
                .expect("load store"),
        );
        store
            .upsert_translation("greeting", "en", Some("Hello".into()), None)
            .await
            .expect("save translation");
        store
            .upsert_translation("greeting", "fr", Some("Bonjour".into()), None)
            .await
            .expect("save translation");
        let server = XcStringsMcpServer::new(store);

        let result = server.list_languages().await.expect("tool success");
        let payload = parse_json(&result);
        let languages = payload
            .get("languages")
            .and_then(|v| v.as_array())
            .expect("languages array");
        assert!(languages.iter().any(|v| v.as_str() == Some("en")));
        assert!(languages.iter().any(|v| v.as_str() == Some("fr")));
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }
}
