//! Offline, version-pinned NeoForge documentation retrieval.
//!
//! The Markdown corpus is vendored under docs/neoforge/1.21.1. Search uses
//! title, heading, path, and body fields, plus Java-identifier-aware tokenizing.
//! It requires no network access or embedding service.

use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::env;
use std::fs;
use std::path::{Component, Path, PathBuf};
use walkdir::WalkDir;

const STOP_WORDS: &[&str] = &[
    "a", "an", "and", "are", "as", "at", "be", "by", "for", "from", "in", "is",
    "it", "of", "on", "or", "that", "the", "their", "this", "to", "use", "using",
    "with", "when", "which", "your",
];

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct NeoForgeDocsManifest {
    pub format_version: u32,
    pub platform: String,
    pub minecraft_version: String,
    pub documentation_scope: String,
    pub source_repository: String,
    pub source_path: String,
    pub source_ref: String,
    pub source_commit: String,
    pub source_license: String,
    pub page_count: usize,
    pub category_count: usize,
    pub total_source_bytes: u64,
    pub pages: Vec<NeoForgeDocPageMeta>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct NeoForgeDocPageMeta {
    pub path: String,
    pub section: String,
    pub source_sha: String,
    pub bytes: u64,
    pub source_url: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DocSearchResult {
    pub title: String,
    pub section: String,
    pub path: String,
    pub source_url: String,
    pub score: f32,
    pub snippet: String,
}

struct IndexedPage {
    meta: NeoForgeDocPageMeta,
    title: String,
    body: String,
    title_terms: HashMap<String, usize>,
    heading_terms: HashMap<String, usize>,
    path_terms: HashMap<String, usize>,
    body_terms: HashMap<String, usize>,
}

pub struct NeoForgeDocsIndex {
    root: PathBuf,
    manifest: NeoForgeDocsManifest,
    pages: Vec<IndexedPage>,
}

impl NeoForgeDocsIndex {
    /// Load and validate the complete version-pinned corpus from disk.
    pub fn load(root: impl AsRef<Path>) -> Result<Self> {
        let root = root.as_ref().to_path_buf();
        let manifest_path = root.join("manifest.json");
        let manifest_text = fs::read_to_string(&manifest_path)
            .with_context(|| format!("reading NeoForge docs manifest {}", manifest_path.display()))?;
        let manifest: NeoForgeDocsManifest = serde_json::from_str(&manifest_text)
            .with_context(|| format!("parsing NeoForge docs manifest {}", manifest_path.display()))?;

        ensure!(manifest.format_version == 1, "unsupported NeoForge docs manifest version {}", manifest.format_version);
        ensure!(manifest.platform.eq_ignore_ascii_case("NeoForge"), "unexpected docs platform '{}'", manifest.platform);
        ensure!(manifest.minecraft_version == "1.21.1", "expected NeoForge docs for Minecraft 1.21.1, found {}", manifest.minecraft_version);
        ensure!(manifest.page_count == manifest.pages.len(), "manifest declares {} pages but lists {}", manifest.page_count, manifest.pages.len());

        let mut pages = Vec::with_capacity(manifest.pages.len());
        let mut seen = HashSet::new();
        for meta in &manifest.pages {
            ensure!(seen.insert(meta.path.clone()), "duplicate documentation path '{}'", meta.path);
            let path = safe_page_path(&root, &meta.path)?;
            let body = fs::read_to_string(&path)
                .with_context(|| format!("reading NeoForge documentation page {}", path.display()))?;
            let title = extract_title(&body, &meta.path);
            let headings = body.lines()
                .filter(|line| line.trim_start().starts_with('#'))
                .collect::<Vec<_>>()
                .join(" ");
            pages.push(IndexedPage {
                title_terms: term_counts(&title),
                heading_terms: term_counts(&headings),
                path_terms: term_counts(&meta.path.replace('/', " ")),
                body_terms: term_counts(&body),
                meta: meta.clone(),
                title,
                body,
            });
        }

        // Do not silently accept an incomplete vendor snapshot.
        let on_disk = WalkDir::new(&root)
            .into_iter()
            .filter_map(std::result::Result::ok)
            .filter(|entry| entry.file_type().is_file())
            .filter(|entry| entry.path().extension().and_then(|v| v.to_str()) == Some("md"))
            .count();
        ensure!(on_disk == manifest.page_count,
            "documentation corpus is incomplete: manifest has {} pages, disk contains {} markdown files",
            manifest.page_count, on_disk);

        Ok(Self { root, manifest, pages })
    }

    pub fn root(&self) -> &Path { &self.root }
    pub fn manifest(&self) -> &NeoForgeDocsManifest { &self.manifest }
    pub fn page_count(&self) -> usize { self.pages.len() }

    /// Search the corpus with deterministic identifier-aware lexical ranking.
    pub fn search(&self, query: &str, limit: usize) -> Vec<DocSearchResult> {
        let query_terms = query_terms(query);
        if query_terms.is_empty() || limit == 0 { return Vec::new(); }

        let page_total = self.pages.len().max(1) as f32;
        let query_lower = query.to_lowercase();
        let mut results = Vec::new();

        for page in &self.pages {
            let mut score = 0.0_f32;
            for term in &query_terms {
                let document_frequency = self.pages.iter()
                    .filter(|candidate| candidate.body_terms.contains_key(term)
                        || candidate.title_terms.contains_key(term)
                        || candidate.heading_terms.contains_key(term)
                        || candidate.path_terms.contains_key(term))
                    .count() as f32;
                let idf = (1.0 + (page_total + 0.5) / (document_frequency + 0.5)).ln();
                let title = *page.title_terms.get(term).unwrap_or(&0) as f32;
                let heading = *page.heading_terms.get(term).unwrap_or(&0) as f32;
                let path = *page.path_terms.get(term).unwrap_or(&0) as f32;
                let body = (*page.body_terms.get(term).unwrap_or(&0)).min(12) as f32;
                score += idf * (title * 8.0 + heading * 5.0 + path * 4.0 + body);
            }

            let phrase = query_lower.trim();
            if !phrase.is_empty() {
                if page.title.to_lowercase().contains(phrase) { score += 12.0; }
                else if page.meta.path.to_lowercase().contains(phrase) { score += 8.0; }
                else if page.body.to_lowercase().contains(phrase) { score += 2.0; }
            }

            if score > 0.0 {
                results.push(DocSearchResult {
                    title: page.title.clone(),
                    section: page.meta.section.clone(),
                    path: page.meta.path.clone(),
                    source_url: page.meta.source_url.clone(),
                    score,
                    snippet: make_snippet(&page.body, &query_terms),
                });
            }
        }

        results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(Ordering::Equal)
            .then_with(|| a.path.cmp(&b.path)));
        results.truncate(limit);
        results
    }
}

/// Override the bundled corpus location with MODPORT_NEOFORGE_DOCS.
pub fn default_docs_root() -> PathBuf {
    env::var_os("MODPORT_NEOFORGE_DOCS")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("docs/neoforge/1.21.1"))
}

fn safe_page_path(root: &Path, relative: &str) -> Result<PathBuf> {
    let candidate = Path::new(relative);
    ensure!(!candidate.is_absolute(), "absolute documentation path is not allowed: {relative}");
    for component in candidate.components() {
        ensure!(matches!(component, Component::Normal(_)), "unsafe documentation path in manifest: {relative}");
    }
    ensure!(candidate.extension().and_then(|v| v.to_str()) == Some("md"),
        "manifest page is not Markdown: {relative}");
    Ok(root.join(candidate))
}

fn extract_title(body: &str, path: &str) -> String {
    let mut in_front_matter = false;
    let mut front_matter_done = false;
    for line in body.lines() {
        let trimmed = line.trim();
        if !front_matter_done && trimmed == "---" {
            in_front_matter = !in_front_matter;
            if !in_front_matter { front_matter_done = true; }
            continue;
        }
        if in_front_matter {
            if let Some(value) = trimmed.strip_prefix("title:") {
                return value.trim().trim_matches(['"', '\'']).to_string();
            }
            continue;
        }
        if let Some(heading) = trimmed.strip_prefix("# ") {
            return heading.trim().trim_end_matches('#').trim().replace('`', "");
        }
    }
    Path::new(path).file_stem().and_then(|v| v.to_str())
        .unwrap_or("NeoForge documentation").replace(['-', '_'], " ")
}

fn query_terms(query: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    tokenize(query).into_iter()
        .filter(|term| term.len() > 1 && !STOP_WORDS.contains(&term.as_str()))
        .filter(|term| seen.insert(term.clone()))
        .collect()
}

/// Keep the full identifier and split Java/PascalCase boundaries:
/// DeferredRegister becomes deferredregister + deferred + register.
fn tokenize(text: &str) -> Vec<String> {
    let mut output = Vec::new();
    for raw in text.split(|c: char| !c.is_alphanumeric()) {
        if raw.is_empty() { continue; }
        output.push(raw.to_lowercase());
        let chars: Vec<char> = raw.chars().collect();
        let mut start = 0usize;
        for i in 1..chars.len() {
            let previous = chars[i - 1];
            let current = chars[i];
            let next = chars.get(i + 1).copied();
            let lower_to_upper = current.is_uppercase() && (previous.is_lowercase() || previous.is_numeric());
            let acronym_boundary = previous.is_uppercase() && current.is_uppercase()
                && next.map(char::is_lowercase).unwrap_or(false);
            if lower_to_upper || acronym_boundary {
                let piece: String = chars[start..i].iter().collect();
                if !piece.is_empty() { output.push(piece.to_lowercase()); }
                start = i;
            }
        }
        let piece: String = chars[start..].iter().collect();
        if !piece.is_empty() && piece.to_lowercase() != raw.to_lowercase() {
            output.push(piece.to_lowercase());
        }
    }
    output
}

fn term_counts(text: &str) -> HashMap<String, usize> {
    let mut counts = HashMap::new();
    for term in tokenize(text) { *counts.entry(term).or_insert(0) += 1; }
    counts
}

fn make_snippet(body: &str, query: &[String]) -> String {
    let best = body.lines()
        .filter(|line| !line.trim().is_empty())
        .max_by_key(|line| {
            let counts = term_counts(line);
            query.iter().map(|term| counts.get(term).copied().unwrap_or(0)).sum::<usize>()
                + usize::from(line.trim_start().starts_with('#')) * 2
        })
        .unwrap_or("No excerpt available.");
    let mut snippet = best.trim().trim_start_matches('#').trim().to_string();
    snippet = snippet.replace("```", "").replace('`', "")
        .replace("**", "").replace("__", "").replace("~~", "");
    if snippet.chars().count() > 260 {
        snippet = snippet.chars().take(257).collect::<String>();
        snippet.push_str("...");
    }
    snippet
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_neoforge_docs_are_complete_and_version_pinned() {
        let index = NeoForgeDocsIndex::load(default_docs_root()).expect("load bundled docs");
        assert_eq!(index.page_count(), 63);
        assert_eq!(index.manifest().minecraft_version, "1.21.1");
        assert_eq!(index.manifest().category_count, 15);
    }

    #[test]
    fn java_api_identifiers_find_their_documentation_page() {
        let index = NeoForgeDocsIndex::load(default_docs_root()).expect("load bundled docs");
        let results = index.search("DataComponentType", 5);
        assert!(!results.is_empty());
        assert!(results.iter().any(|result| result.path == "items/datacomponents.md"));
    }

    #[test]
    fn registration_api_query_finds_registry_docs() {
        let index = NeoForgeDocsIndex::load(default_docs_root()).expect("load bundled docs");
        let results = index.search("DeferredRegister Registry", 5);
        assert!(!results.is_empty());
        assert!(results.iter().any(|result| result.path == "concepts/registries.md"));
    }
}
