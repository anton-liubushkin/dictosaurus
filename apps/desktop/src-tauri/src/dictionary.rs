//! Versioned custom dictionary storage and matching.

use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use unicode_casefold::{Locale, UnicodeCaseFold, Variant};

pub const DICTIONARY_VERSION: u32 = 1;
pub const MAX_DICTIONARY_BYTES: usize = 1024 * 1024;
pub const MAX_DICTIONARY_ENTRIES: usize = 500;
pub const MAX_ALIASES_PER_ENTRY: usize = 32;

static TEMP_FILE_COUNTER: AtomicU64 = AtomicU64::new(0);
static WORD_CHARACTER: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?u)^\w$").expect("word-character regex is valid"));

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DictionaryDocument {
    pub version: u32,
    pub enabled: bool,
    pub entries: Vec<DictionaryEntry>,
}

impl Default for DictionaryDocument {
    fn default() -> Self {
        Self {
            version: DICTIONARY_VERSION,
            enabled: true,
            entries: Vec::new(),
        }
    }
}

impl DictionaryDocument {
    pub fn normalized(&self) -> Result<Self, DictionaryError> {
        if self.version != DICTIONARY_VERSION {
            return Err(DictionaryError::UnsupportedVersion {
                version: self.version,
            });
        }
        if self.entries.len() > MAX_DICTIONARY_ENTRIES {
            return Err(DictionaryError::TooManyEntries {
                count: self.entries.len(),
                max: MAX_DICTIONARY_ENTRIES,
            });
        }
        validate_serialized_size(self)?;

        let mut entries = Vec::with_capacity(self.entries.len());
        let mut ids = HashSet::with_capacity(self.entries.len());
        let mut owners: HashMap<String, String> = HashMap::new();

        for (entry_index, entry) in self.entries.iter().enumerate() {
            if entry.aliases.len() > MAX_ALIASES_PER_ENTRY {
                return Err(DictionaryError::TooManyAliases {
                    entry_index,
                    count: entry.aliases.len(),
                    max: MAX_ALIASES_PER_ENTRY,
                });
            }
            validate_no_null(&entry.id, format!("entries[{entry_index}].id"))?;
            let id = normalize_whitespace(&entry.id);
            if id.is_empty() {
                return Err(DictionaryError::EmptyId { entry_index });
            }
            if !ids.insert(id.clone()) {
                return Err(DictionaryError::DuplicateId { id });
            }

            validate_no_null(&entry.term, format!("entries[{entry_index}].term"))?;
            let canonical_term = normalize_whitespace(&entry.term);
            if canonical_term.is_empty() {
                return Err(DictionaryError::EmptyCanonicalTerm { entry_id: id });
            }

            let canonical_key = comparison_key(&canonical_term);
            register_owner(&mut owners, &canonical_key, &canonical_term)?;

            let mut aliases = Vec::with_capacity(entry.aliases.len());
            let mut seen = HashSet::from([canonical_key]);
            for (alias_index, alias) in entry.aliases.iter().enumerate() {
                validate_no_null(
                    alias,
                    format!("entries[{entry_index}].aliases[{alias_index}]"),
                )?;
                let alias = normalize_whitespace(alias);
                if alias.is_empty() {
                    return Err(DictionaryError::EmptyAlias {
                        entry_id: id.clone(),
                        alias_index,
                    });
                }

                let alias_key = comparison_key(&alias);
                register_owner(&mut owners, &alias_key, &canonical_term)?;
                if seen.insert(alias_key) {
                    aliases.push(alias);
                }
            }

            entries.push(DictionaryEntry {
                id,
                term: canonical_term,
                aliases,
                enabled: entry.enabled,
            });
        }

        let normalized = Self {
            version: DICTIONARY_VERSION,
            enabled: self.enabled,
            entries,
        };
        validate_serialized_size(&normalized)?;
        Ok(normalized)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DictionaryEntry {
    pub id: String,
    pub term: String,
    pub aliases: Vec<String>,
    pub enabled: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DictionaryError {
    AliasConflict {
        alias: String,
        first_canonical_term: String,
        second_canonical_term: String,
    },
    CorruptJson {
        message: String,
    },
    DocumentTooLarge {
        size: usize,
        max: usize,
    },
    DuplicateId {
        id: String,
    },
    EmptyAlias {
        entry_id: String,
        alias_index: usize,
    },
    EmptyCanonicalTerm {
        entry_id: String,
    },
    EmptyId {
        entry_index: usize,
    },
    Storage {
        operation: &'static str,
        message: String,
    },
    TooManyAliases {
        entry_index: usize,
        count: usize,
        max: usize,
    },
    TooManyEntries {
        count: usize,
        max: usize,
    },
    Unavailable {
        message: String,
    },
    NullByte {
        field: String,
    },
    Regex {
        message: String,
    },
    Serialization {
        message: String,
    },
    UnsupportedVersion {
        version: u32,
    },
}

impl fmt::Display for DictionaryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AliasConflict {
                alias,
                first_canonical_term,
                second_canonical_term,
            } => write!(
                formatter,
                "alias {alias:?} belongs to both {first_canonical_term:?} and {second_canonical_term:?}"
            ),
            Self::CorruptJson { message } => write!(formatter, "invalid dictionary JSON: {message}"),
            Self::DocumentTooLarge { size, max } => {
                write!(formatter, "dictionary is {size} bytes; maximum is {max}")
            }
            Self::DuplicateId { id } => write!(formatter, "duplicate dictionary entry id {id:?}"),
            Self::EmptyAlias {
                entry_id,
                alias_index,
            } => write!(formatter, "entry {entry_id:?} has an empty alias at {alias_index}"),
            Self::EmptyCanonicalTerm { entry_id } => {
                write!(formatter, "entry {entry_id:?} has an empty canonical term")
            }
            Self::EmptyId { entry_index } => {
                write!(formatter, "entry at index {entry_index} has an empty id")
            }
            Self::Storage { operation, message } => write!(formatter, "{operation}: {message}"),
            Self::TooManyAliases {
                entry_index,
                count,
                max,
            } => write!(
                formatter,
                "entry at index {entry_index} has {count} aliases; maximum is {max}"
            ),
            Self::TooManyEntries { count, max } => {
                write!(formatter, "dictionary has {count} entries; maximum is {max}")
            }
            Self::Unavailable { message } => write!(formatter, "dictionary unavailable: {message}"),
            Self::NullByte { field } => write!(formatter, "{field} contains a null byte"),
            Self::Regex { message } => write!(formatter, "failed to compile dictionary: {message}"),
            Self::Serialization { message } => {
                write!(formatter, "serialize dictionary: {message}")
            }
            Self::UnsupportedVersion { version } => {
                write!(formatter, "unsupported dictionary version {version}")
            }
        }
    }
}

impl std::error::Error for DictionaryError {}

#[derive(Debug)]
struct CompiledAlias {
    regex: Regex,
    canonical_term: String,
}

#[derive(Debug, Default)]
pub struct CompiledDictionary {
    aliases: Vec<CompiledAlias>,
    hints: Vec<String>,
}

impl CompiledDictionary {
    pub fn compile(document: &DictionaryDocument) -> Result<Self, DictionaryError> {
        let normalized = document.normalized()?;
        Self::compile_normalized(&normalized)
    }

    fn compile_normalized(document: &DictionaryDocument) -> Result<Self, DictionaryError> {
        if !document.enabled {
            return Ok(Self::default());
        }

        let mut aliases = Vec::new();
        let mut hints = Vec::new();
        let mut seen_matchers = HashSet::new();
        let mut seen_hints = HashSet::new();

        for entry in document.entries.iter().filter(|entry| entry.enabled) {
            for phrase in std::iter::once(&entry.term).chain(&entry.aliases) {
                let key = comparison_key(phrase);
                if seen_hints.insert(key.clone()) {
                    hints.push(phrase.clone());
                }
                if !seen_matchers.insert(key) {
                    continue;
                }

                let regex = compile_phrase(phrase)?;
                aliases.push(CompiledAlias {
                    regex,
                    canonical_term: entry.term.clone(),
                });
            }
        }

        Ok(Self { aliases, hints })
    }

    pub fn apply(&self, text: &str) -> String {
        #[derive(Clone, Copy)]
        struct Candidate<'a> {
            start: usize,
            end: usize,
            canonical_term: &'a str,
        }

        let mut candidates = Vec::new();
        for alias in &self.aliases {
            candidates.extend(
                alias
                    .regex
                    .find_iter(text)
                    .filter(|matched| {
                        has_unicode_word_boundaries(text, matched.start(), matched.end())
                    })
                    .map(|matched| Candidate {
                        start: matched.start(),
                        end: matched.end(),
                        canonical_term: &alias.canonical_term,
                    }),
            );
        }

        if candidates.is_empty() {
            return text.to_owned();
        }

        candidates.sort_by(|left, right| {
            (right.end - right.start)
                .cmp(&(left.end - left.start))
                .then_with(|| left.start.cmp(&right.start))
        });

        let mut selected = BTreeMap::new();
        for candidate in candidates {
            let overlaps = selected.range(..candidate.end).next_back().is_some_and(
                |(_, existing): (&usize, &Candidate<'_>)| existing.end > candidate.start,
            );
            if !overlaps {
                selected.insert(candidate.start, candidate);
            }
        }

        let mut output = String::with_capacity(text.len());
        let mut copied_until = 0;
        for candidate in selected.into_values() {
            output.push_str(&text[copied_until..candidate.start]);
            output.push_str(candidate.canonical_term);
            copied_until = candidate.end;
        }
        output.push_str(&text[copied_until..]);
        output
    }

    pub fn build_hints(&self, max_total_bytes: usize) -> String {
        let mut result = String::new();
        for hint in &self.hints {
            let separator_len = usize::from(!result.is_empty()) * 2;
            if result.len() + separator_len + hint.len() > max_total_bytes {
                break;
            }
            if !result.is_empty() {
                result.push_str(", ");
            }
            result.push_str(hint);
        }
        result
    }
}

#[derive(Clone, Debug)]
pub struct DictionarySnapshot {
    compiled: Arc<CompiledDictionary>,
}

impl DictionarySnapshot {
    pub fn compile(document: &DictionaryDocument) -> Result<Self, DictionaryError> {
        Ok(Self {
            compiled: Arc::new(CompiledDictionary::compile(document)?),
        })
    }

    fn from_normalized(document: &DictionaryDocument) -> Result<Self, DictionaryError> {
        Ok(Self {
            compiled: Arc::new(CompiledDictionary::compile_normalized(document)?),
        })
    }

    fn disabled() -> Self {
        Self {
            compiled: Arc::new(CompiledDictionary::default()),
        }
    }

    pub fn apply(&self, text: &str) -> String {
        self.compiled.apply(text)
    }

    pub fn build_hints(&self, max_total_bytes: usize) -> String {
        self.compiled.build_hints(max_total_bytes)
    }
}

pub struct DictionaryStore {
    path: Option<PathBuf>,
    current: DictionaryDocument,
    snapshot: DictionarySnapshot,
    load_error: Option<DictionaryError>,
}

impl DictionaryStore {
    pub fn from_app_data_dir(app_data_dir: PathBuf) -> Self {
        Self::from_path(app_data_dir.join("dictionary.json"))
    }

    pub fn unavailable(message: impl Into<String>) -> Self {
        Self::failed(
            None,
            DictionaryError::Unavailable {
                message: message.into(),
            },
        )
    }

    pub fn from_path(path: PathBuf) -> Self {
        match load_document(&path) {
            Ok(document) => match DictionarySnapshot::from_normalized(&document) {
                Ok(snapshot) => Self {
                    path: Some(path),
                    current: document,
                    snapshot,
                    load_error: None,
                },
                Err(error) => Self::failed(Some(path), error),
            },
            Err(error) => Self::failed(Some(path), error),
        }
    }

    fn failed(path: Option<PathBuf>, error: DictionaryError) -> Self {
        Self {
            path,
            current: DictionaryDocument::default(),
            snapshot: DictionarySnapshot::disabled(),
            load_error: Some(error),
        }
    }

    pub fn current(&self) -> &DictionaryDocument {
        &self.current
    }

    pub fn load_error(&self) -> Option<&DictionaryError> {
        self.load_error.as_ref()
    }

    pub fn snapshot(&self) -> DictionarySnapshot {
        self.snapshot.clone()
    }

    pub fn reload(&mut self) -> Result<DictionaryDocument, DictionaryError> {
        let path = match self.path.clone() {
            Some(path) => path,
            None => {
                let error =
                    self.load_error
                        .clone()
                        .unwrap_or_else(|| DictionaryError::Unavailable {
                            message: "dictionary path is unavailable".into(),
                        });
                self.snapshot = DictionarySnapshot::disabled();
                self.load_error = Some(error.clone());
                return Err(error);
            }
        };

        let reloaded = load_document(&path).and_then(|document| {
            DictionarySnapshot::from_normalized(&document).map(|snapshot| (document, snapshot))
        });

        match reloaded {
            Ok((document, snapshot)) => {
                self.current = document;
                self.snapshot = snapshot;
                self.load_error = None;
                Ok(self.current.clone())
            }
            Err(error) => {
                self.snapshot = DictionarySnapshot::disabled();
                self.load_error = Some(error.clone());
                Err(error)
            }
        }
    }

    pub fn update(
        &mut self,
        document: DictionaryDocument,
    ) -> Result<DictionaryDocument, DictionaryError> {
        let path = self.path.as_deref().ok_or_else(|| {
            self.load_error
                .clone()
                .unwrap_or_else(|| DictionaryError::Unavailable {
                    message: "dictionary path is unavailable".into(),
                })
        })?;
        let normalized = document.normalized()?;
        let snapshot = DictionarySnapshot::from_normalized(&normalized)?;
        let serialized = serde_json::to_vec_pretty(&normalized).map_err(|error| {
            DictionaryError::Serialization {
                message: error.to_string(),
            }
        })?;
        ensure_size(serialized.len())?;
        atomic_write(path, &serialized)?;

        self.current = normalized;
        self.snapshot = snapshot;
        self.load_error = None;
        Ok(self.current.clone())
    }

    pub fn reset(&mut self) -> Result<(), DictionaryError> {
        self.update(DictionaryDocument::default()).map(|_| ())
    }
}

fn load_document(path: &Path) -> Result<DictionaryDocument, DictionaryError> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(DictionaryDocument::default());
        }
        Err(error) => return Err(storage_error("open dictionary", error)),
    };
    let metadata = file
        .metadata()
        .map_err(|error| storage_error("read dictionary metadata", error))?;
    ensure_size(metadata.len().try_into().unwrap_or(usize::MAX))?;
    let mut raw = Vec::with_capacity(metadata.len().try_into().unwrap_or(0));
    file.take((MAX_DICTIONARY_BYTES + 1) as u64)
        .read_to_end(&mut raw)
        .map_err(|error| storage_error("read dictionary", error))?;
    ensure_size(raw.len())?;
    let document: DictionaryDocument =
        serde_json::from_slice(&raw).map_err(|error| DictionaryError::CorruptJson {
            message: error.to_string(),
        })?;
    document.normalized()
}

fn atomic_write(path: &Path, contents: &[u8]) -> Result<(), DictionaryError> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)
            .map_err(|error| storage_error("create dictionary directory", error))?;
    }

    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("dictionary.json");
    let sequence = TEMP_FILE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let temp_path = parent.join(format!(
        ".{file_name}.{}.{sequence}.tmp",
        std::process::id()
    ));

    let write_result = (|| {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temp_path)
            .map_err(|error| storage_error("create temporary dictionary", error))?;
        file.write_all(contents)
            .map_err(|error| storage_error("write temporary dictionary", error))?;
        file.sync_all()
            .map_err(|error| storage_error("sync temporary dictionary", error))?;
        drop(file);
        fs::rename(&temp_path, path).map_err(|error| storage_error("replace dictionary", error))?;
        sync_parent_directory_best_effort(parent);
        Ok(())
    })();

    if write_result.is_err() {
        let _ = fs::remove_file(&temp_path);
    }
    write_result
}

#[cfg(unix)]
fn sync_parent_directory_best_effort(parent: &Path) {
    if let Err(error) = File::open(parent).and_then(|directory| directory.sync_all()) {
        log::warn!("[dictionary] failed to sync dictionary directory: {error}");
    }
}

#[cfg(not(unix))]
fn sync_parent_directory_best_effort(_parent: &Path) {}

fn compile_phrase(phrase: &str) -> Result<Regex, DictionaryError> {
    let escaped = phrase
        .split(' ')
        .map(regex::escape)
        .collect::<Vec<_>>()
        .join(r"\s+");
    Regex::new(&format!(r"(?iu:{escaped})")).map_err(|error| DictionaryError::Regex {
        message: error.to_string(),
    })
}

fn has_unicode_word_boundaries(text: &str, start: usize, end: usize) -> bool {
    let left_is_boundary = text[..start]
        .chars()
        .next_back()
        .is_none_or(|character| !is_word_character(character));
    let right_is_boundary = text[end..]
        .chars()
        .next()
        .is_none_or(|character| !is_word_character(character));
    left_is_boundary && right_is_boundary
}

fn is_word_character(character: char) -> bool {
    let mut encoded = [0; 4];
    WORD_CHARACTER.is_match(character.encode_utf8(&mut encoded))
}

fn normalize_whitespace(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn comparison_key(value: &str) -> String {
    value
        .case_fold_with(Variant::Simple, Locale::NonTurkic)
        .collect()
}

fn register_owner(
    owners: &mut HashMap<String, String>,
    alias_key: &str,
    canonical_term: &str,
) -> Result<(), DictionaryError> {
    if let Some(existing) = owners.get(alias_key) {
        if existing != canonical_term {
            return Err(DictionaryError::AliasConflict {
                alias: alias_key.to_owned(),
                first_canonical_term: existing.clone(),
                second_canonical_term: canonical_term.to_owned(),
            });
        }
    } else {
        owners.insert(alias_key.to_owned(), canonical_term.to_owned());
    }
    Ok(())
}

fn validate_no_null(value: &str, field: String) -> Result<(), DictionaryError> {
    if value.contains('\0') {
        Err(DictionaryError::NullByte { field })
    } else {
        Ok(())
    }
}

fn validate_serialized_size(document: &DictionaryDocument) -> Result<(), DictionaryError> {
    let size = serde_json::to_vec(document)
        .map_err(|error| DictionaryError::Serialization {
            message: error.to_string(),
        })?
        .len();
    ensure_size(size)
}

fn ensure_size(size: usize) -> Result<(), DictionaryError> {
    if size > MAX_DICTIONARY_BYTES {
        Err(DictionaryError::DocumentTooLarge {
            size,
            max: MAX_DICTIONARY_BYTES,
        })
    } else {
        Ok(())
    }
}

fn storage_error(operation: &'static str, error: std::io::Error) -> DictionaryError {
    DictionaryError::Storage {
        operation,
        message: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

    struct TestDir(PathBuf);

    impl TestDir {
        fn new() -> Self {
            let sequence = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "dictosaurus-dictionary-{}-{sequence}",
                std::process::id()
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn entry(id: &str, canonical_term: &str, aliases: &[&str]) -> DictionaryEntry {
        DictionaryEntry {
            id: id.into(),
            term: canonical_term.into(),
            aliases: aliases.iter().map(|alias| (*alias).into()).collect(),
            enabled: true,
        }
    }

    fn document(entries: Vec<DictionaryEntry>) -> DictionaryDocument {
        DictionaryDocument {
            version: DICTIONARY_VERSION,
            enabled: true,
            entries,
        }
    }

    fn snapshot(entries: Vec<DictionaryEntry>) -> DictionarySnapshot {
        DictionarySnapshot::compile(&document(entries)).unwrap()
    }

    #[test]
    fn replaces_cyrillic_case_insensitively() {
        let snapshot = snapshot(vec![entry("1", "Ёлка", &["елочка"])]);

        assert_eq!(snapshot.apply("ЕЛОЧКА и ёлка"), "Ёлка и Ёлка");
    }

    #[test]
    fn replaces_latin_mixed_case() {
        let snapshot = snapshot(vec![entry("1", "Rust", &[])]);

        assert_eq!(snapshot.apply("rUsT"), "Rust");
    }

    #[test]
    fn respects_punctuation_boundaries() {
        let snapshot = snapshot(vec![entry("1", "Rust", &[])]);

        assert_eq!(snapshot.apply("(rust), rust!"), "(Rust), Rust!");
    }

    #[test]
    fn punctuation_ending_alias_respects_unicode_word_boundaries() {
        let snapshot = snapshot(vec![entry("1", "C plus plus", &["C++"])]);

        assert_eq!(
            snapshot.apply("C++ C++x C++я xC++ яC++ (c++)"),
            "C plus plus C++x C++я xC++ яC++ (C plus plus)"
        );
    }

    #[test]
    fn accepts_flexible_whitespace_in_multiword_aliases() {
        let snapshot = snapshot(vec![entry("1", "New York", &["new york"])]);

        assert_eq!(snapshot.apply("new\t \n york"), "New York");
    }

    #[test]
    fn longest_overlapping_match_wins() {
        let snapshot = snapshot(vec![
            entry("1", "Big Apple", &["new york"]),
            entry("2", "New", &["new"]),
        ]);

        assert_eq!(snapshot.apply("new york"), "Big Apple");
    }

    #[test]
    fn longer_match_wins_when_overlap_starts_later() {
        let snapshot = snapshot(vec![
            entry("1", "First", &["new york"]),
            entry("2", "Second", &["york city center"]),
        ]);

        assert_eq!(snapshot.apply("new york city center"), "new Second");
    }

    #[test]
    fn does_not_replace_inside_another_word() {
        let snapshot = snapshot(vec![entry("1", "Cat", &["cat"])]);

        assert_eq!(snapshot.apply("concatenate cat"), "concatenate Cat");
    }

    #[test]
    fn replacements_do_not_cascade() {
        let snapshot = snapshot(vec![
            entry("1", "Beta value", &["alpha"]),
            entry("2", "Gamma", &["beta"]),
        ]);

        assert_eq!(snapshot.apply("alpha"), "Beta value");
    }

    #[test]
    fn disabled_dictionary_and_entries_are_ignored() {
        let disabled_entry = DictionaryEntry {
            enabled: false,
            ..entry("1", "Rust", &["rust"])
        };
        let entry_disabled = snapshot(vec![disabled_entry]);
        let globally_disabled = DictionarySnapshot::compile(&DictionaryDocument {
            enabled: false,
            ..document(vec![entry("2", "Python", &["python"])])
        })
        .unwrap();

        assert_eq!(entry_disabled.apply("rust"), "rust");
        assert_eq!(globally_disabled.apply("python"), "python");
    }

    #[test]
    fn rejects_alias_conflicts_between_canonical_terms() {
        let result = DictionarySnapshot::compile(&document(vec![
            entry("1", "PostgreSQL", &["postgres"]),
            entry("2", "Postgres", &["POSTGRES"]),
        ]));

        assert!(matches!(result, Err(DictionaryError::AliasConflict { .. })));
    }

    #[test]
    fn rejects_alias_conflicts_under_unicode_simple_case_folding() {
        let result = DictionarySnapshot::compile(&document(vec![
            entry("1", "First", &["S"]),
            entry("2", "Second", &["ſ"]),
        ]));

        assert!(matches!(result, Err(DictionaryError::AliasConflict { .. })));
    }

    #[test]
    fn allows_duplicate_alias_for_the_same_canonical_term() {
        let snapshot = snapshot(vec![
            entry("1", "Rust", &["rustlang"]),
            entry("2", "Rust", &["RUSTLANG"]),
        ]);

        assert_eq!(snapshot.apply("rustlang"), "Rust");
        assert_eq!(snapshot.build_hints(100), "Rust, rustlang");
    }

    #[test]
    fn normalizes_values_and_deduplicates_aliases() {
        let normalized = document(vec![entry(
            "  database  ",
            "  New   York ",
            &[" nyc ", "NYC", "New\tYork"],
        )])
        .normalized()
        .unwrap();

        assert_eq!(normalized.entries[0].id, "database");
        assert_eq!(normalized.entries[0].term, "New York");
        assert_eq!(normalized.entries[0].aliases, vec!["nyc"]);
    }

    #[test]
    fn rejects_empty_terms_and_aliases() {
        let empty_term = document(vec![entry("1", " \t ", &[])]);
        let empty_alias = document(vec![entry("1", "Rust", &[" \n "])]);

        assert!(matches!(
            empty_term.normalized(),
            Err(DictionaryError::EmptyCanonicalTerm { .. })
        ));
        assert!(matches!(
            empty_alias.normalized(),
            Err(DictionaryError::EmptyAlias { .. })
        ));
    }

    #[test]
    fn rejects_empty_id() {
        let result = document(vec![entry(" \t ", "Rust", &[])]).normalized();

        assert!(matches!(result, Err(DictionaryError::EmptyId { .. })));
    }

    #[test]
    fn rejects_duplicate_ids_after_whitespace_normalization() {
        let result =
            document(vec![entry("a", "Rust", &[]), entry(" a ", "Python", &[])]).normalized();

        assert!(matches!(
            result,
            Err(DictionaryError::DuplicateId { id }) if id == "a"
        ));
    }

    #[test]
    fn rejects_null_byte_in_id() {
        let result = document(vec![entry("ru\0st", "Rust", &[])]).normalized();

        assert!(matches!(
            result,
            Err(DictionaryError::NullByte { field }) if field == "entries[0].id"
        ));
    }

    #[test]
    fn rejects_null_byte_in_term() {
        let result = document(vec![entry("1", "Ru\0st", &[])]).normalized();

        assert!(matches!(
            result,
            Err(DictionaryError::NullByte { field }) if field == "entries[0].term"
        ));
    }

    #[test]
    fn rejects_null_byte_in_alias() {
        let result = document(vec![entry("1", "Rust", &["ru\0st"])]).normalized();

        assert!(matches!(
            result,
            Err(DictionaryError::NullByte { field }) if field == "entries[0].aliases[0]"
        ));
    }

    #[test]
    fn rejects_excessively_large_documents() {
        let oversized = "a".repeat(MAX_DICTIONARY_BYTES);
        let result = document(vec![entry("1", &oversized, &[])]).normalized();

        assert!(matches!(
            result,
            Err(DictionaryError::DocumentTooLarge { .. })
        ));
    }

    #[test]
    fn rejects_too_many_entries() {
        let entries = (0..=MAX_DICTIONARY_ENTRIES)
            .map(|index| entry(&index.to_string(), &format!("Term {index}"), &[]))
            .collect();

        let result = document(entries).normalized();

        assert!(matches!(
            result,
            Err(DictionaryError::TooManyEntries {
                count: 501,
                max: 500
            })
        ));
    }

    #[test]
    fn rejects_too_many_aliases_per_entry() {
        let aliases = (0..=MAX_ALIASES_PER_ENTRY)
            .map(|index| format!("alias {index}"))
            .collect();
        let document = document(vec![DictionaryEntry {
            id: "1".into(),
            term: "Rust".into(),
            aliases,
            enabled: true,
        }]);

        let result = document.normalized();

        assert!(matches!(
            result,
            Err(DictionaryError::TooManyAliases {
                count: 33,
                max: 32,
                ..
            })
        ));
    }

    #[test]
    fn json_round_trip_uses_public_term_field() {
        let document = document(vec![entry("1", "New York", &["NYC"])]);

        let json = serde_json::to_string(&document).unwrap();
        let decoded: DictionaryDocument = serde_json::from_str(&json).unwrap();

        assert!(json.contains("\"term\""));
        assert!(!json.contains("\"canonicalTerm\""));
        assert_eq!(decoded, document);
    }

    #[test]
    fn serialization_errors_are_distinct_from_corrupt_input() {
        let error = DictionaryError::Serialization {
            message: "failed".into(),
        };

        assert_eq!(error.to_string(), "serialize dictionary: failed");
        assert!(!matches!(error, DictionaryError::CorruptJson { .. }));
    }

    #[test]
    fn corrupt_json_disables_matcher_without_overwriting_file() {
        let dir = TestDir::new();
        let path = dir.path().join("dictionary.json");
        fs::write(&path, "{not-json").unwrap();

        let store = DictionaryStore::from_path(path.clone());

        assert!(matches!(
            store.load_error(),
            Some(DictionaryError::CorruptJson { .. })
        ));
        assert_eq!(store.snapshot().apply("rust"), "rust");
        assert_eq!(fs::read_to_string(path).unwrap(), "{not-json");
    }

    #[test]
    fn reload_recovers_after_external_file_repair() {
        let dir = TestDir::new();
        let path = dir.path().join("dictionary.json");
        fs::write(&path, "{not-json").unwrap();
        let mut store = DictionaryStore::from_path(path.clone());
        let repaired = document(vec![entry("rust", "Rust", &["rustlang"])]);
        fs::write(&path, serde_json::to_vec(&repaired).unwrap()).unwrap();

        let reloaded = store.reload().unwrap();

        assert_eq!(reloaded, repaired);
        assert!(store.load_error().is_none());
        assert_eq!(store.snapshot().apply("rustlang"), "Rust");
    }

    #[test]
    fn reload_failure_is_fail_closed_and_preserves_external_source() {
        let dir = TestDir::new();
        let path = dir.path().join("dictionary.json");
        let initial = document(vec![entry("rust", "Rust", &["rustlang"])]);
        fs::write(&path, serde_json::to_vec(&initial).unwrap()).unwrap();
        let mut store = DictionaryStore::from_path(path.clone());
        fs::write(&path, "{still-not-json").unwrap();

        let error = store.reload().unwrap_err();

        assert!(matches!(error, DictionaryError::CorruptJson { .. }));
        assert!(matches!(
            store.load_error(),
            Some(DictionaryError::CorruptJson { .. })
        ));
        assert_eq!(store.snapshot().apply("rustlang"), "rustlang");
        assert_eq!(fs::read_to_string(path).unwrap(), "{still-not-json");
    }

    #[test]
    fn directory_path_is_a_fail_closed_storage_error() {
        let dir = TestDir::new();

        let store = DictionaryStore::from_path(dir.path().to_path_buf());

        assert!(matches!(
            store.load_error(),
            Some(DictionaryError::Storage { .. })
        ));
        assert_eq!(store.snapshot().apply("rust"), "rust");
    }

    #[test]
    fn unsupported_version_disables_matcher_without_overwriting_file() {
        let dir = TestDir::new();
        let path = dir.path().join("dictionary.json");
        let raw = r#"{"version":2,"enabled":true,"entries":[]}"#;
        fs::write(&path, raw).unwrap();

        let store = DictionaryStore::from_path(path.clone());

        assert!(matches!(
            store.load_error(),
            Some(DictionaryError::UnsupportedVersion { version: 2 })
        ));
        assert_eq!(store.snapshot().apply("rust"), "rust");
        assert_eq!(fs::read_to_string(path).unwrap(), raw);
    }

    #[test]
    fn unavailable_store_reports_path_error_and_uses_disabled_snapshot() {
        let store = DictionaryStore::unavailable("app data directory unavailable");

        assert!(matches!(
            store.load_error(),
            Some(DictionaryError::Unavailable { message })
                if message == "app data directory unavailable"
        ));
        assert_eq!(store.current(), &DictionaryDocument::default());
        assert_eq!(store.snapshot().apply("rust"), "rust");
    }

    #[test]
    fn unavailable_store_rejects_update_and_reset_without_changing_state() {
        let mut store = DictionaryStore::unavailable("app data directory unavailable");
        let path_error = store.load_error().cloned().unwrap();

        assert_eq!(
            store.update(document(vec![entry("1", "Rust", &["rustlang"])])),
            Err(path_error.clone())
        );
        assert_eq!(store.reset(), Err(path_error.clone()));
        assert_eq!(store.load_error(), Some(&path_error));
        assert_eq!(store.current(), &DictionaryDocument::default());
        assert_eq!(store.snapshot().apply("rustlang"), "rustlang");
    }

    #[test]
    fn unavailable_store_rejects_reload_with_its_typed_path_error() {
        let mut store = DictionaryStore::unavailable("app data directory unavailable");
        let path_error = store.load_error().cloned().unwrap();

        assert_eq!(store.reload(), Err(path_error.clone()));
        assert_eq!(store.load_error(), Some(&path_error));
        assert_eq!(store.snapshot().apply("rust"), "rust");
    }

    #[test]
    fn invalid_update_preserves_load_error_and_source_file() {
        let dir = TestDir::new();
        let sources = [
            ("corrupt.json", "{not-json"),
            (
                "unsupported.json",
                r#"{"version":2,"enabled":true,"entries":[]}"#,
            ),
        ];

        for (file_name, raw) in sources {
            let path = dir.path().join(file_name);
            fs::write(&path, raw).unwrap();
            let mut store = DictionaryStore::from_path(path.clone());
            let initial_error = store.load_error().cloned().unwrap();

            let result = store.update(document(vec![entry("1", " ", &[])]));

            assert!(matches!(
                result,
                Err(DictionaryError::EmptyCanonicalTerm { .. })
            ));
            assert_eq!(store.load_error(), Some(&initial_error));
            assert_eq!(store.snapshot().apply("rust"), "rust");
            assert_eq!(fs::read_to_string(path).unwrap(), raw);
        }
    }

    #[test]
    fn reset_clears_load_error_and_atomically_writes_default_document() {
        let dir = TestDir::new();
        let path = dir.path().join("dictionary.json");
        fs::write(&path, "{not-json").unwrap();
        let mut store = DictionaryStore::from_path(path.clone());
        assert!(store.load_error().is_some());

        store.reset().unwrap();

        assert!(store.load_error().is_none());
        assert_eq!(store.current(), &DictionaryDocument::default());
        let persisted: DictionaryDocument =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(persisted, DictionaryDocument::default());
        let leftovers: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|item| item.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty());
    }

    #[test]
    fn valid_update_clears_load_error() {
        let dir = TestDir::new();
        let path = dir.path().join("dictionary.json");
        fs::write(&path, "{not-json").unwrap();
        let mut store = DictionaryStore::from_path(path);
        assert!(store.load_error().is_some());

        store
            .update(document(vec![entry("1", "Rust", &["rustlang"])]))
            .unwrap();

        assert!(store.load_error().is_none());
        assert_eq!(store.snapshot().apply("rustlang"), "Rust");
    }

    #[test]
    fn update_is_atomic_and_leaves_no_temp_file() {
        let dir = TestDir::new();
        let path = dir.path().join("dictionary.json");
        let mut store = DictionaryStore::from_path(path.clone());

        store
            .update(document(vec![entry("1", "Rust", &[])]))
            .unwrap();

        assert!(path.exists());
        let leftovers: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|item| item.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn parent_directory_sync_errors_are_best_effort_after_replace() {
        let dir = TestDir::new();
        let missing_directory = dir.path().join("missing");

        sync_parent_directory_best_effort(&missing_directory);
    }

    #[test]
    fn hints_follow_document_order_and_skip_disabled_values() {
        let disabled = DictionaryEntry {
            enabled: false,
            ..entry("2", "Hidden", &["secret"])
        };
        let snapshot = snapshot(vec![
            entry("1", "Alpha", &["A"]),
            disabled,
            entry("3", "Beta", &["B"]),
        ]);

        assert_eq!(snapshot.build_hints(100), "Alpha, A, Beta, B");
    }

    #[test]
    fn hints_respect_total_length_limit() {
        let snapshot = snapshot(vec![entry("1", "Alpha", &["A", "Beta"])]);

        let hints = snapshot.build_hints(8);

        assert_eq!(hints, "Alpha, A");
        assert!(hints.len() <= 8);
    }
}
