use anyhow::Result;
use regex::Regex;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

const FILE_LIMIT: usize = 200_000;
const AUXILIARY_WORDS: &[&str] = &[
    "eye", "eyes", "glow", "glowing", "emissive", "overlay", "layer",
    "armor", "armour", "saddle", "mask", "beam", "laser", "outline",
];

#[derive(Debug, Clone)]
pub struct TextureReference {
    pub value: String,
    pub source: PathBuf,
}

#[derive(Debug, Clone)]
pub struct SourceRecord {
    pub path: PathBuf,
    pub texture_refs: Vec<String>,
    pub model_mentions: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct TextureResolution {
    pub path: Option<PathBuf>,
    pub score: f64,
    pub evidence: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct TextureSetResolution {
    pub primary: Option<PathBuf>,
    pub variants: Vec<PathBuf>,
    pub auxiliaries: Vec<PathBuf>,
    pub evidence: Vec<String>,
    pub candidate_details: Vec<(PathBuf, f64, String)>,
}

fn normalized_path(path: &Path) -> String {
    let value = path.to_string_lossy().to_string();
    if cfg!(windows) { value.to_ascii_lowercase() } else { value }
}

fn unique_existing(paths: impl IntoIterator<Item = PathBuf>) -> Vec<PathBuf> {
    let mut seen = HashSet::new();
    let mut result = Vec::new();
    for raw in paths {
        let path = fs::canonicalize(&raw).unwrap_or(raw);
        if path.exists() && seen.insert(normalized_path(&path)) {
            result.push(path);
        }
    }
    result
}

fn extract_text_references(text: &str) -> Vec<String> {
    let patterns = [
        r"(?i)(?:[a-z0-9_.-]+:)?(?:assets/)?(?:textures/)?[a-z0-9_./\\-]+\.png",
        r"(?i)(?:[a-z0-9_.-]+:)?textures/[a-z0-9_./\\-]+\.png",
    ];
    let mut refs = HashSet::new();
    for pattern in patterns {
        let regex = Regex::new(pattern).expect("fixed texture reference regex");
        for matched in regex.find_iter(text) {
            let value = matched.as_str()
                .replace('\\', "/")
                .trim_matches(|ch: char| "\"' ;,()[]{}".contains(ch))
                .to_owned();
            if value.to_ascii_lowercase().ends_with(".png") {
                refs.insert(value);
            }
        }
    }
    let mut result: Vec<_> = refs.into_iter().collect();
    result.sort_by_key(|value| value.to_ascii_lowercase());
    result
}

fn extract_model_mentions(text: &str) -> Vec<String> {
    let regex = Regex::new(r"\bModel[A-Za-z0-9_$]+\b").expect("fixed model mention regex");
    let mut mentions: HashSet<String> = regex.find_iter(text)
        .map(|matched| matched.as_str().split('$').next().unwrap_or(matched.as_str()).to_owned())
        .collect();
    let mut result: Vec<_> = mentions.drain().collect();
    result.sort_by_key(|value| value.to_ascii_lowercase());
    result
}

fn is_auxiliary_texture(path: &Path) -> bool {
    let stem = path.file_stem().and_then(|v| v.to_str()).unwrap_or("").to_ascii_lowercase();
    let tokens: HashSet<String> = Regex::new(r"[^a-z0-9]+").unwrap()
        .split(&stem).filter(|token| !token.is_empty()).map(str::to_owned).collect();
    let joined = stem.chars().filter(|ch| ch.is_ascii_alphanumeric()).collect::<String>();
    tokens.iter().any(|token| AUXILIARY_WORDS.contains(&token.as_str()))
        || AUXILIARY_WORDS.iter().any(|word| joined.ends_with(word))
}

fn canonical_name(value: &str) -> String {
    let prefix = Regex::new(r"(?i)^(model|renderer|render|entity)").unwrap();
    let stripped = prefix.replace(value, "");
    stripped.to_ascii_lowercase().chars().filter(|ch| ch.is_ascii_alphanumeric()).collect()
}

fn normalize_reference(value: &str) -> String {
    let mut normalized = value.trim().replace('\\', "/").trim_start_matches('/').to_ascii_lowercase();
    if let Some((namespace, path)) = normalized.split_once(':') {
        if !namespace.is_empty() && !namespace.contains('/') {
            normalized = path.to_owned();
        }
    }
    if let Some(rest) = normalized.strip_prefix("assets/") {
        normalized = rest.to_owned();
    }
    normalized
}

fn texture_relative_candidates(path: &Path) -> HashSet<String> {
    let parts: Vec<String> = path.components()
        .map(|part| part.as_os_str().to_string_lossy().to_ascii_lowercase())
        .collect();
    let mut result = HashSet::new();
    if let Some(name) = path.file_name().and_then(|v| v.to_str()) {
        result.insert(name.to_ascii_lowercase());
    }
    for marker in ["textures", "assets"] {
        if let Some(index) = parts.iter().position(|part| part == marker) {
            result.insert(parts[index..].join("/"));
            if marker == "assets" && index + 1 < parts.len() {
                result.insert(parts[index + 1..].join("/"));
                if index + 2 < parts.len() {
                    result.insert(parts[index + 2..].join("/"));
                }
            }
        }
    }
    result
}

fn is_png(path: &Path) -> bool {
    path.extension().and_then(|v| v.to_str())
        .map(|v| v.eq_ignore_ascii_case("png")).unwrap_or(false)
}

fn is_source(path: &Path) -> bool {
    path.extension().and_then(|v| v.to_str())
        .map(|v| v.eq_ignore_ascii_case("java") || v.eq_ignore_ascii_case("class"))
        .unwrap_or(false)
}

fn iter_files(roots: &[PathBuf], shallow_roots: &[PathBuf], predicate: impl Fn(&Path) -> bool) -> Vec<PathBuf> {
    let mut result = Vec::new();
    let mut seen = HashSet::new();
    for root in roots {
        let candidates = if root.is_file() {
            vec![root.clone()]
        } else {
            WalkDir::new(root).follow_links(false).into_iter()
                .filter_map(|entry| entry.ok())
                .filter(|entry| entry.file_type().is_file())
                .map(|entry| entry.path().to_path_buf())
                .collect()
        };
        for path in candidates {
            if result.len() >= FILE_LIMIT { return result; }
            if predicate(&path) {
                let canonical = fs::canonicalize(&path).unwrap_or(path);
                if seen.insert(normalized_path(&canonical)) {
                    result.push(canonical);
                }
            }
        }
    }
    for root in shallow_roots {
        if !root.is_dir() { continue; }
        let Ok(entries) = fs::read_dir(root) else { continue; };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() && predicate(&path) {
                let canonical = fs::canonicalize(&path).unwrap_or(path);
                if seen.insert(normalized_path(&canonical)) {
                    result.push(canonical);
                }
            }
        }
    }
    result
}

/// Port of Beta 12's companion-aware PNG resolver. It indexes source/class
/// references once, then scores exact renderer/model links before conservative
/// filename-family fallback. Eye/glow/overlay assets never become body skins.
#[derive(Debug, Clone)]
pub struct TextureIndex {
    pub roots: Vec<PathBuf>,
    pub shallow_roots: Vec<PathBuf>,
    pub png_files: Vec<PathBuf>,
    /// Exact texture resource keys -> candidate files, built once per source tree.
    png_by_resource: HashMap<String, Vec<PathBuf>>,
    /// Lowercase PNG basename -> candidate files for namespace/layout fallbacks.
    png_by_basename: HashMap<String, Vec<PathBuf>>,
    pub references: Vec<TextureReference>,
    pub records: Vec<SourceRecord>,
}

impl TextureIndex {
    pub fn new(roots: &[PathBuf], shallow_roots: &[PathBuf]) -> Result<Self> {
        let roots = unique_existing(roots.iter().cloned());
        let shallow_roots = unique_existing(shallow_roots.iter().cloned());
        let mut index = Self {
            roots,
            shallow_roots,
            png_files: Vec::new(),
            png_by_resource: HashMap::new(),
            png_by_basename: HashMap::new(),
            references: Vec::new(),
            records: Vec::new(),
        };
        index.build()?;
        Ok(index)
    }

    fn build(&mut self) -> Result<()> {
        self.png_files = iter_files(&self.roots, &self.shallow_roots, is_png);
        self.png_files.sort_by_key(|path| path.to_string_lossy().to_ascii_lowercase());

        // Every later model texture reference used to walk every PNG and rebuild
        // these keys. Precompute exact resource and basename lookups once instead.
        self.png_by_resource.clear();
        self.png_by_basename.clear();
        for png in &self.png_files {
            for key in texture_relative_candidates(png) {
                self.png_by_resource.entry(key).or_default().push(png.clone());
            }
            if let Some(name) = png.file_name().and_then(|value| value.to_str()) {
                self.png_by_basename.entry(name.to_ascii_lowercase())
                    .or_default().push(png.clone());
            }
        }

        let files = iter_files(&self.roots, &self.shallow_roots, is_source);
        for path in files {
            let bytes = match fs::read(&path) {
                Ok(bytes) => bytes,
                Err(_) => continue,
            };
            let text = if path.extension().and_then(|v| v.to_str()).map(|v| v.eq_ignore_ascii_case("java")).unwrap_or(false) {
                String::from_utf8_lossy(&bytes).trim_start_matches('\u{feff}').to_owned()
            } else {
                bytes.iter().map(|byte| char::from(*byte)).collect()
            };
            let refs = extract_text_references(&text);
            let mentions = extract_model_mentions(&text);
            for reference in &refs {
                self.references.push(TextureReference { value: reference.clone(), source: path.clone() });
            }
            self.records.push(SourceRecord { path, texture_refs: refs, model_mentions: mentions });
        }
        self.records.sort_by_key(|record| record.path.to_string_lossy().to_ascii_lowercase());
        self.references.sort_by_key(|reference| {
            (reference.source.to_string_lossy().to_ascii_lowercase(), reference.value.to_ascii_lowercase())
        });
        Ok(())
    }

    fn associated_records(&self, source: &Path, model_name: &str) -> Vec<(usize, f64, String)> {
        let source = fs::canonicalize(source).unwrap_or_else(|_| source.to_path_buf());
        let model_canon = canonical_name(model_name);
        let mut result = Vec::new();
        for (index, record) in self.records.iter().enumerate() {
            let mut score: f64 = 0.0;
            let mut reason = String::new();
            let mentions_exact = record.model_mentions.iter().any(|name| canonical_name(name) == model_canon);
            let stem_canon = canonical_name(record.path.file_stem().and_then(|v| v.to_str()).unwrap_or(""));
            if record.path == source {
                score = 100_000.0;
                reason = "texture reference in model source".to_owned();
            } else if !model_canon.is_empty() && mentions_exact {
                score = 60_000.0;
                reason = format!("companion {} references exact model class", record.path.file_name().and_then(|value| value.to_str()).unwrap_or(""));
            } else if stem_canon == model_canon {
                score = 30_000.0;
                reason = "companion name exactly matches model".to_owned();
            } else if !model_canon.is_empty() && stem_canon.ends_with(&model_canon)
                && record.path.file_stem().and_then(|v| v.to_str())
                    .map(|stem| stem.to_ascii_lowercase().starts_with("render") || stem.to_ascii_lowercase().starts_with("renderer"))
                    .unwrap_or(false)
            {
                score = 20_000.0;
                reason = "renderer name matches model".to_owned();
            }
            if score > 0.0 {
                result.push((index, score, reason));
            }
        }
        result.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| {
            self.records[a.0].path.to_string_lossy().to_ascii_lowercase()
                .cmp(&self.records[b.0].path.to_string_lossy().to_ascii_lowercase())
        }));
        result
    }

    fn match_reference(&self, reference: &str) -> Vec<(PathBuf, f64, String)> {
        let normalized = normalize_reference(reference);
        if !normalized.ends_with(".png") { return Vec::new(); }
        let name = Path::new(&normalized).file_name().and_then(|v| v.to_str()).unwrap_or("").to_ascii_lowercase();
        let exact = self.png_by_resource.get(&normalized);
        let mut exact_paths = HashSet::<PathBuf>::new();
        let mut matches = Vec::new();
        if let Some(paths) = exact {
            for path in paths {
                exact_paths.insert(path.clone());
                matches.push((path.clone(), 100_000.0, "exact resource path".to_owned()));
            }
        }
        if let Some(paths) = self.png_by_basename.get(&name) {
            for path in paths {
                if exact_paths.contains(path) { continue; }
                matches.push((path.clone(), 50_000.0, "exact resource basename".to_owned()));
            }
        }
        matches
    }

    fn family_match(path: &Path, aliases: &HashSet<String>) -> bool {
        let stem = path.file_stem().and_then(|v| v.to_str()).unwrap_or("").to_ascii_lowercase();
        for alias in aliases {
            if alias.is_empty() { continue; }
            for prefix in ["", "model", "render", "renderer", "entity"] {
                let prefix_name = format!("{prefix}{alias}");
                if let Some(suffix) = stem.strip_prefix(&prefix_name) {
                    if suffix.len() > 1 && suffix.chars().next().map(|ch| matches!(ch, '_' | '-' | '.')).unwrap_or(false) {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Resolve a primary body texture and compatible colour variants, optionally
    /// constrained to an explicit file or directory.
    pub fn resolve_set(&self, source: &Path, model_name: &str, explicit: Option<&Path>, max_variants: usize) -> Result<TextureSetResolution> {
        if let Some(explicit) = explicit {
            let explicit = explicit.canonicalize().unwrap_or_else(|_| explicit.to_path_buf());
            if explicit.is_file() {
                return Ok(TextureSetResolution {
                    primary: Some(explicit.clone()),
                    variants: vec![explicit],
                    evidence: vec!["explicit texture file".to_owned()],
                    ..Default::default()
                });
            }
            if explicit.is_dir() {
                let scoped = TextureIndex::new(&[explicit], &[source.parent().unwrap_or_else(|| Path::new(".")).to_path_buf()])?;
                return scoped.resolve_set(source, model_name, None, max_variants);
            }
            return Ok(TextureSetResolution {
                evidence: vec![format!("texture path does not exist: {}", explicit.display())],
                ..Default::default()
            });
        }

        let model_canon = canonical_name(model_name);
        let mut candidates = HashMap::<PathBuf, (f64, String)>::new();
        let associated = self.associated_records(source, model_name);
        let mut aliases = HashSet::from([model_canon.clone()]);

        for (record_index, relation_score, relation_reason) in associated {
            let record = &self.records[record_index];
            aliases.insert(canonical_name(record.path.file_stem().and_then(|v| v.to_str()).unwrap_or("")));
            for reference in &record.texture_refs {
                for (path, reference_score, reference_reason) in self.match_reference(reference) {
                    let mut score = relation_score + reference_score;
                    if canonical_name(path.file_stem().and_then(|v| v.to_str()).unwrap_or("")) == model_canon {
                        score += 30_000.0;
                    }
                    if is_auxiliary_texture(&path) {
                        score -= 90_000.0;
                    }
                    let evidence = format!("{relation_reason}; {reference_reason}: {reference}");
                    let replace = candidates.get(&path).map(|(previous, _)| score > *previous).unwrap_or(true);
                    if replace { candidates.insert(path, (score, evidence)); }
                }
            }
        }

        aliases.retain(|alias| !alias.is_empty());
        for png in &self.png_files {
            let stem_canon = canonical_name(png.file_stem().and_then(|v| v.to_str()).unwrap_or(""));
            let exact_model_skin = !model_canon.is_empty() && stem_canon == model_canon;
            if (exact_model_skin || Self::family_match(png, &aliases)) && !candidates.contains_key(png) {
                let mut score = 1_000.0 + if exact_model_skin { 5_000.0 } else { 0.0 };
                if is_auxiliary_texture(png) { score -= 90_000.0; }
                let reason = if exact_model_skin {
                    "exact model-name texture fallback"
                } else {
                    "conservative model/renderer texture-family match"
                };
                candidates.insert(png.clone(), (score, reason.to_owned()));
            }
        }

        if candidates.is_empty() {
            return Ok(TextureSetResolution::default());
        }

        let mut ordered: Vec<_> = candidates.into_iter().collect();
        ordered.sort_by(|(path_a, (score_a, _)), (path_b, (score_b, _))| {
            score_b.total_cmp(score_a)
                .then_with(|| path_a.components().count().cmp(&path_b.components().count()))
                .then_with(|| path_a.to_string_lossy().to_ascii_lowercase().cmp(&path_b.to_string_lossy().to_ascii_lowercase()))
        });

        let auxiliaries: Vec<PathBuf> = ordered.iter()
            .filter(|(path, _)| is_auxiliary_texture(path))
            .map(|(path, _)| path.clone())
            .collect();
        let body: Vec<_> = ordered.into_iter().filter(|(path, _)| !is_auxiliary_texture(path)).collect();
        if body.is_empty() {
            return Ok(TextureSetResolution {
                auxiliaries,
                evidence: vec!["only auxiliary/overlay textures matched".to_owned()],
                ..Default::default()
            });
        }

        let primary = body[0].0.clone();
        let evidence = vec![body[0].1.1.clone()];
        let details: Vec<_> = body.iter().take(max_variants).map(|(path, (score, reason))| {
            (path.clone(), *score, reason.clone())
        }).collect();
        let variants = details.iter().map(|(path, _, _)| path.clone()).collect();
        Ok(TextureSetResolution {
            primary: Some(primary),
            variants,
            auxiliaries,
            evidence,
            candidate_details: details,
        })
    }

    pub fn resolve(&self, source: &Path, model_name: &str, explicit: Option<&Path>) -> Result<TextureResolution> {
        let result = self.resolve_set(source, model_name, explicit, 32)?;
        let score = result.candidate_details.iter()
            .find(|(path, _, _)| Some(path.as_path()) == result.primary.as_deref())
            .map(|(_, score, _)| *score)
            .unwrap_or(0.0);
        Ok(TextureResolution { path: result.primary, score, evidence: result.evidence })
    }
}

/// Build the familiar Gradle/resource search roots around one Java/class file.
pub fn infer_search_roots(source: &Path, extra_roots: &[PathBuf]) -> Vec<PathBuf> {
    let source = source.canonicalize().unwrap_or_else(|_| source.to_path_buf());
    let mut roots = extra_roots.to_vec();
    let current = if source.is_file() { source.parent().unwrap_or_else(|| Path::new(".")) } else { source.as_path() };
    let ancestors = std::iter::once(current).chain(current.ancestors().skip(1).take(7));
    for ancestor in ancestors {
        for candidate in [
            ancestor.join("src/main/resources"),
            ancestor.join("src/main/java"),
            ancestor.join("src"),
            ancestor.join("resources"),
            ancestor.join("assets"),
        ] {
            if candidate.exists() { roots.push(candidate); }
        }
    }
    unique_existing(roots)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn touch(path: &Path) {
        if let Some(parent) = path.parent() { fs::create_dir_all(parent).unwrap(); }
        fs::write(path, b"not-real-png-data").unwrap();
    }

    #[test]
    fn preindexed_texture_lookup_keeps_exact_and_basename_fallbacks() {
        let temp = tempdir().unwrap();
        let root = temp.path();
        let texture = root.join("assets/demo/textures/entity/creature/creature.png");
        touch(&texture);
        let index = TextureIndex::new(&[root.to_path_buf()], &[]).unwrap();
        let exact = index.match_reference("demo:textures/entity/creature/creature.png");
        assert!(exact.iter().any(|(path, score, _)| path == &texture && *score == 100_000.0));
        let basename = index.match_reference("other:entity/creature/creature.png");
        assert!(basename.iter().any(|(path, score, _)| path == &texture && *score == 50_000.0));
    }

    #[test]
    fn companion_references_rank_body_variants_and_separate_auxiliaries() {
        let temp = tempdir().unwrap();
        let root = temp.path();
        let java = root.join("src/main/java/example");
        let resources = root.join("src/main/resources/assets/demo/textures/entity/pig");
        let model = java.join("model/ModelPig.java");
        let renderer = java.join("render/RenderPig.java");
        touch(&model);
        touch(&renderer);
        touch(&resources.join("pig.png"));
        touch(&resources.join("pig_red.png"));
        touch(&resources.join("pig_eyes.png"));
        fs::write(&model, r#"
            public class ModelPig {}
        "#).unwrap();
        fs::write(&renderer, r#"
            public class RenderPig {
                ModelPig model;
                String skin = "demo:textures/entity/pig/pig.png";
                String variant = "demo:textures/entity/pig/pig_red.png";
                String eyes = "demo:textures/entity/pig/pig_eyes.png";
            }
        "#).unwrap();

        let index = TextureIndex::new(&[java.to_path_buf(), root.join("src/main/resources")], &[]).unwrap();
        let resolved = index.resolve_set(&model, "ModelPig", None, 32).unwrap();
        assert_eq!(resolved.primary.as_ref().map(|path| path.file_name().unwrap().to_string_lossy().to_string()), Some("pig.png".into()));
        assert!(resolved.variants.iter().any(|path| path.ends_with("pig_red.png")));
        assert!(resolved.auxiliaries.iter().any(|path| path.ends_with("pig_eyes.png")));
        assert!(!resolved.variants.iter().any(|path| path.ends_with("pig_eyes.png")));
    }

    #[test]
    fn auxiliary_only_results_never_become_primary() {
        let temp = tempdir().unwrap();
        let root = temp.path();
        let source = root.join("ModelBat.java");
        let resource_root = root.join("textures");
        touch(&source);
        touch(&resource_root.join("bat_glow.png"));
        fs::write(&source, r#"class ModelBat { String texture="textures/bat_glow.png"; }"#).unwrap();
        let index = TextureIndex::new(&[root.to_path_buf()], &[]).unwrap();
        let resolved = index.resolve_set(&source, "ModelBat", None, 32).unwrap();
        assert!(resolved.primary.is_none());
        assert_eq!(resolved.auxiliaries.len(), 1);
    }
}
