//! Offline, version-pinned NeoForge documentation retrieval.
//!
//! The Markdown corpus is vendored under docs/neoforge/1.21.1. Search uses
//! title, heading, path, and body fields, plus Java-identifier-aware tokenizing.
//! It requires no network access or embedding service.

use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::{BTreeSet, HashMap, HashSet};
use regex::Regex;
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
pub struct NeoForgeKnowledgeEntry {
    pub title: String,
    pub section: String,
    pub path: String,
    pub domains: Vec<String>,
    pub headings: Vec<String>,
    pub api_symbols: Vec<String>,
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
    pub domains: Vec<String>,
    pub api_symbols: Vec<String>,
}

struct IndexedPage {
    meta: NeoForgeDocPageMeta,
    title: String,
    body: String,
    title_terms: HashMap<String, usize>,
    heading_terms: HashMap<String, usize>,
    path_terms: HashMap<String, usize>,
    body_terms: HashMap<String, usize>,
    domains: Vec<String>,
    headings: Vec<String>,
    api_symbols: Vec<String>,
}

pub struct NeoForgeDocsIndex {
    root: PathBuf,
    manifest: NeoForgeDocsManifest,
    pages: Vec<IndexedPage>,
    document_frequency: HashMap<String, usize>,
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
            let headings = extract_headings(&body);
            let heading_text = headings.join(" ");
            let domains = classify_domains(&meta.path, &body);
            let api_symbols = extract_api_symbols(&body);
            pages.push(IndexedPage {
                title_terms: term_counts(&title),
                heading_terms: term_counts(&heading_text),
                path_terms: term_counts(&meta.path.replace('/', " ")),
                body_terms: term_counts(&body),
                meta: meta.clone(),
                title,
                body,
                domains,
                headings,
                api_symbols,
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

        let mut document_frequency = HashMap::new();
        for page in &pages {
            let mut unique_terms = HashSet::new();
            unique_terms.extend(page.title_terms.keys().cloned());
            unique_terms.extend(page.heading_terms.keys().cloned());
            unique_terms.extend(page.path_terms.keys().cloned());
            unique_terms.extend(page.body_terms.keys().cloned());
            for term in unique_terms {
                *document_frequency.entry(term).or_insert(0) += 1;
            }
        }
        Ok(Self { root, manifest, pages, document_frequency })
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
                let document_frequency = *self.document_frequency.get(term).unwrap_or(&0) as f32;
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
                    domains: page.domains.clone(),
                    api_symbols: page.api_symbols.clone(),
                });
            }
        }

        results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(Ordering::Equal)
            .then_with(|| a.path.cmp(&b.path)));
        results.truncate(limit);
        results
    }

    /// Return the complete structured API catalogue derived from the vendored pages.
    pub fn knowledge_entries(&self) -> Vec<NeoForgeKnowledgeEntry> {
        self.pages.iter().map(|page| NeoForgeKnowledgeEntry {
            title: page.title.clone(),
            section: page.meta.section.clone(),
            path: page.meta.path.clone(),
            domains: page.domains.clone(),
            headings: page.headings.clone(),
            api_symbols: page.api_symbols.clone(),
            source_url: page.meta.source_url.clone(),
        }).collect()
    }

    /// Retrieve version-specific documentation by conversion domain and optional query.
    /// The domain may be all to include every page.
    pub fn guide(&self, domain: &str, query: Option<&str>, limit: usize) -> Vec<DocSearchResult> {
        if limit == 0 { return Vec::new(); }
        let domain = canonical_domain(domain);
        let mut results = if let Some(query) = query.filter(|value| !value.trim().is_empty()) {
            self.search(query, self.pages.len())
        } else {
            self.pages.iter().map(|page| DocSearchResult {
                title: page.title.clone(),
                section: page.meta.section.clone(),
                path: page.meta.path.clone(),
                source_url: page.meta.source_url.clone(),
                score: 0.0,
                snippet: make_snippet(&page.body, &[]),
                domains: page.domains.clone(),
                api_symbols: page.api_symbols.clone(),
            }).collect()
        };
        if domain != "all" {
            results.retain(|result| result.domains.iter().any(|candidate| candidate == &domain));
        }
        results.truncate(limit);
        results
    }

    /// List domains recognized by this documentation snapshot.
    pub fn domains(&self) -> Vec<String> {
        let mut domains = BTreeSet::new();
        for page in &self.pages {
            domains.extend(page.domains.iter().cloned());
        }
        domains.into_iter().collect()
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
                return value.trim().trim_matches(|c| c == '"' || c == '\'').to_string();
            }
            continue;
        }
        if let Some(heading) = trimmed.strip_prefix("# ") {
            return heading.trim().trim_end_matches('#').trim().replace('`', "");
        }
    }
    Path::new(path).file_stem().and_then(|v| v.to_str())
        .unwrap_or("NeoForge documentation").replace('-', " ").replace('_', " ")
}



fn canonical_domain(domain: &str) -> String {
    match domain.trim().to_ascii_lowercase().replace('-', "_").as_str() {
        "block" => "blocks".to_owned(),
        "item" => "items".to_owned(),
        "entity" => "entities".to_owned(),
        "blockentity" | "block_entities" => "blockentities".to_owned(),
        "model" | "model_rendering" => "models".to_owned(),
        "recipe" => "recipes".to_owned(),
        "loot" | "loottables" => "loot_tables".to_owned(),
        "data" | "datastorage" => "data_storage".to_owned(),
        other => other.to_owned(),
    }
}

fn classify_domains(path: &str, body: &str) -> Vec<String> {
    let path = path.to_ascii_lowercase();
    let mut domains = BTreeSet::new();
    if path.starts_with("blocks/") { domains.insert("blocks".to_owned()); }
    if path.starts_with("blockentities/") {
        domains.insert("blockentities".to_owned());
        domains.insert("entities".to_owned());
        domains.insert("models".to_owned());
    }
    if path.starts_with("items/") { domains.insert("items".to_owned()); }
    if path.starts_with("concepts/registries") { domains.insert("registries".to_owned()); }
    if path.starts_with("concepts/events") { domains.insert("events".to_owned()); }
    if path.starts_with("concepts/sides") { domains.insert("sides".to_owned()); }
    if path.starts_with("datastorage/") { domains.insert("data_storage".to_owned()); }
    if path.starts_with("inventories/") { domains.insert("inventories".to_owned()); }
    if path.starts_with("gui/") { domains.insert("gui".to_owned()); }
    if path.starts_with("networking/") { domains.insert("networking".to_owned()); }
    if path.starts_with("worldgen/") { domains.insert("worldgen".to_owned()); }
    if path.starts_with("resources/client/models/") {
        domains.insert("models".to_owned());
        domains.insert("rendering".to_owned());
    } else if path.starts_with("resources/client/") {
        domains.insert("client_resources".to_owned());
        domains.insert("rendering".to_owned());
    }
    if path.starts_with("resources/server/recipes/") { domains.insert("recipes".to_owned()); }
    if path.starts_with("resources/server/loottables/") { domains.insert("loot_tables".to_owned()); }
    if path.starts_with("resources/server/enchantments/") { domains.insert("enchantments".to_owned()); }
    if path.starts_with("resources/server/advancements") { domains.insert("advancements".to_owned()); }
    if path.starts_with("resources/server/tags") { domains.insert("tags".to_owned()); }
    if path.starts_with("resources/server/datamaps/") { domains.insert("data_maps".to_owned()); }
    if path == "resources/index.md" { domains.insert("resources".to_owned()); }
    if path.starts_with("misc/config") { domains.insert("config".to_owned()); }
    if path.starts_with("misc/keymappings") {
        domains.insert("keymappings".to_owned());
        domains.insert("client_input".to_owned());
    }
    if path.starts_with("advanced/") { domains.insert("advanced".to_owned()); }
    if path.starts_with("gettingstarted/") { domains.insert("project_setup".to_owned()); }

    let lower = body.to_ascii_lowercase();
    if lower.contains("deferredregister") || lower.contains("registerevent") {
        domains.insert("registries".to_owned());
    }
    if lower.contains("itemstack") || lower.contains("datacomponenttype") {
        domains.insert("items".to_owned());
    }
    if lower.contains("biomemodifier") || lower.contains("configuredfeature") {
        domains.insert("worldgen".to_owned());
    }
    if lower.contains("modelbakery") || lower.contains("bakedmodel") || lower.contains("modelloaders") {
        domains.insert("models".to_owned());
    }
    if lower.contains("payload") && lower.contains("streamcodec") {
        domains.insert("networking".to_owned());
    }
    domains.into_iter().collect()
}

fn extract_headings(body: &str) -> Vec<String> {
    let mut headings = BTreeSet::new();
    let mut front_matter = false;
    let mut front_matter_finished = false;
    for line in body.lines() {
        let trimmed = line.trim();
        if !front_matter_finished && trimmed == "---" {
            front_matter = !front_matter;
            if !front_matter { front_matter_finished = true; }
            continue;
        }
        if front_matter { continue; }
        let count = trimmed.chars().take_while(|character| *character == '#').count();
        if count == 0 || count > 6 { continue; }
        let heading = trimmed[count..].trim().trim_end_matches('#').trim();
        if !heading.is_empty() {
            headings.insert(heading.replace(char::from(96), "").replace('*', ""));
        }
    }
    headings.into_iter().collect()
}

fn extract_api_symbols(body: &str) -> Vec<String> {
    let qualified_identifier = Regex::new(
        r"@?[A-Z][A-Za-z0-9_$]*(?:(?:\.|#|::)[A-Za-z_$][A-Za-z0-9_$]*)*"
    ).expect("static qualified identifier regex is valid");
    let method_reference = Regex::new(r"#[a-z_][A-Za-z0-9_]*")
        .expect("static method reference regex is valid");
    let mut symbols = BTreeSet::new();
    let mut in_fenced_code = false;
    for line in body.lines() {
        let trimmed = line.trim_start();
        let first_three: Vec<char> = trimmed.chars().take(3).collect();
        if first_three.len() == 3 && (first_three.iter().all(|ch| *ch == char::from(96))
            || first_three.iter().all(|ch| *ch == '~')) {
            in_fenced_code = !in_fenced_code;
            continue;
        }
        if in_fenced_code {
            for capture in qualified_identifier.find_iter(line) {
                symbols.insert(capture.as_str().to_owned());
            }
            for capture in method_reference.find_iter(line) {
                symbols.insert(capture.as_str().to_owned());
            }
            continue;
        }
        let segments: Vec<&str> = line.split(char::from(96)).collect();
        for segment in segments.iter().skip(1).step_by(2) {
            for capture in qualified_identifier.find_iter(segment) {
                symbols.insert(capture.as_str().to_owned());
            }
            for capture in method_reference.find_iter(segment) {
                symbols.insert(capture.as_str().to_owned());
            }
        }
    }
    symbols.into_iter().filter(|symbol| symbol.len() > 2).collect()
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
                + if line.trim_start().starts_with('#') { 2 } else { 0 }
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
    fn structured_knowledge_extracts_registration_symbols_and_domains() {
        let index = NeoForgeDocsIndex::load(default_docs_root()).expect("load bundled docs");
        let entries = index.knowledge_entries();
        let registries = entries.iter().find(|entry| entry.path == "concepts/registries.md").unwrap();
        assert!(registries.domains.contains(&"registries".to_owned()));
        assert!(registries.api_symbols.iter().any(|symbol| symbol == "DeferredRegister"));
        assert!(registries.api_symbols.iter().any(|symbol| symbol == "RegisterEvent"));
    }

    #[test]
    fn domain_guidance_filters_to_conversion_area() {
        let index = NeoForgeDocsIndex::load(default_docs_root()).expect("load bundled docs");
        let results = index.guide("blocks", None, 20);
        assert!(!results.is_empty());
        assert!(results.iter().all(|result| result.domains.contains(&"blocks".to_owned())));
        assert!(results.iter().any(|result| result.path == "blocks/index.md"));
    }

    #[test]
    fn search_results_include_api_symbols_and_domains() {
        let index = NeoForgeDocsIndex::load(default_docs_root()).expect("load bundled docs");
        let results = index.search("DeferredRegister", 10);
        assert!(results.iter().any(|result| result.domains.contains(&"registries".to_owned())));
    }

    #[test]
    fn registration_api_query_finds_registry_docs() {
        let index = NeoForgeDocsIndex::load(default_docs_root()).expect("load bundled docs");
        let results = index.search("DeferredRegister Registry", 5);
        assert!(!results.is_empty());
        assert!(results.iter().any(|result| result.path == "concepts/registries.md"));
    }
}
