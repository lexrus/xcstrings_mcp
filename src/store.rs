use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    path::{Path, PathBuf},
    sync::Arc,
};

use serde::{Deserialize, Serialize};
use serde_json::{self, Value};
use thiserror::Error;
use tokio::{fs, sync::RwLock};

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("failed to read xcstrings file: {0}")]
    ReadFailed(#[from] std::io::Error),
    #[error("failed to deserialize/serialize xcstrings json: {0}")]
    SerdeFailed(#[from] serde_json::Error),
    #[error("translation not found for key '{key}' and language '{language}'")]
    TranslationMissing { key: String, language: String },
    #[error("string key '{0}' not found")]
    KeyMissing(String),
    #[error("string key '{0}' already exists")]
    KeyExists(String),
    #[error("xcstrings path is required when no default file has been configured")]
    PathRequired,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct XcStringsFile {
    #[serde(rename = "sourceLanguage")]
    pub source_language: Option<String>,
    #[serde(default)]
    pub strings: BTreeMap<String, XcStringEntry>,
    #[serde(flatten, default)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct XcStringEntry {
    #[serde(rename = "localizations", default)]
    pub localizations: BTreeMap<String, XcLocalization>,
    #[serde(flatten, default)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct XcLocalization {
    #[serde(rename = "stringUnit", default)]
    pub string_unit: Option<XcStringUnit>,
    #[serde(rename = "variations", default)]
    pub variations: BTreeMap<String, BTreeMap<String, XcLocalization>>, // nesting mirrors xcstrings schema
    #[serde(flatten, default)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct XcStringUnit {
    #[serde(default)]
    pub value: Option<String>,
    #[serde(default)]
    pub state: Option<String>,
    #[serde(flatten, default)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TranslationValue {
    pub value: Option<String>,
    pub state: Option<String>,
    #[serde(default)]
    pub variations: BTreeMap<String, BTreeMap<String, TranslationValue>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TranslationUpdate {
    pub value: Option<Option<String>>,
    pub state: Option<Option<String>>,
    #[serde(default)]
    pub variations: Option<BTreeMap<String, BTreeMap<String, TranslationUpdate>>>,
}

impl TranslationValue {
    fn from_localization(loc: &XcLocalization) -> Self {
        let value = loc.string_unit.as_ref().and_then(|unit| unit.value.clone());
        let state = loc.string_unit.as_ref().and_then(|unit| unit.state.clone());
        let variations = loc
            .variations
            .iter()
            .map(|(selector, cases)| {
                let converted = cases
                    .iter()
                    .map(|(case, nested)| {
                        (case.clone(), TranslationValue::from_localization(nested))
                    })
                    .collect();
                (selector.clone(), converted)
            })
            .collect();

        TranslationValue {
            value,
            state,
            variations,
        }
    }
}

impl TranslationUpdate {
    pub fn from_value_state(value: Option<String>, state: Option<String>) -> Self {
        Self {
            value: Some(value),
            state: Some(state),
            variations: None,
        }
    }

    pub fn with_variations(
        mut self,
        variations: BTreeMap<String, BTreeMap<String, TranslationUpdate>>,
    ) -> Self {
        self.variations = Some(variations);
        self
    }

    pub fn add_variation(
        mut self,
        selector: impl Into<String>,
        case: impl Into<String>,
        update: TranslationUpdate,
    ) -> Self {
        let selector = selector.into();
        let case = case.into();
        let variations = self.variations.get_or_insert_with(BTreeMap::new);
        let selector_entry = variations.entry(selector).or_insert_with(BTreeMap::new);
        selector_entry.insert(case, update);
        self
    }
}

impl From<TranslationValue> for TranslationUpdate {
    fn from(value: TranslationValue) -> Self {
        let mut update = TranslationUpdate {
            value: Some(value.value),
            state: Some(value.state),
            variations: None,
        };

        if !value.variations.is_empty() {
            let nested = value
                .variations
                .into_iter()
                .map(|(selector, cases)| {
                    let cases = cases
                        .into_iter()
                        .map(|(case, inner)| (case, TranslationUpdate::from(inner)))
                        .collect();
                    (selector, cases)
                })
                .collect();
            update.variations = Some(nested);
        }

        update
    }
}

fn apply_update(target: &mut XcLocalization, update: TranslationUpdate) {
    let mut unit = target.string_unit.clone().unwrap_or_default();

    if let Some(value) = update.value {
        unit.value = value;
    }

    if let Some(state) = update.state {
        unit.state = state;
    }

    if unit.value.is_some() || unit.state.is_some() || !unit.extra.is_empty() {
        target.string_unit = Some(unit);
    } else {
        target.string_unit = None;
    }

    if let Some(variations) = update.variations {
        let mut existing_variations = std::mem::take(&mut target.variations);

        for (selector, cases_update) in variations {
            let mut selector_entry = existing_variations.remove(&selector).unwrap_or_default();

            for (case_key, nested_update) in cases_update {
                let mut nested_loc = selector_entry
                    .remove(&case_key)
                    .unwrap_or_else(XcLocalization::default);
                apply_update(&mut nested_loc, nested_update);

                if nested_loc.string_unit.is_none()
                    && nested_loc.variations.is_empty()
                    && nested_loc.extra.is_empty()
                {
                    continue;
                }

                selector_entry.insert(case_key, nested_loc);
            }

            if !selector_entry.is_empty() {
                target.variations.insert(selector, selector_entry);
            }
        }

        // re-add untouched selectors
        target.variations.extend(existing_variations);
    }
}

fn localization_contains(loc: &XcLocalization, query: &str) -> bool {
    if loc
        .string_unit
        .as_ref()
        .and_then(|unit| unit.value.as_ref())
        .map(|value| value.to_lowercase().contains(query))
        .unwrap_or(false)
    {
        return true;
    }

    loc.variations.values().any(|cases| {
        cases
            .values()
            .any(|nested| localization_contains(nested, query))
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranslationRecord {
    pub key: String,
    pub comment: Option<String>,
    pub translations: BTreeMap<String, TranslationValue>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranslationSummary {
    pub key: String,
    pub comment: Option<String>,
    pub languages: Vec<String>,
    #[serde(rename = "hasVariations")]
    pub has_variations: bool,
}

#[derive(Clone)]
pub struct XcStringsStore {
    path: PathBuf,
    data: Arc<RwLock<XcStringsFile>>,
}

#[derive(Clone)]
pub struct XcStringsStoreManager {
    default_path: Option<PathBuf>,
    stores: Arc<RwLock<HashMap<PathBuf, Arc<XcStringsStore>>>>,
}

impl XcStringsStoreManager {
    pub async fn new(default_path: Option<PathBuf>) -> Result<Self, StoreError> {
        let manager = Self {
            default_path,
            stores: Arc::new(RwLock::new(HashMap::new())),
        };

        if manager.default_path.is_some() {
            manager.store_for(None).await?;
        }

        Ok(manager)
    }

    pub async fn store_for(&self, path: Option<&str>) -> Result<Arc<XcStringsStore>, StoreError> {
        let resolved_path = match path {
            Some(raw) => PathBuf::from(raw),
            None => self.default_path.clone().ok_or(StoreError::PathRequired)?,
        };

        {
            let stores = self.stores.read().await;
            if let Some(store) = stores.get(&resolved_path) {
                return Ok(store.clone());
            }
        }

        let store = Arc::new(XcStringsStore::load_or_create(&resolved_path).await?);
        let mut stores = self.stores.write().await;
        let entry = stores
            .entry(resolved_path.clone())
            .or_insert_with(|| store.clone());
        Ok(entry.clone())
    }

    pub async fn default_store(&self) -> Result<Arc<XcStringsStore>, StoreError> {
        self.store_for(None).await
    }
}

impl XcStringsStore {
    pub async fn load_or_create(path: impl AsRef<Path>) -> Result<Self, StoreError> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            if !parent.exists() {
                fs::create_dir_all(parent).await?;
            }
        }

        let doc = if path.exists() {
            let raw = fs::read_to_string(&path).await?;
            serde_json::from_str(&raw)?
        } else {
            XcStringsFile::default()
        };

        Ok(Self {
            path,
            data: Arc::new(RwLock::new(doc)),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub async fn reload(&self) -> Result<(), StoreError> {
        let raw = fs::read_to_string(&self.path).await?;
        let doc: XcStringsFile = serde_json::from_str(&raw)?;
        *self.data.write().await = doc;
        Ok(())
    }

    pub async fn list_languages(&self) -> Vec<String> {
        let doc = self.data.read().await;
        let mut langs: BTreeSet<String> = BTreeSet::new();
        if let Some(lang) = &doc.source_language {
            langs.insert(lang.clone());
        }
        for entry in doc.strings.values() {
            langs.extend(entry.localizations.keys().cloned());
        }
        langs.into_iter().collect()
    }

    pub async fn list_records(&self, filter: Option<&str>) -> Vec<TranslationRecord> {
        let query = filter.map(|s| s.to_lowercase());
        let doc = self.data.read().await;
        doc.strings
            .iter()
            .filter_map(|(key, entry)| {
                if let Some(q) = &query {
                    let matches_key = key.to_lowercase().contains(q);
                    let matches_value = entry
                        .localizations
                        .values()
                        .any(|loc| localization_contains(loc, q));
                    if !matches_key && !matches_value {
                        return None;
                    }
                }

                let translations = entry
                    .localizations
                    .iter()
                    .map(|(lang, loc)| (lang.clone(), TranslationValue::from_localization(loc)))
                    .collect();

                let comment = entry
                    .extra
                    .get("comment")
                    .and_then(|value| value.as_str().map(|s| s.to_string()));

                Some(TranslationRecord {
                    key: key.clone(),
                    comment,
                    translations,
                })
            })
            .collect()
    }

    pub async fn list_summaries(&self, filter: Option<&str>) -> Vec<TranslationSummary> {
        let query = filter.map(|s| s.to_lowercase());
        let doc = self.data.read().await;
        doc.strings
            .iter()
            .filter_map(|(key, entry)| {
                if let Some(q) = &query {
                    let matches_key = key.to_lowercase().contains(q);
                    let matches_value = entry
                        .localizations
                        .values()
                        .any(|loc| localization_contains(loc, q));
                    if !matches_key && !matches_value {
                        return None;
                    }
                }

                let comment = entry
                    .extra
                    .get("comment")
                    .and_then(|value| value.as_str().map(|s| s.to_string()));

                let languages = entry.localizations.keys().cloned().collect();
                let has_variations = entry
                    .localizations
                    .values()
                    .any(|loc| !loc.variations.is_empty());

                Some(TranslationSummary {
                    key: key.clone(),
                    comment,
                    languages,
                    has_variations,
                })
            })
            .collect()
    }

    pub async fn get_translation(
        &self,
        key: &str,
        language: &str,
    ) -> Result<Option<TranslationValue>, StoreError> {
        let doc = self.data.read().await;
        Ok(doc
            .strings
            .get(key)
            .and_then(|entry| entry.localizations.get(language))
            .map(TranslationValue::from_localization))
    }

    pub async fn upsert_translation(
        &self,
        key: &str,
        language: &str,
        update: TranslationUpdate,
    ) -> Result<TranslationValue, StoreError> {
        let mut doc = self.data.write().await;
        let entry = doc
            .strings
            .entry(key.to_string())
            .or_insert_with(XcStringEntry::default);

        let loc = entry
            .localizations
            .entry(language.to_string())
            .or_insert_with(XcLocalization::default);

        apply_update(loc, update);

        let updated = TranslationValue::from_localization(loc);

        let serialized = serde_json::to_string_pretty(&*doc)?;
        drop(doc);
        fs::write(&self.path, serialized).await?;

        Ok(updated)
    }

    pub async fn delete_translation(&self, key: &str, language: &str) -> Result<(), StoreError> {
        let mut doc = self.data.write().await;
        let translation_exists = if let Some(entry) = doc.strings.get_mut(key) {
            if entry.localizations.remove(language).is_some() {
                if entry.localizations.is_empty() {
                    doc.strings.remove(key);
                }
                true
            } else {
                false
            }
        } else {
            false
        };

        if !translation_exists {
            return Err(StoreError::TranslationMissing {
                key: key.to_string(),
                language: language.to_string(),
            });
        }

        let serialized = serde_json::to_string_pretty(&*doc)?;
        drop(doc);
        fs::write(&self.path, serialized).await?;
        Ok(())
    }

    pub async fn delete_key(&self, key: &str) -> Result<(), StoreError> {
        let mut doc = self.data.write().await;
        if doc.strings.remove(key).is_none() {
            return Err(StoreError::KeyMissing(key.to_string()));
        }
        let serialized = serde_json::to_string_pretty(&*doc)?;
        drop(doc);
        fs::write(&self.path, serialized).await?;
        Ok(())
    }

    pub async fn rename_key(&self, old_key: &str, new_key: &str) -> Result<(), StoreError> {
        if old_key == new_key {
            return Ok(());
        }

        let mut doc = self.data.write().await;
        if doc.strings.contains_key(new_key) {
            return Err(StoreError::KeyExists(new_key.to_string()));
        }

        let entry = doc
            .strings
            .remove(old_key)
            .ok_or_else(|| StoreError::KeyMissing(old_key.to_string()))?;

        doc.strings.insert(new_key.to_string(), entry);

        let serialized = serde_json::to_string_pretty(&*doc)?;
        drop(doc);
        fs::write(&self.path, serialized).await?;
        Ok(())
    }

    pub async fn set_comment(&self, key: &str, comment: Option<String>) -> Result<(), StoreError> {
        let mut doc = self.data.write().await;
        let entry = doc
            .strings
            .entry(key.to_string())
            .or_insert_with(XcStringEntry::default);
        match comment {
            Some(value) => {
                entry
                    .extra
                    .insert("comment".to_string(), Value::String(value));
            }
            None => {
                entry.extra.remove("comment");
            }
        }
        let serialized = serde_json::to_string_pretty(&*doc)?;
        drop(doc);
        fs::write(&self.path, serialized).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        path::PathBuf,
        sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        },
        time::{SystemTime, UNIX_EPOCH},
    };

    static NEXT_ID: AtomicUsize = AtomicUsize::new(0);

    struct TempStorePath {
        dir: PathBuf,
        file: PathBuf,
    }

    impl TempStorePath {
        fn new(test_name: &str) -> Self {
            let mut dir = std::env::temp_dir();
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
            dir.push(format!("xcstrings_mcp_{test_name}_{nanos}_{id}"));
            std::fs::create_dir_all(&dir).expect("create temp dir");
            let file = dir.join("Localizable.xcstrings");
            Self { dir, file }
        }
    }

    impl Drop for TempStorePath {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    #[tokio::test]
    async fn upsert_and_fetch_translation() {
        let tmp = TempStorePath::new("upsert_fetch");
        let store = XcStringsStore::load_or_create(&tmp.file)
            .await
            .expect("load store");

        store
            .upsert_translation(
                "greeting",
                "en",
                TranslationUpdate::from_value_state(Some("Hello".into()), None),
            )
            .await
            .expect("upsert");
        store
            .upsert_translation(
                "greeting",
                "fr",
                TranslationUpdate::from_value_state(Some("Bonjour".into()), None),
            )
            .await
            .expect("upsert");

        let value = store
            .get_translation("greeting", "en")
            .await
            .expect("get")
            .expect("value");
        assert_eq!(value.value.as_deref(), Some("Hello"));

        let languages = store.list_languages().await;
        assert!(languages.contains(&"en".to_string()));
        assert!(languages.contains(&"fr".to_string()));

        let records = store.list_records(None).await;
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].key, "greeting");
        assert!(records[0].translations.contains_key("fr"));
    }

    #[tokio::test]
    async fn delete_translation_removes_empty_keys() {
        let tmp = TempStorePath::new("delete_translation");
        let store = XcStringsStore::load_or_create(&tmp.file)
            .await
            .expect("load store");

        store
            .upsert_translation(
                "farewell",
                "en",
                TranslationUpdate::from_value_state(Some("Bye".into()), None),
            )
            .await
            .expect("upsert");

        store
            .delete_translation("farewell", "en")
            .await
            .expect("delete translation");

        assert!(matches!(
            store.get_translation("farewell", "en").await.expect("get"),
            None
        ));

        let err = store.delete_key("farewell").await.unwrap_err();
        assert!(matches!(err, StoreError::KeyMissing(_)));
    }

    #[tokio::test]
    async fn rename_key_moves_entry() {
        let tmp = TempStorePath::new("rename_key");
        let store = XcStringsStore::load_or_create(&tmp.file)
            .await
            .expect("load store");

        store
            .upsert_translation(
                "old.key",
                "en",
                TranslationUpdate::from_value_state(Some("Original".into()), None),
            )
            .await
            .expect("seed translation");

        store
            .rename_key("old.key", "new.key")
            .await
            .expect("rename");

        let missing = store
            .get_translation("old.key", "en")
            .await
            .expect("fetch old")
            .is_none();
        assert!(missing);

        let renamed = store
            .get_translation("new.key", "en")
            .await
            .expect("fetch new")
            .expect("translation exists");
        assert_eq!(renamed.value.as_deref(), Some("Original"));

        store
            .upsert_translation(
                "other.key",
                "en",
                TranslationUpdate::from_value_state(Some("Conflict".into()), None),
            )
            .await
            .expect("seed other");

        let err = store.rename_key("new.key", "other.key").await.unwrap_err();
        assert!(matches!(err, StoreError::KeyExists(conflict) if conflict == "other.key"));
    }

    #[tokio::test]
    async fn comment_round_trip() {
        let tmp = TempStorePath::new("comment_round_trip");
        let store = XcStringsStore::load_or_create(&tmp.file)
            .await
            .expect("load store");

        store
            .set_comment("title", Some("Shown on welcome screen".into()))
            .await
            .expect("set comment");

        let records = store.list_records(None).await;
        assert_eq!(
            records[0].comment.as_deref(),
            Some("Shown on welcome screen")
        );

        store
            .set_comment("title", None)
            .await
            .expect("clear comment");
        let records = store.list_records(None).await;
        assert!(records[0].comment.is_none());
    }

    #[tokio::test]
    async fn list_summaries_returns_languages_and_variation_flag() {
        let tmp = TempStorePath::new("list_summaries");
        let store = XcStringsStore::load_or_create(&tmp.file)
            .await
            .expect("load store");

        store
            .upsert_translation(
                "greeting",
                "en",
                TranslationUpdate::from_value_state(Some("Hello".into()), None),
            )
            .await
            .expect("save en");
        let plural_update = TranslationUpdate::from_value_state(None, None).add_variation(
            "plural",
            "other",
            TranslationUpdate::from_value_state(Some("Hallo alle".into()), None),
        );
        store
            .upsert_translation("greeting", "de", plural_update)
            .await
            .expect("save de");

        let summaries = store.list_summaries(None).await;
        assert_eq!(summaries.len(), 1);
        let summary = &summaries[0];
        assert_eq!(summary.key, "greeting");
        assert_eq!(summary.languages, vec!["de".to_string(), "en".to_string()]);
        assert!(summary.has_variations);
    }

    #[tokio::test]
    async fn plural_variations_round_trip() {
        let tmp = TempStorePath::new("plural_round_trip");
        let store = XcStringsStore::load_or_create(&tmp.file)
            .await
            .expect("load store");

        let update = TranslationUpdate::from_value_state(None, None)
            .add_variation(
                "plural",
                "one",
                TranslationUpdate::from_value_state(
                    Some("One file".into()),
                    Some("translated".into()),
                ),
            )
            .add_variation(
                "plural",
                "other",
                TranslationUpdate::from_value_state(
                    Some("{count} files".into()),
                    Some("translated".into()),
                ),
            );

        store
            .upsert_translation("file_count", "en", update)
            .await
            .expect("save plural");

        let value = store
            .get_translation("file_count", "en")
            .await
            .expect("fetch translation")
            .expect("translation exists");

        assert!(value.value.is_none());
        let plural = value
            .variations
            .get("plural")
            .expect("plural selector present");
        assert_eq!(
            plural.get("one").and_then(|entry| entry.value.as_deref()),
            Some("One file")
        );
        assert_eq!(
            plural.get("other").and_then(|entry| entry.value.as_deref()),
            Some("{count} files")
        );

        let records = store.list_records(Some("files")).await;
        assert_eq!(records.len(), 1);
        assert!(records[0]
            .translations
            .get("en")
            .and_then(|entry| entry.variations.get("plural"))
            .is_some());
    }

    #[tokio::test]
    async fn plural_variation_merge_preserves_existing() {
        let tmp = TempStorePath::new("plural_merge");
        let store = XcStringsStore::load_or_create(&tmp.file)
            .await
            .expect("load store");

        let initial = TranslationUpdate::from_value_state(None, None)
            .add_variation(
                "plural",
                "one",
                TranslationUpdate::from_value_state(Some("One".into()), None),
            )
            .add_variation(
                "plural",
                "other",
                TranslationUpdate::from_value_state(Some("Many".into()), None),
            );
        store
            .upsert_translation("items", "en", initial)
            .await
            .expect("save");

        let patch = TranslationUpdate::from_value_state(None, None).add_variation(
            "plural",
            "one",
            TranslationUpdate::from_value_state(Some("Exactly one".into()), None),
        );
        store
            .upsert_translation("items", "en", patch)
            .await
            .expect("patch");

        let value = store
            .get_translation("items", "en")
            .await
            .expect("fetch")
            .expect("exists");
        let plural = value
            .variations
            .get("plural")
            .expect("plural variations available");
        assert_eq!(
            plural.get("one").and_then(|entry| entry.value.as_deref()),
            Some("Exactly one")
        );
        assert_eq!(
            plural.get("other").and_then(|entry| entry.value.as_deref()),
            Some("Many")
        );
    }

    #[tokio::test]
    async fn manager_requires_path_without_default() {
        let manager = XcStringsStoreManager::new(None)
            .await
            .expect("create manager");
        let err = manager.store_for(None).await.err().expect("missing path");
        assert!(matches!(err, StoreError::PathRequired));
    }

    #[tokio::test]
    async fn manager_reuses_loaded_store_for_same_path() {
        let tmp = TempStorePath::new("manager_reuse");
        let manager = XcStringsStoreManager::new(None)
            .await
            .expect("create manager");
        let path_str = tmp.file.to_str().unwrap().to_string();

        let store_a = manager
            .store_for(Some(path_str.as_str()))
            .await
            .expect("first load");
        let store_b = manager
            .store_for(Some(path_str.as_str()))
            .await
            .expect("second load");

        assert!(Arc::ptr_eq(&store_a, &store_b));
    }
}
