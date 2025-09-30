use std::{
    collections::{BTreeSet, HashMap},
    env, io,
    path::{Path, PathBuf},
    sync::Arc,
};

use indexmap::IndexMap;

use serde::{Deserialize, Serialize};
use serde_json::{self};
use thiserror::Error;
use tokio::{fs, sync::RwLock, task};

use crate::apple_json_formatter;

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
    #[error("language '{0}' not found")]
    LanguageMissing(String),
    #[error("language '{0}' already exists")]
    LanguageExists(String),
    #[error("invalid language: {0}")]
    InvalidLanguage(String),
    #[error("cannot remove source language '{0}'")]
    CannotRemoveSourceLanguage(String),
    #[error("cannot rename source language '{0}'")]
    CannotRenameSourceLanguage(String),
}

const DEFAULT_VERSION: &str = "1.0";
const DEFAULT_SOURCE_LANGUAGE: &str = "en";
const DEFAULT_TRANSLATION_STATE: &str = "translated";
const NEEDS_TRANSLATION_STATE: &str = "needs-translation";

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

#[derive(Debug, Clone)]
pub struct XcStringsFile {
    // Store the original JSON to preserve field order
    raw: IndexMap<String, serde_json::Value>,
    // Cached parsed values for easy access
    pub version: String,
    pub format_version: Option<FormatVersion>,
    pub source_language: String,
    pub strings: IndexMap<String, XcStringEntry>,
}

impl Default for XcStringsFile {
    fn default() -> Self {
        let mut raw = IndexMap::new();
        raw.insert(
            "version".to_string(),
            serde_json::Value::String(default_version()),
        );
        raw.insert(
            "sourceLanguage".to_string(),
            serde_json::Value::String(default_source_language()),
        );
        raw.insert(
            "strings".to_string(),
            serde_json::Value::Object(serde_json::Map::new()),
        );

        Self {
            raw,
            version: default_version(),
            format_version: None,
            source_language: default_source_language(),
            strings: IndexMap::new(),
        }
    }
}

impl XcStringsFile {
    fn from_json_value(value: serde_json::Value) -> Result<Self, serde_json::Error> {
        // Parse into IndexMap to preserve order
        let raw: IndexMap<String, serde_json::Value> = serde_json::from_value(value.clone())?;

        // Extract fields
        let version = raw
            .get("version")
            .and_then(|v| v.as_str())
            .unwrap_or("1.0")
            .to_string();

        let format_version = raw
            .get("formatVersion")
            .and_then(|v| serde_json::from_value(v.clone()).ok());

        let source_language = raw
            .get("sourceLanguage")
            .and_then(|v| v.as_str())
            .unwrap_or("en")
            .to_string();

        let strings = raw
            .get("strings")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();

        Ok(Self {
            raw,
            version,
            format_version,
            source_language,
            strings,
        })
    }

    fn to_json_value(&self) -> serde_json::Value {
        // Convert back using the preserved raw structure
        let mut raw = self.raw.clone();

        // Update the fields that may have changed
        raw.insert(
            "version".to_string(),
            serde_json::Value::String(self.version.clone()),
        );

        if let Some(ref fv) = self.format_version {
            raw.insert(
                "formatVersion".to_string(),
                serde_json::to_value(fv).unwrap(),
            );
        } else {
            raw.shift_remove("formatVersion");
        }

        raw.insert(
            "sourceLanguage".to_string(),
            serde_json::Value::String(self.source_language.clone()),
        );
        raw.insert(
            "strings".to_string(),
            serde_json::to_value(&self.strings).unwrap(),
        );

        serde_json::Value::Object(raw.into_iter().collect())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct XcStringEntry {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
    #[serde(rename = "extractionState", skip_serializing_if = "Option::is_none")]
    pub extraction_state: Option<String>,
    #[serde(
        rename = "localizations",
        default,
        skip_serializing_if = "IndexMap::is_empty"
    )]
    pub localizations: IndexMap<String, XcLocalization>,
    #[serde(rename = "shouldTranslate", skip_serializing_if = "Option::is_none")]
    pub should_translate: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct XcLocalization {
    #[serde(rename = "stringUnit", skip_serializing_if = "Option::is_none")]
    pub string_unit: Option<XcStringUnit>,
    #[serde(
        rename = "substitutions",
        default,
        skip_serializing_if = "IndexMap::is_empty"
    )]
    pub substitutions: IndexMap<String, XcSubstitution>,
    #[serde(
        rename = "variations",
        default,
        skip_serializing_if = "IndexMap::is_empty"
    )]
    pub variations: IndexMap<String, IndexMap<String, XcLocalization>>, // nesting mirrors xcstrings schema
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct XcSubstitution {
    #[serde(rename = "argNum", skip_serializing_if = "Option::is_none")]
    pub arg_num: Option<i64>,
    #[serde(rename = "formatSpecifier", skip_serializing_if = "Option::is_none")]
    pub format_specifier: Option<String>,
    #[serde(rename = "stringUnit", skip_serializing_if = "Option::is_none")]
    pub string_unit: Option<XcStringUnit>,
    #[serde(
        rename = "variations",
        default,
        skip_serializing_if = "IndexMap::is_empty"
    )]
    pub variations: IndexMap<String, IndexMap<String, XcLocalization>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct XcStringUnit {
    pub state: Option<String>,
    pub value: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TranslationValue {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub substitutions: IndexMap<String, SubstitutionValue>,
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub variations: IndexMap<String, IndexMap<String, TranslationValue>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TranslationUpdate {
    pub state: Option<Option<String>>,
    pub value: Option<Option<String>>,
    #[serde(default)]
    pub substitutions: Option<IndexMap<String, Option<SubstitutionUpdate>>>,
    #[serde(default)]
    pub variations: Option<IndexMap<String, IndexMap<String, TranslationUpdate>>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SubstitutionValue {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(rename = "argNum", skip_serializing_if = "Option::is_none")]
    pub arg_num: Option<i64>,
    #[serde(rename = "formatSpecifier", skip_serializing_if = "Option::is_none")]
    pub format_specifier: Option<String>,
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub variations: IndexMap<String, IndexMap<String, TranslationValue>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SubstitutionUpdate {
    pub state: Option<Option<String>>,
    pub value: Option<Option<String>>,
    #[serde(rename = "argNum", default)]
    pub arg_num: Option<Option<i64>>,
    #[serde(rename = "formatSpecifier", default)]
    pub format_specifier: Option<Option<String>>,
    #[serde(default)]
    pub variations: Option<IndexMap<String, IndexMap<String, TranslationUpdate>>>,
}

impl TranslationValue {
    fn from_localization(loc: &XcLocalization) -> Self {
        let state = loc.string_unit.as_ref().and_then(|u| u.state.clone());
        let value = loc.string_unit.as_ref().and_then(|u| u.value.clone());
        let substitutions = loc
            .substitutions
            .iter()
            .map(|(name, sub)| (name.clone(), SubstitutionValue::from_substitution(sub)))
            .collect();
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
            state,
            value,
            substitutions,
            variations,
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
            state: Some(normalized_state),
            value: Some(value),
            substitutions: None,
            variations: None,
        }
    }

    pub fn with_variations(
        mut self,
        variations: IndexMap<String, IndexMap<String, TranslationUpdate>>,
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
        let variations = self.variations.get_or_insert_with(IndexMap::new);
        let selector_entry = variations.entry(selector).or_insert_with(IndexMap::new);
        selector_entry.insert(case, update);
        self
    }
}

impl SubstitutionValue {
    fn from_substitution(sub: &XcSubstitution) -> Self {
        let state = sub.string_unit.as_ref().and_then(|unit| unit.state.clone());
        let value = sub.string_unit.as_ref().and_then(|unit| unit.value.clone());
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
            state,
            value,
            arg_num: sub.arg_num,
            format_specifier: sub.format_specifier.clone(),
            variations,
        }
    }
}

impl From<TranslationValue> for TranslationUpdate {
    fn from(value: TranslationValue) -> Self {
        let mut update = TranslationUpdate {
            state: Some(value.state),
            value: Some(value.value),
            substitutions: None,
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
    // Consider a unit as having content if it has either a value or an explicit state.
    // This keeps placeholders that mark work-in-progress translations (e.g. needs-translation).
    unit.value.is_some() || unit.state.is_some()
}

fn localization_is_empty(loc: &XcLocalization) -> bool {
    loc.string_unit
        .as_ref()
        .map(|unit| !string_unit_has_content(unit))
        .unwrap_or(true)
        && loc.variations.is_empty()
        && loc.substitutions.is_empty()
}

/// Context for variation validation to enforce schema constraints
#[derive(Debug, Clone, Copy, PartialEq)]
enum VariationContext {
    TopLevel,
    NestedUnderPlural,
    NestedUnderDevice,
}

/// Validates and normalizes variations according to xcstrings schema constraints:
/// - At top level: Cannot have both "plural" and "device" variations
/// - Nested under "plural": Cannot have "device" variations
/// - Nested under "device": Cannot have another "device" variation (but can have "plural")
fn validate_and_normalize_variations(
    variations: &mut IndexMap<String, IndexMap<String, XcLocalization>>,
    context: VariationContext,
) {
    // First, recursively normalize nested localizations
    for (selector, cases) in variations.iter_mut() {
        // Determine context for nested variations
        let nested_context = match (context, selector.as_str()) {
            (VariationContext::TopLevel, "plural") => VariationContext::NestedUnderPlural,
            (VariationContext::TopLevel, "device") => VariationContext::NestedUnderDevice,
            (VariationContext::NestedUnderDevice, "plural") => VariationContext::NestedUnderPlural,
            _ => context, // Other selectors maintain current context
        };

        cases.retain(|_, nested| {
            // Recursively normalize nested localizations
            !normalize_localization_inner(nested, nested_context)
        });
    }

    // Apply context-specific validation rules
    match context {
        VariationContext::TopLevel => {
            // Cannot have both "plural" and "device" at top level
            if variations.contains_key("plural") && variations.contains_key("device") {
                eprintln!("Warning: Invalid variation combination - cannot have both 'plural' and 'device' at top level. Removing 'device'.");
                variations.shift_remove("device");
            }
        }
        VariationContext::NestedUnderPlural => {
            // Cannot have "device" when nested under "plural"
            if variations.contains_key("device") {
                eprintln!("Warning: Invalid variation - cannot have 'device' nested under 'plural'. Removing 'device'.");
                variations.shift_remove("device");
            }
        }
        VariationContext::NestedUnderDevice => {
            // Cannot have another "device" when already nested under "device"
            if variations.contains_key("device") {
                eprintln!("Warning: Invalid variation - cannot have 'device' nested under another 'device'. Removing nested 'device'.");
                variations.shift_remove("device");
            }
        }
    }

    // Remove empty variation sets
    variations.retain(|_, cases| !cases.is_empty());
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

    // Validate and normalize variations (substitutions follow same rules as top-level)
    validate_and_normalize_variations(&mut sub.variations, VariationContext::TopLevel);

    substitution_is_empty(sub)
}

fn normalize_localization(loc: &mut XcLocalization) -> bool {
    normalize_localization_inner(loc, VariationContext::TopLevel)
}

fn normalize_localization_inner(loc: &mut XcLocalization, context: VariationContext) -> bool {
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

    // Validate and normalize variations with appropriate context
    validate_and_normalize_variations(&mut loc.variations, context);

    loc.substitutions
        .retain(|_, sub| !normalize_substitution(sub));

    localization_is_empty(loc)
}

fn placeholder_localization() -> XcLocalization {
    let mut loc = XcLocalization::default();
    loc.string_unit = Some(XcStringUnit {
        state: Some(NEEDS_TRANSLATION_STATE.to_string()),
        value: None,
    });
    loc
}

/// Extracts the main translation value from a localization.
/// Returns None if there's no string unit or no value.
fn extract_translation_value(loc: &XcLocalization) -> Option<String> {
    loc.string_unit.as_ref()?.value.clone()
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

    if let Some(state) = update.state {
        unit.state = state;
    }

    if let Some(value) = update.value {
        unit.value = value;
    }

    sanitize_string_unit(&mut unit);

    if string_unit_has_content(&unit) {
        target.string_unit = Some(unit);
    }

    if let Some(variations) = update.variations {
        let mut existing_variations = std::mem::take(&mut target.variations);

        for (selector, cases_update) in variations {
            let mut selector_entry = existing_variations
                .shift_remove(&selector)
                .unwrap_or_default();

            for (case_key, nested_update) in cases_update {
                let mut nested_loc = selector_entry
                    .shift_remove(&case_key)
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

        // Validate the resulting variations
        validate_and_normalize_variations(&mut target.variations, VariationContext::TopLevel);
    }

    if let Some(substitutions) = update.substitutions {
        let mut existing_substitutions = std::mem::take(&mut target.substitutions);

        for (name, maybe_update) in substitutions {
            match maybe_update {
                Some(sub_update) => {
                    let mut substitution = existing_substitutions
                        .shift_remove(&name)
                        .unwrap_or_else(XcSubstitution::default);
                    apply_substitution_update(&mut substitution, sub_update);

                    if !substitution_is_empty(&substitution) {
                        target.substitutions.insert(name, substitution);
                    }
                }
                None => {
                    existing_substitutions.shift_remove(&name);
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
            let mut selector_entry = existing_variations
                .shift_remove(&selector)
                .unwrap_or_default();

            for (case_key, nested_update) in cases_update {
                let mut nested_loc = selector_entry
                    .shift_remove(&case_key)
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

        // Validate the resulting variations for substitutions (same rules as TopLevel)
        validate_and_normalize_variations(&mut target.variations, VariationContext::TopLevel);
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
    pub translations: IndexMap<String, TranslationValue>,
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
            let value: serde_json::Value = serde_json::from_str(&raw)?;
            XcStringsFile::from_json_value(value)?
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
        let value: serde_json::Value = serde_json::from_str(&raw)?;
        let mut doc = XcStringsFile::from_json_value(value)?;
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

    /// Returns a map of languages to their untranslated keys.
    /// A translation is considered untranslated if:
    /// - The value is empty/None
    /// - No localization exists for that language
    pub async fn list_untranslated(&self) -> HashMap<String, Vec<String>> {
        let doc = self.data.read().await;
        let mut result: HashMap<String, Vec<String>> = HashMap::new();

        // Get all languages
        let mut langs: BTreeSet<String> = BTreeSet::new();
        langs.insert(doc.source_language.clone());
        for entry in doc.strings.values() {
            langs.extend(entry.localizations.keys().cloned());
        }

        // For each key, check which languages have untranslated values
        for (key, entry) in doc.strings.iter() {
            // Check each language for untranslated status
            for lang in langs.iter() {
                let is_untranslated = if let Some(localization) = entry.localizations.get(lang) {
                    match extract_translation_value(localization) {
                        None => true,                            // No value
                        Some(value) if value.is_empty() => true, // Empty value
                        Some(_) => false, // Has a value (even if same as other languages)
                    }
                } else {
                    true // No localization for this language
                };

                if is_untranslated {
                    result
                        .entry(lang.clone())
                        .or_insert_with(Vec::new)
                        .push(key.clone());
                }
            }
        }

        result
    }

    /// Returns a map of languages to their translation percentage (0-100)
    /// Keys marked as should_translate=false are excluded from the calculation
    /// A translation is considered complete if it has a non-empty value
    pub async fn get_translation_percentages(&self) -> HashMap<String, f64> {
        let doc = self.data.read().await;
        let mut result: HashMap<String, f64> = HashMap::new();

        // Get all languages
        let mut langs: BTreeSet<String> = BTreeSet::new();
        langs.insert(doc.source_language.clone());
        for entry in doc.strings.values() {
            langs.extend(entry.localizations.keys().cloned());
        }

        // Count only keys that should be translated
        let translatable_keys: Vec<&String> = doc
            .strings
            .iter()
            .filter(|(_, entry)| entry.should_translate.unwrap_or(true))
            .map(|(key, _)| key)
            .collect();

        if translatable_keys.is_empty() {
            return result;
        }

        let total_keys = translatable_keys.len() as f64;

        for lang in langs.iter() {
            let mut translated_count = 0;

            for key in translatable_keys.iter() {
                let entry = &doc.strings[*key];

                // Check if this language has a valid translation (non-empty value)
                let is_translated = if let Some(localization) = entry.localizations.get(lang) {
                    match extract_translation_value(localization) {
                        None => false,
                        Some(value) if value.is_empty() => false,
                        Some(_) => true, // Has a non-empty value
                    }
                } else {
                    false
                };

                if is_translated {
                    translated_count += 1;
                }
            }

            let percentage = (translated_count as f64 / total_keys) * 100.0;
            result.insert(lang.clone(), percentage);
        }

        result
    }

    pub async fn add_language(&self, language: &str) -> Result<(), StoreError> {
        let trimmed = language.trim();
        if trimmed.is_empty() {
            return Err(StoreError::InvalidLanguage(
                "Language code cannot be empty".to_string(),
            ));
        }
        let language = trimmed.to_string();

        let mut doc = self.data.write().await;

        // Check if language already exists
        let mut existing_langs: BTreeSet<String> = BTreeSet::new();
        existing_langs.insert(doc.source_language.clone());
        for entry in doc.strings.values() {
            existing_langs.extend(entry.localizations.keys().cloned());
        }

        if existing_langs.contains(&language) {
            return Err(StoreError::LanguageExists(language));
        }

        // Add placeholder localizations for the new language so editors can immediately
        // surface translation slots without clobbering existing values.
        for entry in doc.strings.values_mut() {
            entry
                .localizations
                .entry(language.clone())
                .or_insert_with(placeholder_localization);
        }

        normalize_strings_file(&mut doc);
        let json_value = doc.to_json_value();
        let serialized = apple_json_formatter::to_apple_format(&json_value);
        drop(doc);
        fs::write(&self.path, serialized).await?;
        Ok(())
    }

    pub async fn remove_language(&self, language: &str) -> Result<(), StoreError> {
        let trimmed = language.trim();
        if trimmed.is_empty() {
            return Err(StoreError::InvalidLanguage(
                "Language code cannot be empty".to_string(),
            ));
        }
        let language = trimmed.to_string();

        let mut doc = self.data.write().await;

        // Cannot remove the source language
        if language == doc.source_language {
            return Err(StoreError::CannotRemoveSourceLanguage(language));
        }

        // Check if language exists
        let mut language_exists = false;
        for entry in doc.strings.values() {
            if entry.localizations.contains_key(language.as_str()) {
                language_exists = true;
                break;
            }
        }

        if !language_exists {
            return Err(StoreError::LanguageMissing(language.clone()));
        }

        // Remove the language from all string entries
        for entry in doc.strings.values_mut() {
            entry.localizations.shift_remove(language.as_str());
        }

        // Remove any string entries that have no localizations left
        doc.strings
            .retain(|_, entry| !entry.localizations.is_empty());

        normalize_strings_file(&mut doc);
        let json_value = doc.to_json_value();
        let serialized = apple_json_formatter::to_apple_format(&json_value);
        drop(doc);
        fs::write(&self.path, serialized).await?;
        Ok(())
    }

    pub async fn update_language(
        &self,
        old_language: &str,
        new_language: &str,
    ) -> Result<(), StoreError> {
        let old_trimmed = old_language.trim();
        if old_trimmed.is_empty() {
            return Err(StoreError::InvalidLanguage(
                "Language code cannot be empty".to_string(),
            ));
        }
        let new_trimmed = new_language.trim();
        if new_trimmed.is_empty() {
            return Err(StoreError::InvalidLanguage(
                "Language code cannot be empty".to_string(),
            ));
        }

        if old_trimmed == new_trimmed {
            return Ok(()); // No change needed
        }

        let old_language = old_trimmed.to_string();
        let new_language = new_trimmed.to_string();

        let mut doc = self.data.write().await;

        // Cannot rename the source language
        if old_language == doc.source_language {
            return Err(StoreError::CannotRenameSourceLanguage(old_language));
        }

        // Check if old language exists
        let mut old_language_exists = false;
        for entry in doc.strings.values() {
            if entry.localizations.contains_key(old_language.as_str()) {
                old_language_exists = true;
                break;
            }
        }

        if !old_language_exists {
            return Err(StoreError::LanguageMissing(old_language));
        }

        // Check if new language already exists
        let mut new_language_exists = false;
        for entry in doc.strings.values() {
            if entry.localizations.contains_key(new_language.as_str()) {
                new_language_exists = true;
                break;
            }
        }

        if new_language_exists {
            return Err(StoreError::LanguageExists(new_language.clone()));
        }

        // Rename the language in all string entries
        for entry in doc.strings.values_mut() {
            if let Some(localization) = entry.localizations.shift_remove(old_language.as_str()) {
                entry
                    .localizations
                    .insert(new_language.clone(), localization);
            }
        }

        normalize_strings_file(&mut doc);
        let json_value = doc.to_json_value();
        let serialized = apple_json_formatter::to_apple_format(&json_value);
        drop(doc);
        fs::write(&self.path, serialized).await?;
        Ok(())
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
        let json_value = doc.to_json_value();
        let serialized = apple_json_formatter::to_apple_format(&json_value);
        drop(doc);
        fs::write(&self.path, serialized).await?;

        Ok(updated)
    }

    pub async fn delete_translation(&self, key: &str, language: &str) -> Result<(), StoreError> {
        let mut doc = self.data.write().await;
        let translation_exists = if let Some(entry) = doc.strings.get_mut(key) {
            if entry.localizations.shift_remove(language).is_some() {
                if entry.localizations.is_empty() {
                    doc.strings.shift_remove(key);
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
        let json_value = doc.to_json_value();
        let serialized = apple_json_formatter::to_apple_format(&json_value);
        drop(doc);
        fs::write(&self.path, serialized).await?;
        Ok(())
    }

    pub async fn delete_key(&self, key: &str) -> Result<(), StoreError> {
        let mut doc = self.data.write().await;
        if doc.strings.shift_remove(key).is_none() {
            return Err(StoreError::KeyMissing(key.to_string()));
        }
        normalize_strings_file(&mut doc);
        let json_value = doc.to_json_value();
        let serialized = apple_json_formatter::to_apple_format(&json_value);
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
            .shift_remove(old_key)
            .ok_or_else(|| StoreError::KeyMissing(old_key.to_string()))?;

        doc.strings.insert(new_key.to_string(), entry);

        normalize_strings_file(&mut doc);
        let json_value = doc.to_json_value();
        let serialized = apple_json_formatter::to_apple_format(&json_value);
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
        let json_value = doc.to_json_value();
        let serialized = apple_json_formatter::to_apple_format(&json_value);
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
        let json_value = doc.to_json_value();
        let serialized = apple_json_formatter::to_apple_format(&json_value);
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
        let json_value = doc.to_json_value();
        let serialized = apple_json_formatter::to_apple_format(&json_value);
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
        let mut substitutions = IndexMap::new();
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
        let mut removal_map = IndexMap::new();
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
        assert_eq!(summary.languages, vec!["en".to_string(), "de".to_string()]);
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
        let mut substitutions = IndexMap::new();

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
        let mut variations = IndexMap::new();
        let mut plural_cases = IndexMap::new();

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
        let mut substitutions = IndexMap::new();

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
        let mut substitutions = IndexMap::new();

        let mut sub_update = SubstitutionUpdate::default();
        let mut variations = IndexMap::new();
        let mut plural_cases = IndexMap::new();

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
        assert!(content.contains("\"variations\""));
        assert!(content.contains("\"plural\""));
    }

    #[tokio::test]
    async fn test_variation_constraints_top_level_plural_and_device() {
        // Test that plural and device cannot coexist at top level
        let tmp = TempStorePath::new("variation_constraints_top");
        let store = XcStringsStore::load_or_create(&tmp.file).await.unwrap();

        // Try to create a translation with both plural and device at top level
        let mut update = TranslationUpdate::default();
        let mut variations = IndexMap::new();

        // Add plural variations
        let mut plural_cases = IndexMap::new();
        plural_cases.insert(
            "one".to_string(),
            XcLocalization {
                string_unit: Some(XcStringUnit {
                    state: Some("translated".to_string()),
                    value: Some("One item".to_string()),
                }),
                variations: IndexMap::new(),
                substitutions: IndexMap::new(),
            },
        );
        plural_cases.insert(
            "other".to_string(),
            XcLocalization {
                string_unit: Some(XcStringUnit {
                    state: Some("translated".to_string()),
                    value: Some("Many items".to_string()),
                }),
                variations: IndexMap::new(),
                substitutions: IndexMap::new(),
            },
        );
        variations.insert("plural".to_string(), plural_cases);

        // Add device variations (should be rejected)
        let mut device_cases = IndexMap::new();
        device_cases.insert(
            "iphone".to_string(),
            XcLocalization {
                string_unit: Some(XcStringUnit {
                    state: Some("translated".to_string()),
                    value: Some("iPhone version".to_string()),
                }),
                variations: IndexMap::new(),
                substitutions: IndexMap::new(),
            },
        );
        variations.insert("device".to_string(), device_cases);

        update.variations = Some(
            variations
                .into_iter()
                .map(|(k, v)| {
                    let cases = v
                        .into_iter()
                        .map(|(case_key, loc)| {
                            (
                                case_key,
                                TranslationUpdate::from(TranslationValue::from_localization(&loc)),
                            )
                        })
                        .collect();
                    (k, cases)
                })
                .collect(),
        );

        let result = store
            .upsert_translation("test.key", "en", update)
            .await
            .unwrap();

        // Verify that only plural remains (device should be removed)
        assert!(result.variations.contains_key("plural"));
        assert!(!result.variations.contains_key("device"));
    }

    #[tokio::test]
    async fn test_variation_constraints_no_device_under_plural() {
        // Test that device cannot be nested under plural
        let tmp = TempStorePath::new("variation_constraints_nested_plural");
        let store = XcStringsStore::load_or_create(&tmp.file).await.unwrap();

        // Create a translation with device nested under plural (should be rejected)
        let mut update = TranslationUpdate::default();
        let mut variations = IndexMap::new();

        let mut plural_cases = IndexMap::new();
        let mut one_loc = XcLocalization::default();
        one_loc.string_unit = Some(XcStringUnit {
            state: Some("translated".to_string()),
            value: Some("One".to_string()),
        });

        // Try to add device variation under plural/one (should be rejected)
        let mut device_cases = IndexMap::new();
        device_cases.insert(
            "iphone".to_string(),
            XcLocalization {
                string_unit: Some(XcStringUnit {
                    state: Some("translated".to_string()),
                    value: Some("iPhone One".to_string()),
                }),
                variations: IndexMap::new(),
                substitutions: IndexMap::new(),
            },
        );
        one_loc
            .variations
            .insert("device".to_string(), device_cases);

        plural_cases.insert("one".to_string(), one_loc);
        variations.insert("plural".to_string(), plural_cases);

        update.variations = Some(
            variations
                .into_iter()
                .map(|(k, v)| {
                    let cases = v
                        .into_iter()
                        .map(|(case_key, loc)| {
                            (
                                case_key,
                                TranslationUpdate::from(TranslationValue::from_localization(&loc)),
                            )
                        })
                        .collect();
                    (k, cases)
                })
                .collect(),
        );

        let result = store
            .upsert_translation("test.key2", "en", update)
            .await
            .unwrap();

        // Verify that device was removed from under plural
        let plural_vars = result.variations.get("plural").unwrap();
        let one_var = plural_vars.get("one").unwrap();
        assert!(!one_var.variations.contains_key("device"));
    }

    #[tokio::test]
    async fn test_variation_constraints_no_device_under_device() {
        // Test that device cannot be nested under another device
        let tmp = TempStorePath::new("variation_constraints_nested_device");
        let store = XcStringsStore::load_or_create(&tmp.file).await.unwrap();

        // Create a translation with device nested under device (should be rejected)
        let mut update = TranslationUpdate::default();
        let mut variations = IndexMap::new();

        let mut device_cases = IndexMap::new();
        let mut iphone_loc = XcLocalization::default();
        iphone_loc.string_unit = Some(XcStringUnit {
            state: Some("translated".to_string()),
            value: Some("iPhone".to_string()),
        });

        // Try to add another device variation under device/iphone (should be rejected)
        let mut nested_device = IndexMap::new();
        nested_device.insert(
            "ipad".to_string(),
            XcLocalization {
                string_unit: Some(XcStringUnit {
                    state: Some("translated".to_string()),
                    value: Some("Nested iPad".to_string()),
                }),
                variations: IndexMap::new(),
                substitutions: IndexMap::new(),
            },
        );
        iphone_loc
            .variations
            .insert("device".to_string(), nested_device);

        device_cases.insert("iphone".to_string(), iphone_loc);
        variations.insert("device".to_string(), device_cases);

        update.variations = Some(
            variations
                .into_iter()
                .map(|(k, v)| {
                    let cases = v
                        .into_iter()
                        .map(|(case_key, loc)| {
                            (
                                case_key,
                                TranslationUpdate::from(TranslationValue::from_localization(&loc)),
                            )
                        })
                        .collect();
                    (k, cases)
                })
                .collect(),
        );

        let result = store
            .upsert_translation("test.key3", "en", update)
            .await
            .unwrap();

        // Verify that nested device was removed
        let device_vars = result.variations.get("device").unwrap();
        let iphone_var = device_vars.get("iphone").unwrap();
        assert!(!iphone_var.variations.contains_key("device"));
    }

    #[tokio::test]
    async fn test_format_preservation() {
        // Test that we preserve Apple's JSON format with spaces before colons
        let tmp = TempStorePath::new("format_preservation");

        // Create initial file with Apple format
        let initial_content = r#"{
  "version" : "1.0",
  "sourceLanguage" : "en",
  "strings" : {
    "first.key" : {
      "localizations" : {
        "en" : {
          "stringUnit" : {
            "state" : "translated",
            "value" : "First value"
          }
        }
      }
    },
    "second.key" : {
      "localizations" : {
        "en" : {
          "stringUnit" : {
            "state" : "translated",
            "value" : "Second value"
          }
        }
      }
    }
  }
}"#;

        fs::write(&tmp.file, initial_content).await.unwrap();

        // Load the store
        let store = XcStringsStore::load_or_create(&tmp.file).await.unwrap();

        // Make a small change - add a third key (should preserve order and format)
        store
            .upsert_translation(
                "third.key",
                "en",
                TranslationUpdate::from_value_state(
                    Some("Third value".into()),
                    Some("translated".into()),
                ),
            )
            .await
            .unwrap();

        // Read the file back
        let updated_content = fs::read_to_string(&tmp.file).await.unwrap();

        // Check that format is preserved (spaces before colons)
        assert!(updated_content.contains("\"version\" : \"1.0\""));
        assert!(updated_content.contains("\"sourceLanguage\" : \"en\""));
        assert!(updated_content.contains("\"first.key\" : {"));
        assert!(updated_content.contains("\"second.key\" : {"));
        assert!(updated_content.contains("\"third.key\" : {"));
        assert!(updated_content.contains("\"state\" : \"translated\""));

        // Check that order is preserved (first.key still comes before second.key)
        let first_pos = updated_content.find("\"first.key\"").unwrap();
        let second_pos = updated_content.find("\"second.key\"").unwrap();
        let third_pos = updated_content.find("\"third.key\"").unwrap();
        assert!(first_pos < second_pos);
        assert!(second_pos < third_pos);

        // Update existing key - should maintain position
        store
            .upsert_translation(
                "first.key",
                "en",
                TranslationUpdate::from_value_state(Some("Updated first value".into()), None),
            )
            .await
            .unwrap();

        let updated_content2 = fs::read_to_string(&tmp.file).await.unwrap();

        // Check order is still preserved after update
        let first_pos2 = updated_content2.find("\"first.key\"").unwrap();
        let second_pos2 = updated_content2.find("\"second.key\"").unwrap();
        let third_pos2 = updated_content2.find("\"third.key\"").unwrap();
        assert!(first_pos2 < second_pos2);
        assert!(second_pos2 < third_pos2);
        assert!(updated_content2.contains("\"value\" : \"Updated first value\""));
    }

    #[tokio::test]
    async fn test_variation_constraints_plural_allowed_under_device() {
        // Test that plural IS allowed under device
        let tmp = TempStorePath::new("variation_constraints_plural_under_device");
        let store = XcStringsStore::load_or_create(&tmp.file).await.unwrap();

        // Create a translation with plural nested under device (should be allowed)
        let mut update = TranslationUpdate::default();
        let mut variations = IndexMap::new();

        let mut device_cases = IndexMap::new();
        let mut iphone_loc = XcLocalization::default();

        // Add plural variation under device/iphone (should be allowed)
        let mut plural_cases = IndexMap::new();
        plural_cases.insert(
            "one".to_string(),
            XcLocalization {
                string_unit: Some(XcStringUnit {
                    state: Some("translated".to_string()),
                    value: Some("One item on iPhone".to_string()),
                }),
                variations: IndexMap::new(),
                substitutions: IndexMap::new(),
            },
        );
        plural_cases.insert(
            "other".to_string(),
            XcLocalization {
                string_unit: Some(XcStringUnit {
                    state: Some("translated".to_string()),
                    value: Some("Many items on iPhone".to_string()),
                }),
                variations: IndexMap::new(),
                substitutions: IndexMap::new(),
            },
        );
        iphone_loc
            .variations
            .insert("plural".to_string(), plural_cases);

        device_cases.insert("iphone".to_string(), iphone_loc);
        variations.insert("device".to_string(), device_cases);

        update.variations = Some(
            variations
                .into_iter()
                .map(|(k, v)| {
                    let cases = v
                        .into_iter()
                        .map(|(case_key, loc)| {
                            (
                                case_key,
                                TranslationUpdate::from(TranslationValue::from_localization(&loc)),
                            )
                        })
                        .collect();
                    (k, cases)
                })
                .collect(),
        );

        let result = store
            .upsert_translation("test.key4", "en", update)
            .await
            .unwrap();

        // Verify that plural under device was preserved
        let device_vars = result.variations.get("device").unwrap();
        let iphone_var = device_vars.get("iphone").unwrap();
        assert!(iphone_var.variations.contains_key("plural"));
        let plural_vars = iphone_var.variations.get("plural").unwrap();
        assert!(plural_vars.contains_key("one"));
        assert!(plural_vars.contains_key("other"));
    }

    #[tokio::test]
    async fn delete_plural_variation_with_null_value() {
        let tmp = TempStorePath::new("delete_plural_null");
        let store = XcStringsStore::load_or_create(&tmp.file)
            .await
            .expect("load store");

        // First, create a translation with plural variations
        let initial = TranslationUpdate::from_value_state(None, None)
            .add_variation(
                "plural",
                "one",
                TranslationUpdate::from_value_state(
                    Some("One item".into()),
                    Some("translated".into()),
                ),
            )
            .add_variation(
                "plural",
                "other",
                TranslationUpdate::from_value_state(
                    Some("%d items".into()),
                    Some("translated".into()),
                ),
            );

        store
            .upsert_translation("items.count", "en", initial)
            .await
            .expect("create initial");

        // Verify both plural forms exist
        let result = store
            .get_translation("items.count", "en")
            .await
            .expect("fetch initial")
            .expect("translation exists");

        let plural_vars = result.variations.get("plural").expect("has plural");
        assert_eq!(plural_vars.len(), 2);
        assert!(plural_vars.contains_key("one"));
        assert!(plural_vars.contains_key("other"));

        // Now delete the "one" case by setting value to None
        let delete_one = TranslationUpdate {
            state: None,
            value: None,
            variations: Some({
                let mut variations = IndexMap::new();
                let mut plural_cases = IndexMap::new();
                plural_cases.insert(
                    "one".to_string(),
                    TranslationUpdate {
                        state: Some(None),
                        value: Some(None), // Explicitly set to None to delete
                        substitutions: None,
                        variations: None,
                    },
                );
                variations.insert("plural".to_string(), plural_cases);
                variations
            }),
            substitutions: None,
        };

        store
            .upsert_translation("items.count", "en", delete_one)
            .await
            .expect("delete one case");

        // Verify only "other" case remains
        let result = store
            .get_translation("items.count", "en")
            .await
            .expect("fetch after delete")
            .expect("translation still exists");

        let plural_vars = result.variations.get("plural").expect("still has plural");
        assert_eq!(
            plural_vars.len(),
            1,
            "Should have only one plural case left"
        );
        assert!(
            !plural_vars.contains_key("one"),
            "One case should be deleted"
        );
        assert!(
            plural_vars.contains_key("other"),
            "Other case should remain"
        );
    }

    #[tokio::test]
    async fn add_language_succeeds_and_ready_for_translations() {
        let tmp = TempStorePath::new("add_language");
        let store = XcStringsStore::load_or_create(&tmp.file).await.unwrap();

        // Add some initial translations
        store
            .upsert_translation(
                "greeting",
                "en",
                TranslationUpdate::from_value_state(Some("Hello".into()), None),
            )
            .await
            .unwrap();

        // Add a new language (creates placeholder entries immediately)
        store.add_language("fr").await.unwrap();

        let languages = store.list_languages().await;
        assert!(languages.contains(&"fr".to_string()));

        // Placeholder should exist with needs-translation state and no value yet
        let placeholder = store
            .get_translation("greeting", "fr")
            .await
            .expect("lookup succeeds")
            .expect("placeholder created");
        assert_eq!(placeholder.state.as_deref(), Some(NEEDS_TRANSLATION_STATE));
        assert!(placeholder.value.is_none());

        // Update translation for this language
        store
            .upsert_translation(
                "greeting",
                "fr",
                TranslationUpdate::from_value_state(Some("Bonjour".into()), None),
            )
            .await
            .unwrap();

        // Now the language still appears and has the translated value
        let languages = store.list_languages().await;
        assert!(languages.contains(&"fr".to_string()));

        let greeting = store
            .get_translation("greeting", "fr")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(greeting.value.as_deref(), Some("Bonjour"));
        assert_eq!(greeting.state.as_deref(), Some(DEFAULT_TRANSLATION_STATE));
    }

    #[tokio::test]
    async fn add_language_to_empty_file_succeeds_but_not_visible() {
        let tmp = TempStorePath::new("add_language_empty");
        let store = XcStringsStore::load_or_create(&tmp.file).await.unwrap();

        // Add a language to an empty file
        store.add_language("fr").await.unwrap();

        // With no strings present, there's nothing to attach placeholders to yet
        let languages = store.list_languages().await;
        assert!(!languages.contains(&"fr".to_string()));
        assert!(languages.contains(&"en".to_string())); // Source language is always present

        // But if we add a translation, the language will appear
        store
            .upsert_translation(
                "greeting",
                "fr",
                TranslationUpdate::from_value_state(Some("Bonjour".into()), None),
            )
            .await
            .unwrap();

        let languages = store.list_languages().await;
        assert!(languages.contains(&"fr".to_string()));
    }

    #[tokio::test]
    async fn add_language_fails_if_already_exists() {
        let tmp = TempStorePath::new("add_language_exists");
        let store = XcStringsStore::load_or_create(&tmp.file).await.unwrap();

        // Add some initial translations
        store
            .upsert_translation(
                "greeting",
                "en",
                TranslationUpdate::from_value_state(Some("Hello".into()), None),
            )
            .await
            .unwrap();

        // Try to add English again (source language)
        let result = store.add_language("en").await;
        assert!(matches!(result, Err(StoreError::LanguageExists(_))));

        // Add French translation (not just add language)
        store
            .upsert_translation(
                "greeting",
                "fr",
                TranslationUpdate::from_value_state(Some("Bonjour".into()), None),
            )
            .await
            .unwrap();

        // Try to add French again (now it exists because it has translations)
        let result = store.add_language("fr").await;
        assert!(matches!(result, Err(StoreError::LanguageExists(_))));
    }

    #[tokio::test]
    async fn add_language_fails_if_empty() {
        let tmp = TempStorePath::new("add_language_empty");
        let store = XcStringsStore::load_or_create(&tmp.file).await.unwrap();

        let result = store.add_language("").await;
        assert!(matches!(result, Err(StoreError::InvalidLanguage(_))));

        let result = store.add_language("   ").await;
        assert!(matches!(result, Err(StoreError::InvalidLanguage(_))));
    }

    #[tokio::test]
    async fn remove_language_deletes_localizations() {
        let tmp = TempStorePath::new("remove_language");
        let store = XcStringsStore::load_or_create(&tmp.file).await.unwrap();

        // Add translations in multiple languages
        store
            .upsert_translation(
                "greeting",
                "en",
                TranslationUpdate::from_value_state(Some("Hello".into()), None),
            )
            .await
            .unwrap();

        store
            .upsert_translation(
                "greeting",
                "fr",
                TranslationUpdate::from_value_state(Some("Bonjour".into()), None),
            )
            .await
            .unwrap();

        store
            .upsert_translation(
                "greeting",
                "es",
                TranslationUpdate::from_value_state(Some("Hola".into()), None),
            )
            .await
            .unwrap();

        // Remove French
        store.remove_language("fr").await.unwrap();

        // Verify French was removed
        let languages = store.list_languages().await;
        assert!(!languages.contains(&"fr".to_string()));
        assert!(languages.contains(&"en".to_string()));
        assert!(languages.contains(&"es".to_string()));

        let greeting_fr = store.get_translation("greeting", "fr").await.unwrap();
        assert!(greeting_fr.is_none());

        let greeting_en = store.get_translation("greeting", "en").await.unwrap();
        assert!(greeting_en.is_some());
        assert_eq!(greeting_en.unwrap().value.as_deref(), Some("Hello"));
    }

    #[tokio::test]
    async fn remove_language_fails_if_source_language() {
        let tmp = TempStorePath::new("remove_source_language");
        let store = XcStringsStore::load_or_create(&tmp.file).await.unwrap();

        let result = store.remove_language("en").await;
        assert!(matches!(
            result,
            Err(StoreError::CannotRemoveSourceLanguage(_))
        ));
    }

    #[tokio::test]
    async fn remove_language_fails_if_not_exists() {
        let tmp = TempStorePath::new("remove_language_missing");
        let store = XcStringsStore::load_or_create(&tmp.file).await.unwrap();

        let result = store.remove_language("fr").await;
        assert!(matches!(result, Err(StoreError::LanguageMissing(_))));
    }

    #[tokio::test]
    async fn update_language_renames_successfully() {
        let tmp = TempStorePath::new("update_language");
        let store = XcStringsStore::load_or_create(&tmp.file).await.unwrap();

        // Add translations
        store
            .upsert_translation(
                "greeting",
                "en",
                TranslationUpdate::from_value_state(Some("Hello".into()), None),
            )
            .await
            .unwrap();

        store
            .upsert_translation(
                "greeting",
                "fr",
                TranslationUpdate::from_value_state(Some("Bonjour".into()), None),
            )
            .await
            .unwrap();

        // Rename French to French-France
        store.update_language("fr", "fr-FR").await.unwrap();

        // Verify the rename
        let languages = store.list_languages().await;
        assert!(!languages.contains(&"fr".to_string()));
        assert!(languages.contains(&"fr-FR".to_string()));

        let greeting_fr = store.get_translation("greeting", "fr").await.unwrap();
        assert!(greeting_fr.is_none());

        let greeting_fr_fr = store.get_translation("greeting", "fr-FR").await.unwrap();
        assert!(greeting_fr_fr.is_some());
        assert_eq!(greeting_fr_fr.unwrap().value.as_deref(), Some("Bonjour"));
    }

    #[tokio::test]
    async fn update_language_fails_if_source_language() {
        let tmp = TempStorePath::new("update_source_language");
        let store = XcStringsStore::load_or_create(&tmp.file).await.unwrap();

        let result = store.update_language("en", "en-US").await;
        assert!(matches!(
            result,
            Err(StoreError::CannotRenameSourceLanguage(_))
        ));
    }

    #[tokio::test]
    async fn update_language_fails_if_old_not_exists() {
        let tmp = TempStorePath::new("update_language_missing");
        let store = XcStringsStore::load_or_create(&tmp.file).await.unwrap();

        let result = store.update_language("fr", "fr-FR").await;
        assert!(matches!(result, Err(StoreError::LanguageMissing(_))));
    }

    #[tokio::test]
    async fn update_language_fails_if_new_exists() {
        let tmp = TempStorePath::new("update_language_exists");
        let store = XcStringsStore::load_or_create(&tmp.file).await.unwrap();

        // Add translations
        store
            .upsert_translation(
                "greeting",
                "fr",
                TranslationUpdate::from_value_state(Some("Bonjour".into()), None),
            )
            .await
            .unwrap();

        store
            .upsert_translation(
                "greeting",
                "es",
                TranslationUpdate::from_value_state(Some("Hola".into()), None),
            )
            .await
            .unwrap();

        // Try to rename French to Spanish (which already exists)
        let result = store.update_language("fr", "es").await;
        assert!(matches!(result, Err(StoreError::LanguageExists(_))));
    }

    #[tokio::test]
    async fn update_language_no_op_if_same_name() {
        let tmp = TempStorePath::new("update_language_same");
        let store = XcStringsStore::load_or_create(&tmp.file).await.unwrap();

        // Add translation
        store
            .upsert_translation(
                "greeting",
                "fr",
                TranslationUpdate::from_value_state(Some("Bonjour".into()), None),
            )
            .await
            .unwrap();

        // "Rename" to the same name
        let result = store.update_language("fr", "fr").await;
        assert!(result.is_ok());

        // Verify nothing changed
        let greeting_fr = store.get_translation("greeting", "fr").await.unwrap();
        assert!(greeting_fr.is_some());
        assert_eq!(greeting_fr.unwrap().value.as_deref(), Some("Bonjour"));
    }

    #[tokio::test]
    async fn list_untranslated_with_empty_values() {
        let tmp = TempStorePath::new("list_untranslated_empty");
        let store = XcStringsStore::load_or_create(&tmp.file).await.unwrap();

        // Add translations - some with missing/no value
        store
            .upsert_translation(
                "key1",
                "en",
                TranslationUpdate::from_value_state(Some("Hello".into()), None),
            )
            .await
            .unwrap();

        store
            .upsert_translation(
                "key2",
                "en",
                TranslationUpdate::from_value_state(Some("World".into()), None),
            )
            .await
            .unwrap();

        store
            .upsert_translation(
                "key2",
                "fr",
                TranslationUpdate::from_value_state(Some("Monde".into()), None),
            )
            .await
            .unwrap();

        // key1 has no French translation at all

        let untranslated = store.list_untranslated().await;

        // French should have key1 as untranslated (missing)
        let fr_untranslated = untranslated.get("fr");
        assert!(fr_untranslated.is_some());
        let fr_keys = fr_untranslated.unwrap();
        assert_eq!(fr_keys.len(), 1);
        assert!(fr_keys.contains(&"key1".to_string()));

        // English should have no untranslated keys
        let en_untranslated = untranslated.get("en");
        if let Some(keys) = en_untranslated {
            assert!(keys.is_empty());
        }
    }

    #[tokio::test]
    async fn list_untranslated_with_duplicate_values() {
        let tmp = TempStorePath::new("list_untranslated_duplicates");
        let store = XcStringsStore::load_or_create(&tmp.file).await.unwrap();

        // Add translations where French has the same value as English
        // This is now considered translated (duplicates are allowed)
        store
            .upsert_translation(
                "key1",
                "en",
                TranslationUpdate::from_value_state(Some("Hello".into()), None),
            )
            .await
            .unwrap();

        store
            .upsert_translation(
                "key1",
                "fr",
                TranslationUpdate::from_value_state(Some("Hello".into()), None), // Same as English - now OK
            )
            .await
            .unwrap();

        store
            .upsert_translation(
                "key2",
                "en",
                TranslationUpdate::from_value_state(Some("World".into()), None),
            )
            .await
            .unwrap();

        store
            .upsert_translation(
                "key2",
                "fr",
                TranslationUpdate::from_value_state(Some("Monde".into()), None), // Properly translated
            )
            .await
            .unwrap();

        let untranslated = store.list_untranslated().await;

        // Both languages should have no untranslated keys (duplicates are now allowed)
        let fr_untranslated = untranslated.get("fr");
        if let Some(keys) = fr_untranslated {
            assert!(keys.is_empty());
        }

        let en_untranslated = untranslated.get("en");
        if let Some(keys) = en_untranslated {
            assert!(keys.is_empty());
        }
    }

    #[tokio::test]
    async fn list_untranslated_with_no_translations() {
        let tmp = TempStorePath::new("list_untranslated_none");
        let store = XcStringsStore::load_or_create(&tmp.file).await.unwrap();

        // Empty store
        let untranslated = store.list_untranslated().await;

        // Should only have source language with no untranslated keys
        assert!(untranslated.is_empty() || untranslated.get("en").unwrap().is_empty());
    }

    #[tokio::test]
    async fn list_untranslated_with_all_translated() {
        let tmp = TempStorePath::new("list_untranslated_all_done");
        let store = XcStringsStore::load_or_create(&tmp.file).await.unwrap();

        // Add fully translated keys
        store
            .upsert_translation(
                "key1",
                "en",
                TranslationUpdate::from_value_state(Some("Hello".into()), None),
            )
            .await
            .unwrap();

        store
            .upsert_translation(
                "key1",
                "fr",
                TranslationUpdate::from_value_state(Some("Bonjour".into()), None),
            )
            .await
            .unwrap();

        store
            .upsert_translation(
                "key2",
                "en",
                TranslationUpdate::from_value_state(Some("World".into()), None),
            )
            .await
            .unwrap();

        store
            .upsert_translation(
                "key2",
                "fr",
                TranslationUpdate::from_value_state(Some("Monde".into()), None),
            )
            .await
            .unwrap();

        let untranslated = store.list_untranslated().await;

        // All languages should have no untranslated keys
        for (_, keys) in untranslated.iter() {
            assert!(keys.is_empty());
        }
    }

    #[tokio::test]
    async fn get_translation_percentages_empty_store() {
        let tmp = TempStorePath::new("percentages_empty");
        let store = XcStringsStore::load_or_create(&tmp.file).await.unwrap();

        let percentages = store.get_translation_percentages().await;

        // Empty store should return empty map
        assert!(percentages.is_empty());
    }

    #[tokio::test]
    async fn get_translation_percentages_partial_translation() {
        let tmp = TempStorePath::new("percentages_partial");
        let store = XcStringsStore::load_or_create(&tmp.file).await.unwrap();

        // Add 4 keys
        store
            .upsert_translation(
                "key1",
                "en",
                TranslationUpdate::from_value_state(Some("Hello".into()), None),
            )
            .await
            .unwrap();

        store
            .upsert_translation(
                "key2",
                "en",
                TranslationUpdate::from_value_state(Some("World".into()), None),
            )
            .await
            .unwrap();

        store
            .upsert_translation(
                "key3",
                "en",
                TranslationUpdate::from_value_state(Some("Foo".into()), None),
            )
            .await
            .unwrap();

        store
            .upsert_translation(
                "key4",
                "en",
                TranslationUpdate::from_value_state(Some("Bar".into()), None),
            )
            .await
            .unwrap();

        // French: 3 translated (including duplicate), 1 missing (key3 will be filtered as empty)
        store
            .upsert_translation(
                "key1",
                "fr",
                TranslationUpdate::from_value_state(Some("Bonjour".into()), None),
            )
            .await
            .unwrap();

        store
            .upsert_translation(
                "key2",
                "fr",
                TranslationUpdate::from_value_state(Some("Monde".into()), None),
            )
            .await
            .unwrap();

        // key3: no French translation (empty will be filtered out by normalization)

        store
            .upsert_translation(
                "key4",
                "fr",
                TranslationUpdate::from_value_state(Some("Bar".into()), None), // Duplicate - now OK
            )
            .await
            .unwrap();

        let percentages = store.get_translation_percentages().await;

        // English should be 100% (all 4 keys have values)
        let en_percentage = percentages.get("en").unwrap();
        assert_eq!(*en_percentage, 100.0);

        // French should be 75% (3 out of 4, key3 is missing)
        let fr_percentage = percentages.get("fr").unwrap();
        assert_eq!(*fr_percentage, 75.0);
    }

    #[tokio::test]
    async fn get_translation_percentages_fully_translated() {
        let tmp = TempStorePath::new("percentages_full");
        let store = XcStringsStore::load_or_create(&tmp.file).await.unwrap();

        // Add fully translated keys
        store
            .upsert_translation(
                "key1",
                "en",
                TranslationUpdate::from_value_state(Some("Hello".into()), None),
            )
            .await
            .unwrap();

        store
            .upsert_translation(
                "key1",
                "fr",
                TranslationUpdate::from_value_state(Some("Bonjour".into()), None),
            )
            .await
            .unwrap();

        store
            .upsert_translation(
                "key2",
                "en",
                TranslationUpdate::from_value_state(Some("World".into()), None),
            )
            .await
            .unwrap();

        store
            .upsert_translation(
                "key2",
                "fr",
                TranslationUpdate::from_value_state(Some("Monde".into()), None),
            )
            .await
            .unwrap();

        let percentages = store.get_translation_percentages().await;

        // Both languages should be 100%
        let en_percentage = percentages.get("en").unwrap();
        assert_eq!(*en_percentage, 100.0);

        let fr_percentage = percentages.get("fr").unwrap();
        assert_eq!(*fr_percentage, 100.0);
    }

    #[tokio::test]
    async fn get_translation_percentages_multiple_languages() {
        let tmp = TempStorePath::new("percentages_multi");
        let store = XcStringsStore::load_or_create(&tmp.file).await.unwrap();

        // Add 2 keys
        store
            .upsert_translation(
                "key1",
                "en",
                TranslationUpdate::from_value_state(Some("Hello".into()), None),
            )
            .await
            .unwrap();

        store
            .upsert_translation(
                "key2",
                "en",
                TranslationUpdate::from_value_state(Some("World".into()), None),
            )
            .await
            .unwrap();

        // French: 1 translated, 1 missing
        store
            .upsert_translation(
                "key1",
                "fr",
                TranslationUpdate::from_value_state(Some("Bonjour".into()), None),
            )
            .await
            .unwrap();

        // German: 2 translated
        store
            .upsert_translation(
                "key1",
                "de",
                TranslationUpdate::from_value_state(Some("Hallo".into()), None),
            )
            .await
            .unwrap();

        store
            .upsert_translation(
                "key2",
                "de",
                TranslationUpdate::from_value_state(Some("Welt".into()), None),
            )
            .await
            .unwrap();

        // Spanish: 0 translated (both missing)

        let percentages = store.get_translation_percentages().await;

        // English: 100% (2/2)
        let en_percentage = percentages.get("en").unwrap();
        assert_eq!(*en_percentage, 100.0);

        // French: 50% (1/2)
        let fr_percentage = percentages.get("fr").unwrap();
        assert_eq!(*fr_percentage, 50.0);

        // German: 100% (2/2)
        let de_percentage = percentages.get("de").unwrap();
        assert_eq!(*de_percentage, 100.0);
    }

    #[tokio::test]
    async fn get_translation_percentages_excludes_should_not_translate() {
        let tmp = TempStorePath::new("percentages_should_translate");
        let store = XcStringsStore::load_or_create(&tmp.file).await.unwrap();

        // Add 3 keys
        store
            .upsert_translation(
                "key1",
                "en",
                TranslationUpdate::from_value_state(Some("Hello".into()), None),
            )
            .await
            .unwrap();

        store
            .upsert_translation(
                "key2",
                "en",
                TranslationUpdate::from_value_state(Some("World".into()), None),
            )
            .await
            .unwrap();

        store
            .upsert_translation(
                "key3",
                "en",
                TranslationUpdate::from_value_state(Some("NoTranslate".into()), None),
            )
            .await
            .unwrap();

        // Mark key3 as should_translate=false
        store
            .set_should_translate("key3", Some(false))
            .await
            .unwrap();

        // French: only translate key1
        store
            .upsert_translation(
                "key1",
                "fr",
                TranslationUpdate::from_value_state(Some("Bonjour".into()), None),
            )
            .await
            .unwrap();

        // key2 is missing French, key3 should not be counted

        let percentages = store.get_translation_percentages().await;

        // English: 100% (2/2 translatable keys)
        let en_percentage = percentages.get("en").unwrap();
        assert_eq!(*en_percentage, 100.0);

        // French: 50% (1/2 translatable keys)
        // key3 is excluded from the calculation
        let fr_percentage = percentages.get("fr").unwrap();
        assert_eq!(*fr_percentage, 50.0);
    }
}
