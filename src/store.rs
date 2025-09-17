use std::{
    collections::{BTreeMap, BTreeSet},
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranslationValue {
    pub value: Option<String>,
    pub state: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranslationRecord {
    pub key: String,
    pub comment: Option<String>,
    pub translations: BTreeMap<String, TranslationValue>,
}

#[derive(Clone)]
pub struct XcStringsStore {
    path: PathBuf,
    data: Arc<RwLock<XcStringsFile>>,
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
                    let matches_value = entry.localizations.values().any(|loc| {
                        loc.string_unit
                            .as_ref()
                            .and_then(|unit| unit.value.as_ref())
                            .map(|value| value.to_lowercase().contains(q))
                            .unwrap_or(false)
                    });
                    if !matches_key && !matches_value {
                        return None;
                    }
                }

                let translations = entry
                    .localizations
                    .iter()
                    .map(|(lang, loc)| {
                        let value = loc
                            .string_unit
                            .as_ref()
                            .map(|unit| TranslationValue {
                                value: unit.value.clone(),
                                state: unit.state.clone(),
                            })
                            .unwrap_or_else(|| TranslationValue {
                                value: None,
                                state: None,
                            });
                        (lang.clone(), value)
                    })
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
            .map(|loc| TranslationValue {
                value: loc.string_unit.as_ref().and_then(|unit| unit.value.clone()),
                state: loc.string_unit.as_ref().and_then(|unit| unit.state.clone()),
            }))
    }

    pub async fn upsert_translation(
        &self,
        key: &str,
        language: &str,
        value: Option<String>,
        state: Option<String>,
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

        let mut unit = loc.string_unit.clone().unwrap_or_default();
        unit.value = value.clone();
        unit.state = state.clone();
        loc.string_unit = Some(unit);

        let serialized = serde_json::to_string_pretty(&*doc)?;
        drop(doc);
        fs::write(&self.path, serialized).await?;

        Ok(TranslationValue { value, state })
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
        sync::atomic::{AtomicUsize, Ordering},
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
            .upsert_translation("greeting", "en", Some("Hello".into()), None)
            .await
            .expect("upsert");
        store
            .upsert_translation("greeting", "fr", Some("Bonjour".into()), None)
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
            .upsert_translation("farewell", "en", Some("Bye".into()), None)
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
}
