use regex::Regex;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::PathBuf;
use walkdir::WalkDir;

#[derive(Debug, Clone, Default)]
pub struct LocalizationCatalog {
    names: BTreeMap<String, String>,
    descriptions: BTreeMap<String, Vec<String>>,
}

impl LocalizationCatalog {
    pub fn load(roots: &[PathBuf], field_registry_map: &BTreeMap<String, BTreeSet<String>>) -> Self {
        let mut catalog = Self::default();
        for root in roots {
            if !root.is_dir() { continue; }
            let mut paths: Vec<PathBuf> = WalkDir::new(root)
                .follow_links(false).into_iter()
                .filter_map(|entry| entry.ok())
                .filter(|entry| entry.file_type().is_file())
                .map(|entry| entry.into_path()).collect();
            paths.sort();

            for path in &paths {
                let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else { continue };
                let lower_name = file_name.to_ascii_lowercase();
                if lower_name.ends_with(".lang") {
                    let english = path.file_stem().and_then(|stem| stem.to_str())
                        .is_some_and(|stem| stem.eq_ignore_ascii_case("en_us"));
                    if !english { continue; }
                    let Ok(bytes) = fs::read(path) else { continue };
                    for line in String::from_utf8_lossy(&bytes).lines() {
                        let line = line.trim_start_matches('\u{feff}').trim();
                        if line.is_empty() || line.starts_with('#') { continue; }
                        if let Some((key, value)) = line.split_once('=') { catalog.add(key, value); }
                    }
                } else if lower_name == "en_us.json" {
                    let Ok(bytes) = fs::read(path) else { continue };
                    let Ok(payload) = serde_json::from_slice::<Value>(&bytes) else { continue };
                    if let Some(entries) = payload.as_object() {
                        for (key, value) in entries {
                            if let Some(value) = value.as_str() { catalog.add(key, value); }
                        }
                    }
                }
            }

            // Legacy mods often put English names directly in LanguageRegistry calls.
            for path in paths.iter().filter(|path| {
                path.extension().and_then(|value| value.to_str())
                    .is_some_and(|value| value.eq_ignore_ascii_case("java"))
            }) {
                let Ok(bytes) = fs::read(path) else { continue };
                catalog.add_java_localizations(&String::from_utf8_lossy(&bytes), field_registry_map);
            }
        }
        catalog
    }

    fn add(&mut self, translation_key: &str, value: &str) {
        let clean_value = value.trim();
        let lowered = translation_key.trim().to_ascii_lowercase();
        if clean_value.is_empty() || lowered.is_empty() { return; }

        self.names.entry(lowered.clone()).or_insert_with(|| clean_value.to_owned());
        self.names.entry(compact_registry(&lowered)).or_insert_with(|| clean_value.to_owned());

        let mut tokens: Vec<String> = lowered.split(|ch| ch == '.' || ch == ':')
            .filter(|token| !token.is_empty()).map(str::to_owned).collect();
        let is_description = tokens.iter().any(|token| {
            matches!(token.as_str(), "desc" | "description" | "tooltip" | "info")
        });
        if tokens.last().is_some_and(|token| {
            matches!(token.as_str(), "name" | "desc" | "description" | "tooltip")
        }) { tokens.pop(); }

        let ignored = ["item", "tile", "block", "entity", "biome", "dimension", "tooltip"];
        let candidates: Vec<&str> = tokens.iter().map(String::as_str)
            .filter(|token| !ignored.contains(token)).collect();
        if !is_description {
            for candidate in candidates.iter().rev().take(3) {
                self.names.entry((*candidate).to_owned()).or_insert_with(|| clean_value.to_owned());
                self.names.entry(compact_registry(candidate)).or_insert_with(|| clean_value.to_owned());
            }
        } else {
            for candidate in candidates.iter().rev().take(2) {
                let key = compact_registry(candidate);
                if key.is_empty() { continue; }
                let values = self.descriptions.entry(key).or_default();
                if !values.iter().any(|value| value == clean_value) { values.push(clean_value.to_owned()); }
            }
        }
    }

    fn add_java_localizations(&mut self, source: &str, field_registry_map: &BTreeMap<String, BTreeSet<String>>) {
        let add_name = Regex::new(
            r#"(?s)\baddNameForObject\s*\(\s*([A-Za-z_$][\w$]*)\s*,\s*["'](?:en_US|en_us)["']\s*,\s*["']((?:\\.|[^"'\\])*)["']\s*\)"#
        ).expect("addNameForObject regex");
        for capture in add_name.captures_iter(source) {
            let display = decode_java_string(&capture[2]);
            if let Some(registries) = field_registry_map.get(&capture[1]) {
                for registry in registries {
                    self.add(&format!("item.{registry}.name"), &display);
                    self.add(&format!("block.{registry}.name"), &display);
                    self.add(&format!("entity.{registry}.name"), &display);
                    self.add(registry, &display);
                }
            }
        }

        let add_string = Regex::new(
            r#"(?s)\baddStringLocalization\s*\(\s*["']([^"']+)["']\s*,\s*["'](?:en_US|en_us)["']\s*,\s*["']((?:\\.|[^"'\\])*)["']\s*\)"#
        ).expect("addStringLocalization regex");
        for capture in add_string.captures_iter(source) {
            self.add(&capture[1], &decode_java_string(&capture[2]));
        }
    }

    pub fn translated_name(&self, registry: &str, fallback: &str) -> String {
        let compact = compact_registry(registry);
        let mut candidates = vec![compact.clone()];
        for suffix in ["ingot", "ore", "block", "item", "tool", "sword", "staff", "bow",
            "helmet", "chestplate", "leggings", "boots"] {
            if compact.ends_with(suffix) && compact.len() > suffix.len() {
                candidates.push(compact[..compact.len() - suffix.len()].to_owned());
            }
        }
        for candidate in candidates {
            if let Some(value) = self.names.get(&candidate) { return value.clone(); }
        }
        fallback.to_owned()
    }

    pub fn descriptions(&self, registry: &str) -> Vec<String> {
        let compact = compact_registry(registry);
        let mut candidates = vec![compact.clone()];
        for suffix in ["ingot", "ore", "block", "item", "tool", "sword", "staff", "bow",
            "helmet", "chestplate", "leggings", "boots"] {
            if compact.ends_with(suffix) && compact.len() > suffix.len() {
                candidates.push(compact[..compact.len() - suffix.len()].to_owned());
            }
        }
        let mut result = Vec::new();
        for candidate in candidates {
            if let Some(values) = self.descriptions.get(&candidate) {
                for value in values {
                    if !result.contains(value) { result.push(value.clone()); }
                }
            }
        }
        result
    }

    pub fn extract_tooltips(source: &str) -> Vec<String> {
        let calls = Regex::new(r#"(?s)\b(?:tooltip|list|par\d+List)\s*\.\s*add\s*\((.*?)\)\s*;"#)
            .expect("tooltip call regex");
        let literals = Regex::new(r#""((?:\\.|[^"\\])*)""#).expect("Java string regex");
        let formatting = [
            ("BLACK", "§0"), ("DARK_BLUE", "§1"), ("DARK_GREEN", "§2"), ("DARK_AQUA", "§3"),
            ("DARK_RED", "§4"), ("DARK_PURPLE", "§5"), ("GOLD", "§6"), ("GRAY", "§7"),
            ("DARK_GRAY", "§8"), ("BLUE", "§9"), ("GREEN", "§a"), ("AQUA", "§b"),
            ("RED", "§c"), ("LIGHT_PURPLE", "§d"), ("YELLOW", "§e"), ("WHITE", "§f"),
            ("BOLD", "§l"), ("STRIKETHROUGH", "§m"), ("UNDERLINE", "§n"),
            ("ITALIC", "§o"), ("RESET", "§r"),
        ];
        let mut result = Vec::new();
        for call in calls.captures_iter(source) {
            let expression = call.get(1).map(|value| value.as_str()).unwrap_or("");
            let mut value = String::new();
            for (name, code) in formatting {
                let pattern = format!(
                    r"\b(?:TextFormatting|EnumChatFormatting|ChatFormatting|Formatting)\.{name}\b"
                );
                if Regex::new(&pattern).is_ok_and(|regex| regex.is_match(expression)) {
                    value.push_str(code);
                }
            }
            for literal in literals.captures_iter(expression) {
                value.push_str(&decode_java_string(&literal[1]));
            }
            let value = value.trim().to_owned();
            if !value.is_empty() && !result.contains(&value) { result.push(value); }
        }
        result
    }

    pub fn item_information(&self, source: &str, registry: &str) -> Vec<String> {
        let mut result = Self::extract_tooltips(source);
        for description in self.descriptions(registry) {
            if !result.contains(&description) { result.push(description); }
        }
        result
    }
}

pub fn compact_registry(value: &str) -> String {
    value.chars().filter(|ch| ch.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase).collect()
}

fn decode_java_string(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut chars = value.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '\\' { output.push(ch); continue; }
        let Some(escaped) = chars.next() else { output.push('\\'); break };
        match escaped {
            'n' => output.push('\n'), 'r' => output.push('\r'), 't' => output.push('\t'),
            'b' => output.push('\u{0008}'), 'f' => output.push('\u{000c}'),
            '\\' => output.push('\\'), '\'' => output.push('\''), '"' => output.push('"'),
            'u' => {
                while chars.peek() == Some(&'u') { chars.next(); }
                let digits: String = chars.by_ref().take(4).collect();
                if digits.len() == 4 {
                    if let Ok(codepoint) = u32::from_str_radix(&digits, 16) {
                        if let Some(decoded) = char::from_u32(codepoint) { output.push(decoded); }
                    }
                }
            }
            digit @ '0'..='7' => {
                let mut octal = String::from(digit);
                for _ in 0..2 {
                    if chars.peek().is_some_and(|next| matches!(next, '0'..='7')) {
                        octal.push(chars.next().unwrap_or('0'));
                    } else { break; }
                }
                if let Ok(codepoint) = u32::from_str_radix(&octal, 8) {
                    if let Some(decoded) = char::from_u32(codepoint) { output.push(decoded); }
                }
            }
            other => output.push(other),
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn reads_legacy_lang_and_modern_json_names_and_descriptions() {
        let temp = tempdir().unwrap();
        let lang = temp.path().join("assets/demo/lang/en_US.lang");
        let json_path = temp.path().join("assets/demo/lang/en_us.json");
        fs::create_dir_all(lang.parent().unwrap()).unwrap();
        fs::write(&lang, concat!(
            "# english names\n",
            "item.ruby_ingot.name=Ruby Ingot\n",
            "item.ruby_ingot.desc=Shines \\u0026 glows\n"
        )).unwrap();
        fs::write(&json_path, r#"{
            "block.ruby_ore": "Ruby Ore",
            "block.ruby_ore.tooltip": "Mined deep underground"
        }"#).unwrap();
        let catalog = LocalizationCatalog::load(&[temp.path().to_path_buf()], &BTreeMap::new());
        assert_eq!(catalog.translated_name("ruby_ingot", "Ruby"), "Ruby Ingot");
        assert_eq!(catalog.descriptions("ruby_ingot"), vec!["Shines & glows"]);
        assert_eq!(catalog.translated_name("ruby_ore", "Ore"), "Ruby Ore");
        assert_eq!(catalog.descriptions("ruby_ore"), vec!["Mined deep underground"]);
    }

    #[test]
    fn reads_java_language_registry_calls_and_formatted_tooltips() {
        let temp = tempdir().unwrap();
        fs::write(temp.path().join("Names.java"), r#"
            class Names {
                void init() {
                    LanguageRegistry.instance().addNameForObject(RUBY, "en_US", "Ruby \\u0026 Gold");
                    LanguageRegistry.instance().addStringLocalization("item.java_ruby.name", "en_US", "Java Ruby");
                }
            }
        "#).unwrap();
        let fields = BTreeMap::from([(
            "RUBY".to_owned(), BTreeSet::from(["ruby_gem".to_owned()]),
        )]);
        let catalog = LocalizationCatalog::load(&[temp.path().to_path_buf()], &fields);
        assert_eq!(catalog.translated_name("ruby_gem", "Gem"), "Ruby & Gold");
        assert_eq!(catalog.translated_name("java_ruby", "Fallback"), "Java Ruby");
        assert_eq!(
            LocalizationCatalog::extract_tooltips("tooltip.add(TextFormatting.GOLD + \"A golden blade\");"),
            vec!["§6A golden blade"]
        );
        assert_eq!(
            LocalizationCatalog::extract_tooltips("par3List.add(EnumChatFormatting.ITALIC + \"Very sharp\");"),
            vec!["§oVery sharp"]
        );
    }
}
