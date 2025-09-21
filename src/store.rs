use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    env, io,
    path::{Path, PathBuf},
    sync::Arc,
};

use serde::{Deserialize, Serialize};
use serde_json::{self};
use thiserror::Error;
use tokio::{fs, sync::RwLock, task};

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

const DEFAULT_VERSION: &str = "1.0";
const DEFAULT_SOURCE_LANGUAGE: &str = "en";
const DEFAULT_TRANSLATION_STATE: &str = "translated";

fn default_version() -> String {
    DEFAULT_VERSION.to_string()
}

fn default_source_language() -> String {
    DEFAULT_SOURCE_LANGUAGE.to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum FormatVersion {
    String(String),
    Integer(i64),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct XcStringsFile {
    #[serde(rename = "version", default = "default_version")]
    pub version: String,
    #[serde(rename = "formatVersion", skip_serializing_if = "Option::is_none")]
    pub format_version: Option<FormatVersion>,
    #[serde(rename = "sourceLanguage", default = "default_source_language")]
    pub source_language: String,
    #[serde(default)]
    pub strings: BTreeMap<String, XcStringEntry>,
}

impl Default for XcStringsFile {
    fn default() -> Self {
        Self {
            version: default_version(),
            format_version: None,
            source_language: default_source_language(),
            strings: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct XcStringEntry {
    #[serde(
        rename = "localizations",
        default,
        skip_serializing_if = "BTreeMap::is_empty"
    )]
    pub localizations: BTreeMap<String, XcLocalization>,
    #[serde(rename = "extractionState", skip_serializing_if = "Option::is_none")]
    pub extraction_state: Option<String>,
    #[serde(rename = "shouldTranslate", skip_serializing_if = "Option::is_none")]
    pub should_translate: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct XcLocalization {
    #[serde(rename = "stringUnit", skip_serializing_if = "Option::is_none")]
    pub string_unit: Option<XcStringUnit>,
    #[serde(
        rename = "variations",
        default,
        skip_serializing_if = "BTreeMap::is_empty"
    )]
    pub variations: BTreeMap<String, BTreeMap<String, XcLocalization>>, // nesting mirrors xcstrings schema
    #[serde(
        rename = "substitutions",
        default,
        skip_serializing_if = "BTreeMap::is_empty"
    )]
    pub substitutions: BTreeMap<String, XcSubstitution>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct XcSubstitution {
    #[serde(rename = "stringUnit", skip_serializing_if = "Option::is_none")]
    pub string_unit: Option<XcStringUnit>,
    #[serde(
        rename = "variations",
        default,
        skip_serializing_if = "BTreeMap::is_empty"
    )]
    pub variations: BTreeMap<String, BTreeMap<String, XcLocalization>>,
    #[serde(rename = "argNum", skip_serializing_if = "Option::is_none")]
    pub arg_num: Option<i64>,
    #[serde(rename = "formatSpecifier", skip_serializing_if = "Option::is_none")]
    pub format_specifier: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct XcStringUnit {
    pub value: Option<String>,
    pub state: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TranslationValue {
    pub value: Option<String>,
    pub state: Option<String>,
    #[serde(default)]
    pub variations: BTreeMap<String, BTreeMap<String, TranslationValue>>,
    #[serde(default)]
    pub substitutions: BTreeMap<String, SubstitutionValue>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TranslationUpdate {
    pub value: Option<Option<String>>,
    pub state: Option<Option<String>>,
    #[serde(default)]
    pub variations: Option<BTreeMap<String, BTreeMap<String, TranslationUpdate>>>,
    #[serde(default)]
    pub substitutions: Option<BTreeMap<String, Option<SubstitutionUpdate>>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SubstitutionValue {
    pub value: Option<String>,
    pub state: Option<String>,
    #[serde(rename = "argNum", skip_serializing_if = "Option::is_none")]
    pub arg_num: Option<i64>,
    #[serde(rename = "formatSpecifier", skip_serializing_if = "Option::is_none")]
    pub format_specifier: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub variations: BTreeMap<String, BTreeMap<String, TranslationValue>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SubstitutionUpdate {
    pub value: Option<Option<String>>,
    pub state: Option<Option<String>>,
    #[serde(rename = "argNum", default)]
    pub arg_num: Option<Option<i64>>,
    #[serde(rename = "formatSpecifier", default)]
    pub format_specifier: Option<Option<String>>,
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
        let substitutions = loc
            .substitutions
            .iter()
            .map(|(name, sub)| (name.clone(), SubstitutionValue::from_substitution(sub)))
            .collect();

        TranslationValue {
            value,
            state,
            variations,
            substitutions,
        }
    }
}

impl TranslationUpdate {
    pub fn from_value_state(value: Option<String>, state: Option<String>) -> Self {
        let normalized_state = if value.as_ref().map(|v| !v.is_empty()).unwrap_or(false) {
            state.or_else(|| Some(DEFAULT_TRANSLATION_STATE.to_string()))
        } else {
            state
        };

        Self {
            value: Some(value),
            state: Some(normalized_state),
            variations: None,
            substitutions: None,
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

impl SubstitutionValue {
    fn from_substitution(sub: &XcSubstitution) -> Self {
        let value = sub.string_unit.as_ref().and_then(|unit| unit.value.clone());
        let state = sub.string_unit.as_ref().and_then(|unit| unit.state.clone());
        let variations = sub
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

        SubstitutionValue {
            value,
            state,
            arg_num: sub.arg_num,
            format_specifier: sub.format_specifier.clone(),
            variations,
        }
    }
}

impl From<TranslationValue> for TranslationUpdate {
    fn from(value: TranslationValue) -> Self {
        let mut update = TranslationUpdate {
            value: Some(value.value),
            state: Some(value.state),
            variations: None,
            substitutions: None,
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

        if !value.substitutions.is_empty() {
            let nested = value
                .substitutions
                .into_iter()
                .map(|(name, sub)| (name, Some(SubstitutionUpdate::from(sub))))
                .collect();
            update.substitutions = Some(nested);
        }

        update
    }
}

impl From<SubstitutionValue> for SubstitutionUpdate {
    fn from(value: SubstitutionValue) -> Self {
        let SubstitutionValue {
            value: base_value,
            state,
            arg_num,
            format_specifier,
            variations,
        } = value;

        let mut update = SubstitutionUpdate {
            value: Some(base_value),
            state: Some(state),
            arg_num: Some(arg_num),
            format_specifier: Some(format_specifier),
            variations: None,
        };

        if !variations.is_empty() {
            let nested = variations
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

fn is_blank(value: &Option<String>) -> bool {
    value.as_ref().map(|s| s.trim().is_empty()).unwrap_or(true)
}

fn sanitize_string_unit(unit: &mut XcStringUnit) {
    // Only remove empty values if there's no explicit state
    // This allows empty placeholders with state (e.g., "new") to persist
    if is_blank(&unit.value) && unit.state.is_none() {
        unit.value = None;
    }

    if is_blank(&unit.state) {
        unit.state = None;
    }

    if unit.value.is_some() && unit.state.is_none() {
        unit.state = Some(DEFAULT_TRANSLATION_STATE.to_string());
    }
}

fn string_unit_has_content(unit: &XcStringUnit) -> bool {
    // Consider a unit as having content if it has both value and state
    // Even empty string values are valid if they have a state
    unit.value.is_some() && unit.state.is_some()
}

fn localization_is_empty(loc: &XcLocalization) -> bool {
    loc.string_unit
        .as_ref()
        .map(|unit| !string_unit_has_content(unit))
        .unwrap_or(true)
        && loc.variations.is_empty()
        && loc.substitutions.is_empty()
}

fn normalize_substitution(sub: &mut XcSubstitution) -> bool {
    if let Some(unit) = sub.string_unit.as_mut() {
        sanitize_string_unit(unit);
    }

    if sub
        .string_unit
        .as_ref()
        .map(|unit| !string_unit_has_content(unit))
        .unwrap_or(false)
    {
        sub.string_unit = None;
    }

    sub.variations.retain(|_, cases| {
        cases.retain(|_, nested| !normalize_localization(nested));
        !cases.is_empty()
    });

    substitution_is_empty(sub)
}

fn normalize_localization(loc: &mut XcLocalization) -> bool {
    if let Some(unit) = loc.string_unit.as_mut() {
        sanitize_string_unit(unit);
    }

    if loc
        .string_unit
        .as_ref()
        .map(|unit| !string_unit_has_content(unit))
        .unwrap_or(false)
    {
        loc.string_unit = None;
    }

    loc.variations.retain(|_, cases| {
        cases.retain(|_, nested| !normalize_localization(nested));
        !cases.is_empty()
    });

    loc.substitutions
        .retain(|_, sub| !normalize_substitution(sub));

    localization_is_empty(loc)
}

fn normalize_strings_file(doc: &mut XcStringsFile) {
    if doc.version.trim().is_empty() {
        doc.version = default_version();
    }

    if doc.source_language.trim().is_empty() {
        doc.source_language = default_source_language();
    }

    doc.strings.retain(|_, entry| {
        entry
            .localizations
            .retain(|_, loc| !normalize_localization(loc));

        if entry.localizations.is_empty() {
            entry.comment.is_some()
                || entry.extraction_state.is_some()
                || entry.should_translate.is_some()
        } else {
            true
        }
    });
}

fn apply_update(target: &mut XcLocalization, update: TranslationUpdate) {
    let mut unit = target.string_unit.take().unwrap_or_default();

    if let Some(value) = update.value {
        unit.value = value;
    }

    if let Some(state) = update.state {
        unit.state = state;
    }

    sanitize_string_unit(&mut unit);

    if string_unit_has_content(&unit) {
        target.string_unit = Some(unit);
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

                if localization_is_empty(&nested_loc) {
                    continue;
                }

                selector_entry.insert(case_key, nested_loc);
            }

            if !selector_entry.is_empty() {
                target.variations.insert(selector, selector_entry);
            }
        }

        target.variations.extend(
            existing_variations
                .into_iter()
                .filter(|(_, cases)| !cases.is_empty()),
        );
    }

    if let Some(substitutions) = update.substitutions {
        let mut existing_substitutions = std::mem::take(&mut target.substitutions);

        for (name, maybe_update) in substitutions {
            match maybe_update {
                Some(sub_update) => {
                    let mut substitution = existing_substitutions
                        .remove(&name)
                        .unwrap_or_else(XcSubstitution::default);
                    apply_substitution_update(&mut substitution, sub_update);

                    if !substitution_is_empty(&substitution) {
                        target.substitutions.insert(name, substitution);
                    }
                }
                None => {
                    existing_substitutions.remove(&name);
                }
            }
        }

        target.substitutions.extend(
            existing_substitutions
                .into_iter()
                .filter(|(_, sub)| !substitution_is_empty(sub)),
        );
    }
}

fn apply_substitution_update(target: &mut XcSubstitution, update: SubstitutionUpdate) {
    let mut unit = target.string_unit.take().unwrap_or_default();

    if let Some(value) = update.value {
        unit.value = value;
    }

    if let Some(state) = update.state {
        unit.state = state;
    }

    sanitize_string_unit(&mut unit);

    if string_unit_has_content(&unit) {
        target.string_unit = Some(unit);
    }

    if let Some(arg_num) = update.arg_num {
        target.arg_num = arg_num;
    }

    if let Some(format_specifier) = update.format_specifier {
        target.format_specifier = format_specifier;
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

                if localization_is_empty(&nested_loc) {
                    continue;
                }

                selector_entry.insert(case_key, nested_loc);
            }

            if !selector_entry.is_empty() {
                target.variations.insert(selector, selector_entry);
            }
        }

        target.variations.extend(
            existing_variations
                .into_iter()
                .filter(|(_, cases)| !cases.is_empty()),
        );
    }
}

fn substitution_is_empty(sub: &XcSubstitution) -> bool {
    let string_unit_empty = sub
        .string_unit
        .as_ref()
        .map(|unit| !string_unit_has_content(unit))
        .unwrap_or(true);

    string_unit_empty
        && sub.variations.is_empty()
        && sub.arg_num.is_none()
        && sub.format_specifier.is_none()
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
    }) || loc
        .substitutions
        .values()
        .any(|sub| substitution_contains(sub, query))
}

fn substitution_contains(sub: &XcSubstitution, query: &str) -> bool {
    if sub
        .string_unit
        .as_ref()
        .and_then(|unit| unit.value.as_ref())
        .map(|value| value.to_lowercase().contains(query))
        .unwrap_or(false)
    {
        return true;
    }

    sub.variations.values().any(|cases| {
        cases
            .values()
            .any(|nested| localization_contains(nested, query))
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranslationRecord {
    pub key: String,
    pub comment: Option<String>,
    #[serde(rename = "extractionState")]
    pub extraction_state: Option<String>,
    #[serde(rename = "shouldTranslate")]
    pub should_translate: Option<bool>,
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
    search_root: PathBuf,
    stores: Arc<RwLock<HashMap<PathBuf, Arc<XcStringsStore>>>>,
    discovered_paths: Arc<RwLock<Vec<PathBuf>>>,
}

impl XcStringsStoreManager {
    pub async fn new(default_path: Option<PathBuf>) -> Result<Self, StoreError> {
        let cwd = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let normalized_default = default_path.map(|path| {
            if path.is_absolute() {
                path
            } else {
                cwd.join(path)
            }
        });

        let search_root = normalized_default
            .as_ref()
            .and_then(|path| path.parent().map(|p| p.to_path_buf()))
            .unwrap_or_else(|| cwd.clone());

        let manager = Self {
            default_path: normalized_default,
            search_root,
            stores: Arc::new(RwLock::new(HashMap::new())),
            discovered_paths: Arc::new(RwLock::new(Vec::new())),
        };

        manager.refresh_discovered_paths().await?;

        if manager.default_path.is_some() {
            manager.store_for(None).await?;
        }

        Ok(manager)
    }

    fn resolve_path(&self, raw: &str) -> PathBuf {
        let path = PathBuf::from(raw);
        if path.is_absolute() {
            path
        } else {
            self.search_root.join(path)
        }
    }

    fn normalize_path(&self, path: PathBuf) -> PathBuf {
        std::fs::canonicalize(&path).unwrap_or(path)
    }

    pub fn default_path(&self) -> Option<PathBuf> {
        self.default_path.clone()
    }

    pub fn search_root(&self) -> &Path {
        &self.search_root
    }

    pub async fn available_paths(&self) -> Vec<PathBuf> {
        self.discovered_paths.read().await.clone()
    }

    pub async fn refresh_discovered_paths(&self) -> Result<Vec<PathBuf>, StoreError> {
        let root = self.search_root.clone();
        let default_path = self.default_path.clone();

        let discovered = task::spawn_blocking(move || -> Result<Vec<PathBuf>, io::Error> {
            let mut matches = discover_xcstrings(&root);

            if let Some(default_path) = default_path {
                let normalized = std::fs::canonicalize(&default_path).unwrap_or(default_path);
                if !matches.iter().any(|existing| existing == &normalized) {
                    matches.push(normalized);
                }
            }

            matches.sort();
            matches.dedup();
            Ok(matches)
        })
        .await
        .map_err(|err| {
            StoreError::ReadFailed(io::Error::new(io::ErrorKind::Other, err.to_string()))
        })??;

        {
            let mut guard = self.discovered_paths.write().await;
            *guard = discovered.clone();
        }

        Ok(discovered)
    }

    pub async fn store_for(&self, path: Option<&str>) -> Result<Arc<XcStringsStore>, StoreError> {
        let resolved_path = match path {
            Some(raw) => self.resolve_path(raw),
            None => self.default_path.clone().ok_or(StoreError::PathRequired)?,
        };
        let resolved_path = self.normalize_path(resolved_path);

        {
            let stores = self.stores.read().await;
            if let Some(store) = stores.get(&resolved_path) {
                // Try to reload to ensure we have the latest file contents
                // If reload fails, still return the cached store
                let _ = store.reload().await;
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

fn discover_xcstrings(root: &Path) -> Vec<PathBuf> {
    if !root.exists() {
        return Vec::new();
    }

    let mut results = Vec::new();
    let mut stack = vec![root.to_path_buf()];

    while let Some(dir) = stack.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(_) => continue,
        };

        for entry in entries.flatten() {
            let path = entry.path();
            let file_type = match entry.file_type() {
                Ok(kind) => kind,
                Err(_) => continue,
            };

            if file_type.is_dir() {
                if let Some(name) = path.file_name().and_then(|value| value.to_str()) {
                    let lowered = name.to_ascii_lowercase();
                    if lowered == "target" || lowered == ".git" || lowered == "node_modules" {
                        continue;
                    }
                }
                stack.push(path);
            } else if file_type.is_file() {
                let is_xcstrings = path
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .map(|ext| ext.eq_ignore_ascii_case("xcstrings"))
                    .unwrap_or(false);
                if is_xcstrings {
                    let normalized = std::fs::canonicalize(&path).unwrap_or(path);
                    results.push(normalized);
                }
            }
        }
    }

    results
}

impl XcStringsStore {
    pub async fn load_or_create(path: impl AsRef<Path>) -> Result<Self, StoreError> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            if !parent.exists() {
                fs::create_dir_all(parent).await?;
            }
        }

        let mut doc = if path.exists() {
            let raw = fs::read_to_string(&path).await?;
            serde_json::from_str(&raw)?
        } else {
            XcStringsFile::default()
        };

        normalize_strings_file(&mut doc);

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
        let mut doc: XcStringsFile = serde_json::from_str(&raw)?;
        normalize_strings_file(&mut doc);
        *self.data.write().await = doc;
        Ok(())
    }

    pub async fn list_languages(&self) -> Vec<String> {
        let doc = self.data.read().await;
        let mut langs: BTreeSet<String> = BTreeSet::new();
        langs.insert(doc.source_language.clone());
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

                Some(TranslationRecord {
                    key: key.clone(),
                    comment: entry.comment.clone(),
                    extraction_state: entry.extraction_state.clone(),
                    should_translate: entry.should_translate,
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

                let languages = entry.localizations.keys().cloned().collect();
                let has_variations = entry
                    .localizations
                    .values()
                    .any(|loc| !loc.variations.is_empty() || !loc.substitutions.is_empty());

                Some(TranslationSummary {
                    key: key.clone(),
                    comment: entry.comment.clone(),
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

        normalize_strings_file(&mut doc);
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

        normalize_strings_file(&mut doc);
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
        normalize_strings_file(&mut doc);
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

        normalize_strings_file(&mut doc);
        let serialized = serde_json::to_string_pretty(&*doc)?;
        drop(doc);
        fs::write(&self.path, serialized).await?;
        Ok(())
    }

    pub async fn set_extraction_state(
        &self,
        key: &str,
        state: Option<String>,
    ) -> Result<(), StoreError> {
        let mut doc = self.data.write().await;
        let entry = doc
            .strings
            .entry(key.to_string())
            .or_insert_with(XcStringEntry::default);
        entry.extraction_state = state;

        normalize_strings_file(&mut doc);
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
        entry.comment = comment;
        normalize_strings_file(&mut doc);
        let serialized = serde_json::to_string_pretty(&*doc)?;
        drop(doc);
        fs::write(&self.path, serialized).await?;
        Ok(())
    }

    pub async fn set_should_translate(
        &self,
        key: &str,
        should_translate: Option<bool>,
    ) -> Result<(), StoreError> {
        let mut doc = self.data.write().await;
        let entry = doc
            .strings
            .entry(key.to_string())
            .or_insert_with(XcStringEntry::default);
        entry.should_translate = should_translate;
        normalize_strings_file(&mut doc);
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
            .upsert_translation(
                "title",
                "en",
                TranslationUpdate::from_value_state(Some("Welcome".into()), None),
            )
            .await
            .expect("seed translation");

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
    async fn set_extraction_state_round_trip() {
        let tmp = TempStorePath::new("extraction_state_round_trip");
        let store = XcStringsStore::load_or_create(&tmp.file)
            .await
            .expect("load store");

        store
            .upsert_translation(
                "welcome",
                "en",
                TranslationUpdate::from_value_state(Some("Hi".into()), None),
            )
            .await
            .expect("seed translation for extraction state");

        store
            .set_extraction_state("welcome", Some("manual".into()))
            .await
            .expect("set extraction state");

        let records = store.list_records(None).await;
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].extraction_state.as_deref(), Some("manual"));

        store
            .set_extraction_state("welcome", None)
            .await
            .expect("clear extraction state");
        let records = store.list_records(None).await;
        assert!(records[0].extraction_state.is_none());
    }

    #[tokio::test]
    async fn set_should_translate_round_trip() {
        let tmp = TempStorePath::new("should_translate_round_trip");
        let store = XcStringsStore::load_or_create(&tmp.file)
            .await
            .expect("load store");

        store
            .upsert_translation(
                "login.button",
                "en",
                TranslationUpdate::from_value_state(Some("Login".into()), None),
            )
            .await
            .expect("seed translation for should_translate");

        store
            .set_should_translate("login.button", Some(true))
            .await
            .expect("set should_translate to true");

        let records = store.list_records(None).await;
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].should_translate, Some(true));

        store
            .set_should_translate("login.button", Some(false))
            .await
            .expect("set should_translate to false");
        let records = store.list_records(None).await;
        assert_eq!(records[0].should_translate, Some(false));

        store
            .set_should_translate("login.button", None)
            .await
            .expect("clear should_translate");
        let records = store.list_records(None).await;
        assert!(records[0].should_translate.is_none());
    }

    #[tokio::test]
    async fn substitution_updates_round_trip() {
        let tmp = TempStorePath::new("substitution_round_trip");
        let store = XcStringsStore::load_or_create(&tmp.file)
            .await
            .expect("load store");

        let mut update = TranslationUpdate::default();
        update.value = Some(Some("Found %#@arg1@".into()));
        let mut substitutions = BTreeMap::new();
        let mut substitution = SubstitutionUpdate::default();
        substitution.value = Some(Some("%arg item".into()));
        substitution.arg_num = Some(Some(1));
        substitution.format_specifier = Some(Some("ld".into()));
        substitutions.insert("arg1".to_string(), Some(substitution));
        update.substitutions = Some(substitutions);

        store
            .upsert_translation("message", "en", update)
            .await
            .expect("upsert substitution");

        let en_translation = store
            .get_translation("message", "en")
            .await
            .expect("fetch translation")
            .expect("translation exists");

        let arg1 = en_translation
            .substitutions
            .get("arg1")
            .expect("substitution present");
        assert_eq!(arg1.value.as_deref(), Some("%arg item"));
        assert_eq!(arg1.arg_num, Some(1));
        assert_eq!(arg1.format_specifier.as_deref(), Some("ld"));

        let mut removal = TranslationUpdate::default();
        let mut removal_map = BTreeMap::new();
        removal_map.insert("arg1".to_string(), None);
        removal.substitutions = Some(removal_map);

        store
            .upsert_translation("message", "en", removal)
            .await
            .expect("remove substitution");

        let en_translation = store
            .get_translation("message", "en")
            .await
            .expect("fetch translation")
            .expect("translation exists");
        assert!(en_translation.substitutions.is_empty());
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

    #[tokio::test]
    async fn test_add_substitution_with_empty_value_and_state() {
        let temp = TempStorePath::new("test_substitution_with_state");
        let path = temp.file.clone();

        // Create initial file
        let initial_content = serde_json::json!({
            "sourceLanguage": "en",
            "version": "1.0",
            "strings": {
                "test.key": {
                    "localizations": {
                        "en": {
                            "stringUnit": {
                                "state": "translated",
                                "value": "Hello %@, you have %d messages"
                            }
                        }
                    }
                }
            }
        });

        fs::write(&path, initial_content.to_string()).await.unwrap();

        let store = XcStringsStore::load_or_create(path.clone()).await.unwrap();

        // Add a substitution with empty value but with state
        let mut update = TranslationUpdate::default();
        let mut substitutions = BTreeMap::new();

        let mut sub_update = SubstitutionUpdate::default();
        sub_update.value = Some(Some("".to_string()));
        sub_update.state = Some(Some("new".to_string()));

        substitutions.insert("userName".to_string(), Some(sub_update));
        update.substitutions = Some(substitutions);

        let result = store
            .upsert_translation("test.key", "en", update)
            .await
            .unwrap();

        // Verify the substitution was added
        assert!(!result.substitutions.is_empty());
        let subs = &result.substitutions;
        assert!(subs.contains_key("userName"));

        let user_name_sub = &subs["userName"];
        assert_eq!(user_name_sub.value, Some("".to_string()));
        assert_eq!(user_name_sub.state, Some("new".to_string()));

        // Verify it persists in the file
        let content = fs::read_to_string(&path).await.unwrap();
        assert!(content.contains("\"userName\""));
        assert!(content.contains("\"substitutions\""));
    }

    #[tokio::test]
    async fn test_add_plural_variation_with_empty_value_and_state() {
        let temp = TempStorePath::new("test_plural_with_state");
        let path = temp.file.clone();

        // Create initial file
        let initial_content = serde_json::json!({
            "sourceLanguage": "en",
            "version": "1.0",
            "strings": {
                "message.count": {
                    "localizations": {
                        "en": {
                            "stringUnit": {
                                "state": "translated",
                                "value": "You have messages"
                            }
                        }
                    }
                }
            }
        });

        fs::write(&path, initial_content.to_string()).await.unwrap();

        let store = XcStringsStore::load_or_create(path.clone()).await.unwrap();

        // Add plural variation with empty value but with state
        let mut update = TranslationUpdate::default();
        let mut variations = BTreeMap::new();
        let mut plural_cases = BTreeMap::new();

        let mut one_update = TranslationUpdate::default();
        one_update.value = Some(Some("".to_string()));
        one_update.state = Some(Some("new".to_string()));

        plural_cases.insert("one".to_string(), one_update);
        variations.insert("plural".to_string(), plural_cases);
        update.variations = Some(variations);

        let result = store
            .upsert_translation("message.count", "en", update)
            .await
            .unwrap();

        // Verify the variation was added
        assert!(!result.variations.is_empty());
        let vars = &result.variations;
        assert!(vars.contains_key("plural"));

        let plural_vars = &vars["plural"];
        assert!(plural_vars.contains_key("one"));

        let one_var = &plural_vars["one"];
        assert_eq!(one_var.value, Some("".to_string()));
        assert_eq!(one_var.state, Some("new".to_string()));

        // Verify it persists in the file
        let content = fs::read_to_string(&path).await.unwrap();
        assert!(content.contains("\"variations\""));
        assert!(content.contains("\"plural\""));
        assert!(content.contains("\"one\""));
    }

    #[tokio::test]
    async fn test_substitution_without_state_gets_filtered() {
        let temp = TempStorePath::new("test_substitution_without_state");
        let path = temp.file.clone();

        // Create initial file
        let initial_content = serde_json::json!({
            "sourceLanguage": "en",
            "version": "1.0",
            "strings": {
                "test.key": {
                    "localizations": {
                        "en": {
                            "stringUnit": {
                                "state": "translated",
                                "value": "Hello"
                            }
                        }
                    }
                }
            }
        });

        fs::write(&path, initial_content.to_string()).await.unwrap();

        let store = XcStringsStore::load_or_create(path.clone()).await.unwrap();

        // Try to add a substitution with only empty value (no state)
        let mut update = TranslationUpdate::default();
        let mut substitutions = BTreeMap::new();

        let mut sub_update = SubstitutionUpdate::default();
        sub_update.value = Some(Some("".to_string()));
        // No state set!

        substitutions.insert("userName".to_string(), Some(sub_update));
        update.substitutions = Some(substitutions);

        let result = store
            .upsert_translation("test.key", "en", update)
            .await
            .unwrap();

        // The substitution should be filtered out because it has no content
        assert!(result.substitutions.is_empty());

        // Verify it's not in the file
        let content = fs::read_to_string(&path).await.unwrap();
        assert!(!content.contains("\"substitutions\""));
    }

    #[tokio::test]
    async fn test_substitution_variations_with_state() {
        let temp = TempStorePath::new("test_substitution_variations");
        let path = temp.file.clone();

        // Create initial file with a substitution
        let initial_content = serde_json::json!({
            "sourceLanguage": "en",
            "version": "1.0",
            "strings": {
                "test.key": {
                    "localizations": {
                        "en": {
                            "stringUnit": {
                                "state": "translated",
                                "value": "You have %d messages"
                            },
                            "substitutions": {
                                "count": {
                                    "stringUnit": {
                                        "state": "translated",
                                        "value": "message count"
                                    },
                                    "argNum": 1,
                                    "formatSpecifier": "d"
                                }
                            }
                        }
                    }
                }
            }
        });

        fs::write(&path, initial_content.to_string()).await.unwrap();

        let store = XcStringsStore::load_or_create(path.clone()).await.unwrap();

        // Add plural variation to the substitution with state
        let mut update = TranslationUpdate::default();
        let mut substitutions = BTreeMap::new();

        let mut sub_update = SubstitutionUpdate::default();
        let mut variations = BTreeMap::new();
        let mut plural_cases = BTreeMap::new();

        let mut one_update = TranslationUpdate::default();
        one_update.value = Some(Some("".to_string()));
        one_update.state = Some(Some("new".to_string()));
        plural_cases.insert("one".to_string(), one_update);

        let mut other_update = TranslationUpdate::default();
        other_update.value = Some(Some("".to_string()));
        other_update.state = Some(Some("new".to_string()));
        plural_cases.insert("other".to_string(), other_update);

        variations.insert("plural".to_string(), plural_cases);
        sub_update.variations = Some(variations);

        substitutions.insert("count".to_string(), Some(sub_update));
        update.substitutions = Some(substitutions);

        let result = store
            .upsert_translation("test.key", "en", update)
            .await
            .unwrap();

        // Verify the substitution variations were added
        assert!(!result.substitutions.is_empty());
        let subs = &result.substitutions;
        assert!(subs.contains_key("count"));

        let count_sub = &subs["count"];
        assert!(!count_sub.variations.is_empty());
        assert!(count_sub.variations.contains_key("plural"));

        let plural_vars = &count_sub.variations["plural"];
        assert_eq!(plural_vars.len(), 2);
        assert!(plural_vars.contains_key("one"));
        assert!(plural_vars.contains_key("other"));

        // Check each variation has the correct state
        for (_, var) in plural_vars {
            assert_eq!(var.value, Some("".to_string()));
            assert_eq!(var.state, Some("new".to_string()));
        }

        // Verify it persists in the file
        let content = fs::read_to_string(&path).await.unwrap();
        assert!(content.contains("\"variations\""));
        assert!(content.contains("\"plural\""));
        assert!(content.contains("\"one\""));
        assert!(content.contains("\"other\""));
    }
}
