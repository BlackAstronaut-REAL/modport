//! Detect the Minecraft/Forge source API generation before legacy API matching.
//!
//! A mod's own version is deliberately never treated as the Minecraft version.
//! Strong sources are explicit mcmod.info/minecraft dependency metadata and
//! build coordinates; source identifiers are fallback evidence only.

use regex::Regex;
use serde::Serialize;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::sync::OnceLock;
use walkdir::WalkDir;

use crate::metadata::ModMetadata;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VersionEvidence {
    pub minecraft_version: String,
    pub source: String,
    pub confidence: String,
    pub detail: String,
    pub score: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SourceVersionDetection {
    /// Exact Minecraft version when evidence is sufficient; otherwise empty.
    pub minecraft_version: String,
    /// Supported API family: 1.7.10, 1.8.9, 1.12.2 or 1.21.1.
    pub api_release: String,
    /// Forge/NeoForge version string when present.
    pub loader_family: String,
    pub loader_version: String,
    /// high, medium, low, ambiguous, or unknown.
    pub confidence: String,
    /// detected, ambiguous, or unknown.
    pub status: String,
    pub evidence: Vec<VersionEvidence>,
    pub notes: Vec<String>,
}

impl SourceVersionDetection {
    /// The version filter to use for the versioned source API dictionary.
    pub fn api_release_filter(&self) -> Option<&str> {
        match self.api_release.as_str() {
            "1.7.10" | "1.8.9" | "1.12.2" | "1.21.1" => Some(&self.api_release),
            _ => None,
        }
    }
}

fn version_pattern() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"(?i)(1\.7\.10|1\.8\.9|1\.12\.2|1\.21\.1)").expect("static MC version regex")
    })
}

fn minecraft_key_pattern() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(
            r#"(?im)^\s*(?:ext\.)?(?:minecraft_version|minecraftVersion|mc_version|mcVersion|mcversion|minecraft)\s*[:=]\s*["']?\s*(1\.\d+(?:\.\d+)?)"#
        ).expect("static Minecraft property regex")
    })
}

fn forge_coordinate_pattern() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(
            r#"(?i)(?:net\.minecraftforge:forge|minecraftforge:forge|forge)\s*[:= ]\s*['"]?(1\.\d+(?:\.\d+)?)-(10\.13\.4|11\.15\.1|14\.23\.5)(?:\.\d+)?"#
        ).expect("static Forge dependency coordinate regex")
    })
}

fn neoforge_version_pattern() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r#"(?i)(?:net\.neoforged:neoforge|neoforge)\s*[:= ]\s*['"]?(\d+\.\d+(?:\.\d+)?)"#)
            .expect("static NeoForge dependency coordinate regex")
    })
}

fn forge_version_pattern() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"(?i)(10\.13\.4|11\.15\.1|14\.23\.5)(?:\.\d+)?")
            .expect("static Forge version regex")
    })
}

fn dependency_range_pattern() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r#"(?i)versionRange\s*=\s*["']\s*[\[(]\s*(1\.\d+(?:\.\d+)?)"#)
            .expect("static Minecraft dependency range regex")
    })
}

fn canonical_mc_version(value: &str) -> Option<&'static str> {
    let found = version_pattern().captures(value)?.get(1)?.as_str();
    match found {
        "1.7.10" => Some("1.7.10"),
        "1.8.9" => Some("1.8.9"),
        "1.12.2" => Some("1.12.2"),
        "1.21.1" => Some("1.21.1"),
        _ => None,
    }
}

fn version_from_forge(value: &str) -> Option<&'static str> {
    let found = forge_version_pattern().captures(value)?.get(1)?.as_str();
    match found {
        "10.13.4" => Some("1.7.10"),
        "11.15.1" => Some("1.8.9"),
        "14.23.5" => Some("1.12.2"),
        _ => None,
    }
}

fn add_evidence(
    evidence: &mut Vec<VersionEvidence>,
    version: &str,
    source: impl Into<String>,
    confidence: &str,
    detail: impl Into<String>,
    score: u16,
) {
    let Some(version) = canonical_mc_version(version) else { return; };
    let source = source.into();
    let detail = detail.into();
    if evidence.iter().any(|item| {
        item.minecraft_version == version && item.source == source && item.detail == detail
    }) {
        return;
    }
    evidence.push(VersionEvidence {
        minecraft_version: version.to_owned(),
        source,
        confidence: confidence.to_owned(),
        detail,
        score,
    });
}

fn add_forge_evidence(
    evidence: &mut Vec<VersionEvidence>,
    forge_version: &str,
    source: impl Into<String>,
    detail: impl Into<String>,
    score: u16,
) {
    if let Some(version) = version_from_forge(forge_version) {
        add_evidence(
            evidence,
            version,
            source,
            "high",
            format!("{}; Forge release family implies Minecraft {}", detail.into(), version),
            score,
        );
    }
}

fn metadata_evidence(metadata: &ModMetadata, evidence: &mut Vec<VersionEvidence>) {
    if !metadata.minecraft_version.trim().is_empty() {
        let source = if metadata.source.to_ascii_lowercase().ends_with("mcmod.info") {
            "mcmod.info mcversion"
        } else if metadata.source.to_ascii_lowercase().ends_with("mods.toml") {
            "mods.toml Minecraft dependency range"
        } else {
            "mod metadata Minecraft version"
        };
        let confidence = if source == "mcmod.info mcversion" { "high" } else { "medium" };
        let score = if confidence == "high" { 130 } else { 95 };
        add_evidence(
            evidence,
            &metadata.minecraft_version,
            source,
            confidence,
            format!("Minecraft version field recovered from {}", metadata.source),
            score,
        );
    }
    if !metadata.forge_version.trim().is_empty() {
        add_forge_evidence(
            evidence,
            &metadata.forge_version,
            "mods.toml Forge dependency",
            format!("Forge version/range {}", metadata.forge_version),
            105,
        );
    }
    if !metadata.neoforge_version.trim().is_empty() && !metadata.minecraft_version.trim().is_empty() {
        add_evidence(
            evidence,
            &metadata.minecraft_version,
            "neoforge.mods.toml NeoForge dependency",
            "high",
            format!("NeoForge version/range {} is declared for Minecraft {}", metadata.neoforge_version, metadata.minecraft_version),
            118,
        );
    }
}

fn inspect_build_text(path: &Path, text: &str, evidence: &mut Vec<VersionEvidence>) {
    let source_path = path.to_string_lossy().to_string();
    for capture in minecraft_key_pattern().captures_iter(text) {
        if let Some(version) = capture.get(1) {
            add_evidence(
                evidence,
                version.as_str(),
                format!("{source_path} Minecraft property"),
                "high",
                format!("Explicit Minecraft build property '{}'", version.as_str()),
                125,
            );
        }
    }

    for capture in forge_coordinate_pattern().captures_iter(text) {
        let minecraft = capture.get(1).map(|value| value.as_str()).unwrap_or_default();
        let forge = capture.get(2).map(|value| value.as_str()).unwrap_or_default();
        add_evidence(
            evidence,
            minecraft,
            format!("{source_path} Forge dependency coordinate"),
            "high",
            format!("Forge coordinate targets Minecraft {minecraft} (Forge {forge})"),
            125,
        );
    }

    if path.file_name().and_then(|name| name.to_str()).is_some_and(|name| {
        name.eq_ignore_ascii_case("mods.toml") || name.eq_ignore_ascii_case("neoforge.mods.toml")
    }) {
        for capture in dependency_range_pattern().captures_iter(text) {
            if let Some(version) = capture.get(1) {
                add_evidence(
                    evidence,
                    version.as_str(),
                    format!("{source_path} Minecraft dependency range"),
                    "medium",
                    format!("Minecraft dependency range begins at {}", version.as_str()),
                    92,
                );
            }
        }
    }

    for capture in neoforge_version_pattern().captures_iter(text) {
        if let Some(version) = capture.get(1) {
            if let Some(minecraft) = dependency_range_pattern().captures(text).and_then(|value| value.get(1)) {
                add_evidence(
                    evidence,
                    minecraft.as_str(),
                    format!("{source_path} NeoForge dependency coordinate"),
                    "high",
                    format!("NeoForge loader {} targets Minecraft {}", version.as_str(), minecraft.as_str()),
                    122,
                );
            }
        }
    }

    for capture in forge_version_pattern().captures_iter(text) {
        if let Some(version) = capture.get(0) {
            add_forge_evidence(
                evidence,
                version.as_str(),
                format!("{source_path} Forge version"),
                format!("Detected Forge version {}", version.as_str()),
                100,
            );
        }
    }

    if text.contains("net.neoforged.") || text.contains("net/neoforged/") {
        add_evidence(
            evidence,
            "1.21.1",
            format!("{source_path} NeoForge namespace"),
            "low",
            "NeoForge namespace indicates a modern NeoForge source; exact MC patch still needs metadata",
            35,
        );
    }

    if text.contains("cpw.mods.fml.") || text.contains("cpw/mods/fml/") {
        add_evidence(
            evidence,
            "1.7.10",
            format!("{source_path} legacy FML package"),
            "medium",
            "cpw.mods.fml package namespace is characteristic of the 1.7.10 Forge API",
            72,
        );
    }

    if text.contains("net.minecraft.util.math.BlockPos")
        || text.contains("net/minecraft/util/math/BlockPos")
    {
        add_evidence(
            evidence,
            "1.12.2",
            format!("{source_path} 1.12 package signature"),
            "medium",
            "net.minecraft.util.math.BlockPos package is characteristic of the 1.12.x mappings",
            75,
        );
    }

    if text.contains("RegistryEvent.Register")
        || text.contains("net.minecraftforge.event.RegistryEvent")
        || text.contains("net/minecraftforge/event/RegistryEvent")
    {
        add_evidence(
            evidence,
            "1.12.2",
            format!("{source_path} Forge registry event API"),
            "medium",
            "RegistryEvent.Register-style registration is characteristic of the 1.12 Forge API",
            76,
        );
    }

    if text.contains("net.minecraft.util.BlockPos")
        || text.contains("net/minecraft/util/BlockPos")
    {
        add_evidence(
            evidence,
            "1.8.9",
            format!("{source_path} 1.8 package signature"),
            "low",
            "net.minecraft.util.BlockPos indicates the post-1.7 API layout; not sufficient alone to distinguish 1.8 from 1.12",
            42,
        );
    }
}

/// Detect supported Minecraft/Forge API generation from mod metadata, Gradle
/// coordinates/properties, dependency descriptors, and source-level signatures.
///
/// Version evidence is intentionally preserved in the report. Conflicting strong
/// declarations result in an ambiguous detection instead of silently selecting
/// the wrong source API dictionary.
pub fn detect_source_version(
    roots: &[std::path::PathBuf],
    metadata: Option<&ModMetadata>,
) -> SourceVersionDetection {
    let mut evidence = Vec::new();
    if let Some(metadata) = metadata {
        metadata_evidence(metadata, &mut evidence);
    }

    for root in roots {
        for entry in WalkDir::new(root).follow_links(false).into_iter().filter_map(|entry| entry.ok()) {
            if !entry.file_type().is_file() { continue; }
            let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
            let extension = entry.path().extension().and_then(|value| value.to_str()).unwrap_or("").to_ascii_lowercase();
            let relevant_build_file = matches!(
                name.as_str(),
                "mcmod.info" | "mods.toml" | "neoforge.mods.toml" | "gradle.properties"
                    | "build.gradle" | "build.gradle.kts" | "pom.xml" | "manifest.mf"
            );
            let relevant_source = extension == "java";
            if !relevant_build_file && !relevant_source { continue; }

            let Ok(text) = fs::read_to_string(entry.path()) else { continue; };
            if relevant_build_file {
                inspect_build_text(entry.path(), &text, &mut evidence);
            } else if text.contains("cpw.mods.fml.")
                || text.contains("net.minecraftforge.event.RegistryEvent")
                || text.contains("net.minecraft.util.math.BlockPos")
                || text.contains("net.neoforged.")
            {
                // Full source/API parsing is performed by the method dictionary.
                inspect_build_text(entry.path(), &text, &mut evidence);
            }
        }
    }

    let mut scores = BTreeMap::<String, u32>::new();
    for item in &evidence {
        *scores.entry(item.minecraft_version.clone()).or_insert(0) += item.score as u32;
    }

    let mut ranked: Vec<(String, u32)> = scores.into_iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

    let mut result = SourceVersionDetection {
        minecraft_version: String::new(),
        api_release: String::new(),
        loader_family: metadata.map(|value| value.loader_family.clone()).unwrap_or_default(),
        loader_version: metadata
            .map(|value| if !value.forge_version.is_empty() { value.forge_version.clone() } else { value.neoforge_version.clone() })
            .unwrap_or_default(),
        confidence: "unknown".to_owned(),
        status: "unknown".to_owned(),
        evidence,
        notes: Vec::new(),
    };

    if ranked.is_empty() {
        result.notes.push(
            "No explicit Minecraft version or reliable Forge API signature was found. The full multi-version dictionary should be retained rather than guessing."
                .to_owned(),
        );
        return result;
    }

    let top = &ranked[0];
    let second_score = ranked.get(1).map(|entry| entry.1).unwrap_or(0);
    let strong_conflicting_evidence = result.evidence.iter().any(|item| {
        item.score >= 90 && item.minecraft_version != top.0
    });
    if strong_conflicting_evidence || (second_score >= 65 && top.1.saturating_sub(second_score) < 35) {
        result.status = "ambiguous".to_owned();
        result.confidence = "ambiguous".to_owned();
        result.notes.push(
            "Conflicting source-version declarations or similarly weighted API clues were found. Resolve the selected source root before filtering the dictionary."
                .to_owned(),
        );
        return result;
    }

    result.minecraft_version = top.0.clone();
    result.api_release = top.0.clone();
    result.confidence = evidence_confidence(top.1.min(u16::MAX as u32) as u16).to_owned();
    result.status = "detected".to_owned();

    if result.loader_version.is_empty() {
        if let Some(loader) = result.evidence.iter().find(|item| {
            item.minecraft_version == result.minecraft_version && item.detail.contains("Forge version")
        }) {
            result.loader_version = loader.detail.clone();
        }
    }
    result.notes.push(format!(
        "Selected {} from {} piece(s) of version evidence. Evidence is retained for auditing.",
        result.minecraft_version,
        result.evidence.iter().filter(|item| item.minecraft_version == result.minecraft_version).count()
    ));
    result
}

fn evidence_confidence(score: u16) -> &'static str {
    match score {
        110.. => "high",
        65.. => "medium",
        _ => "low",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_legacy_generation_from_mcmod_info_metadata() {
        let metadata = ModMetadata {
            minecraft_version: "1.7.10".to_owned(),
            source: "mcmod.info".to_owned(),
            version: "3.4.2".to_owned(),
            ..Default::default()
        };
        let detected = detect_source_version(&[], Some(&metadata));
        assert_eq!(detected.api_release, "1.7.10");
        assert_eq!(detected.confidence, "high");
    }

    #[test]
    fn detects_minecraft_version_from_gradle_properties() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("gradle.properties"), "minecraft_version=1.8.9\nforge_version=11.15.1.2318\n").unwrap();
        let detected = detect_source_version(&[root.path().to_path_buf()], None);
        assert_eq!(detected.api_release, "1.8.9");
        assert_eq!(detected.status, "detected");
    }

    #[test]
    fn detects_112_from_forge_registry_api_signature() {
        let root = tempfile::tempdir().unwrap();
        fs::write(
            root.path().join("Registration.java"),
            "import net.minecraftforge.event.RegistryEvent; class Registration { RegistryEvent.Register event; }",
        ).unwrap();
        let detected = detect_source_version(&[root.path().to_path_buf()], None);
        assert_eq!(detected.api_release, "1.12.2");
    }

    #[test]
    fn detects_neoforge_family_and_loader_version_from_metadata() {
        let metadata = ModMetadata {
            minecraft_version: "1.21.1".to_owned(),
            neoforge_version: "21.1.100".to_owned(),
            loader_family: "NeoForge".to_owned(),
            source: "neoforge.mods.toml".to_owned(),
            ..Default::default()
        };
        let detected = detect_source_version(&[], Some(&metadata));
        assert_eq!(detected.api_release, "1.21.1");
        assert_eq!(detected.loader_family, "NeoForge");
        assert_eq!(detected.loader_version, "21.1.100");
    }

    #[test]
    fn refuses_to_guess_when_high_confidence_versions_conflict() {
        let metadata = ModMetadata {
            minecraft_version: "1.7.10".to_owned(),
            source: "mcmod.info".to_owned(),
            ..Default::default()
        };
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("gradle.properties"), "minecraft_version=1.12.2\n").unwrap();
        let detected = detect_source_version(&[root.path().to_path_buf()], Some(&metadata));
        assert_eq!(detected.status, "ambiguous");
        assert!(detected.api_release.is_empty());
    }
}
