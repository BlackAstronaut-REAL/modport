use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProjectSources {
    pub compiled: String,
    pub decompiled: String,
    pub source: String,
}

impl ProjectSources {
    pub fn paths(&self) -> Vec<PathBuf> {
        [&self.compiled, &self.decompiled, &self.source]
            .into_iter()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .collect()
    }

    fn normalized_roots(&self) -> Vec<String> {
        let mut paths = self.paths().into_iter().map(|path| {
            let path = fs::canonicalize(&path).unwrap_or(path);
            let value = path.to_string_lossy().replace('\\', "/");
            if cfg!(windows) { value.to_ascii_lowercase() } else { value }
        }).collect::<Vec<_>>();
        paths.sort();
        paths.dedup();
        paths
    }

    fn from_manifest(value: &Value) -> Self {
        let sources = value.get("sources").unwrap_or(&Value::Null);
        Self {
            compiled: sources.get("compiled").and_then(Value::as_str).unwrap_or("").to_owned(),
            decompiled: sources.get("decompiled").and_then(Value::as_str).unwrap_or("").to_owned(),
            source: sources.get("source").and_then(Value::as_str).unwrap_or("").to_owned(),
        }
    }

    fn manifest_value(&self) -> Value {
        json!({
            "compiled": self.compiled,
            "decompiled": self.decompiled,
            "source": self.source,
        })
    }
}

#[derive(Debug, Clone)]
pub struct StoredProject {
    pub name: String,
    pub sources: ProjectSources,
    pub manifest_path: PathBuf,
}

fn app_data_directory() -> PathBuf {
    if let Some(path) = std::env::var_os("APPDATA") {
        return PathBuf::from(path).join("MojModelBridge");
    }
    if let Some(path) = std::env::var_os("LOCALAPPDATA") {
        return PathBuf::from(path).join("MojModelBridge");
    }
    if let Some(path) = std::env::var_os("HOME") {
        return PathBuf::from(path).join(".local").join("share").join("mojmodelbridge");
    }
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
        .join(".mojmodelbridge")
}

fn projects_directory() -> PathBuf {
    app_data_directory().join("projects")
}

fn read_manifest(path: &Path) -> Option<Value> {
    fs::read(path).ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
}

pub fn load_project(manifest_path: &Path) -> Result<StoredProject> {
    let bytes = fs::read(manifest_path)
        .with_context(|| format!("Reading project manifest {}", manifest_path.display()))?;
    let value: Value = serde_json::from_slice(&bytes)
        .with_context(|| format!("Parsing project manifest {}", manifest_path.display()))?;
    let sources = ProjectSources::from_manifest(&value);
    if sources.paths().is_empty() {
        bail!("Project {} has no configured source roots.", manifest_path.display());
    }
    let name = value.get("name").and_then(Value::as_str)
        .filter(|name| !name.trim().is_empty())
        .unwrap_or("Mod project")
        .to_owned();
    Ok(StoredProject {
        name,
        sources,
        manifest_path: manifest_path.to_path_buf(),
    })
}

pub fn load_recent_project() -> Option<StoredProject> {
    let recent_path = app_data_directory().join("last-project.txt");
    let manifest = fs::read_to_string(recent_path).ok()?;
    load_project(Path::new(manifest.trim())).ok()
}

pub fn list_projects() -> Vec<StoredProject> {
    let directory = projects_directory();
    let Ok(entries) = fs::read_dir(directory) else { return Vec::new(); };
    let mut projects = entries.filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false))
        .filter_map(|entry| load_project(&entry.path().join("project.json")).ok())
        .collect::<Vec<_>>();
    projects.sort_by_key(|project| project.name.to_ascii_lowercase());
    projects
}

fn project_slug(name: &str) -> String {
    let mut slug = String::new();
    let mut previous_separator = false;
    for character in name.trim().chars() {
        if character.is_ascii_alphanumeric() {
            slug.push(character.to_ascii_lowercase());
            previous_separator = false;
        } else if !previous_separator && !slug.is_empty() {
            slug.push('-');
            previous_separator = true;
        }
    }
    let slug = slug.trim_matches('-');
    if slug.is_empty() { "mod-project".to_owned() } else { slug.to_owned() }
}

/// Persist the original input paths. Projects are matched by their canonical
/// root set, so importing the same folders again updates the existing project.
pub fn save_project(name: &str, sources: &ProjectSources) -> Result<PathBuf> {
    let roots = sources.normalized_roots();
    if roots.is_empty() {
        bail!("Select at least one source folder before creating a project.");
    }
    for path in sources.paths() {
        if !path.is_dir() {
            bail!("Source folder '{}' does not exist.", path.display());
        }
    }

    let projects_dir = projects_directory();
    fs::create_dir_all(&projects_dir)
        .with_context(|| format!("Creating project storage at {}", projects_dir.display()))?;

    let mut existing_manifest = None;
    for entry in fs::read_dir(&projects_dir)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() { continue; }
        let manifest = entry.path().join("project.json");
        let Some(value) = read_manifest(&manifest) else { continue; };
        let existing = ProjectSources::from_manifest(&value);
        if existing.normalized_roots() == roots {
            existing_manifest = Some(manifest);
            break;
        }
    }

    let manifest_path = if let Some(manifest) = existing_manifest {
        manifest
    } else {
        let base = project_slug(name);
        let mut candidate = projects_dir.join(&base);
        let mut suffix = 2usize;
        while candidate.exists() {
            candidate = projects_dir.join(format!("{base}-{suffix}"));
            suffix += 1;
        }
        fs::create_dir_all(&candidate)?;
        candidate.join("project.json")
    };

    let display_name = if name.trim().is_empty() { "Mod project" } else { name.trim() };
    let manifest = json!({
        "schema_version": 1,
        "name": display_name,
        "sources": sources.manifest_value(),
        "root_paths": roots,
    });
    fs::write(&manifest_path, format!("{}\n", serde_json::to_string_pretty(&manifest)?))
        .with_context(|| format!("Writing project manifest {}", manifest_path.display()))?;
    fs::write(app_data_directory().join("last-project.txt"), manifest_path.to_string_lossy().as_bytes())?;
    Ok(manifest_path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_paths_preserve_roles_and_deduplicate_for_scanning() {
        let temp = tempfile::tempdir().unwrap();
        let compiled = temp.path().join("compiled");
        let source = temp.path().join("source");
        fs::create_dir_all(&compiled).unwrap();
        fs::create_dir_all(&source).unwrap();
        let sources = ProjectSources {
            compiled: compiled.to_string_lossy().to_string(),
            decompiled: String::new(),
            source: source.to_string_lossy().to_string(),
        };
        assert_eq!(sources.paths(), vec![compiled.clone(), source.clone()]);
        assert_eq!(sources.normalized_roots().len(), 2);
        let decoded = ProjectSources::from_manifest(&json!({
            "sources": {"compiled": sources.compiled, "decompiled": "", "source": sources.source}
        }));
        assert_eq!(decoded.compiled, compiled.to_string_lossy());
        assert_eq!(decoded.source, source.to_string_lossy());
    }

    #[test]
    fn project_slug_is_stable_and_safe_for_folder_names() {
        assert_eq!(project_slug("Lost Infinity: Reborn!"), "lost-infinity-reborn");
        assert_eq!(project_slug(""), "mod-project");
    }
}
