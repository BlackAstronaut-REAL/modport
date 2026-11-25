//! Legacy tooltip/lore recovery for native Minecraft items.
//!
//! Mirrors oldVer/mcreator_content_export.py::_extract_tooltip_strings:
//! collect tooltip/list add calls, preserve adjacent Java string fragments,
//! and retain vanilla formatting codes for faithful native tooltip rendering.

use regex::Regex;
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::sync::OnceLock;
use walkdir::WalkDir;

static TOOLTIP_ADD: OnceLock<Regex> = OnceLock::new();
static FORMATTING: OnceLock<Regex> = OnceLock::new();
static JAVA_STRING: OnceLock<Regex> = OnceLock::new();

pub fn extract_legacy_lore(source_text: &str) -> Vec<String> {
    let call_pattern = TOOLTIP_ADD.get_or_init(|| {
        Regex::new(r"(?i)\b(?:tooltip|list|par\d+List)\s*\.\s*add\s*\(")
            .expect("static tooltip add regex")
    });
    let formatting_pattern = FORMATTING.get_or_init(|| {
        Regex::new(r"\b(?:TextFormatting|EnumChatFormatting|ChatFormatting|Formatting)\s*\.\s*([A-Za-z_][A-Za-z0-9_]*)")
            .expect("static tooltip formatting regex")
    });
    let string_pattern = JAVA_STRING.get_or_init(|| {
        Regex::new(r#""((?:\\.|[^"\\])*)""#).expect("static Java string regex")
    });

    let mut result = Vec::new();
    for call in call_pattern.find_iter(source_text) {
        let Some(open_rel) = source_text[call.start()..call.end()].rfind('(') else { continue };
        let open = call.start() + open_rel;
        let Ok((expression, _)) = crate::source::java::extract_balanced(source_text, open, '(', ')') else {
            continue;
        };

        let mut rewritten = String::with_capacity(expression.len());
        let mut cursor = 0usize;
        for matched in formatting_pattern.captures_iter(&expression) {
            let Some(full) = matched.get(0) else { continue };
            let name = matched.get(1).map(|value| value.as_str()).unwrap_or("");
            let Some(code) = formatting_code(name) else { continue };
            rewritten.push_str(&expression[cursor..full.start()]);
            rewritten.push_str(&format!("\"§{code}\""));
            cursor = full.end();
        }
        rewritten.push_str(&expression[cursor..]);

        let mut line = String::new();
        for matched in string_pattern.captures_iter(&rewritten) {
            let Some(raw) = matched.get(1) else { continue };
            line.push_str(&decode_java_string(raw.as_str()));
        }
        // Keep whitespace at fragment boundaries; a trailing space can be
        // intentional when adjacent styled Java strings form one sentence.
        if !line.trim().is_empty() && !result.contains(&line) {
            result.push(line);
        }
    }
    result
}

fn formatting_code(name: &str) -> Option<char> {
    Some(match name.to_ascii_uppercase().as_str() {
        "BLACK" => '0',
        "DARK_BLUE" => '1',
        "DARK_GREEN" => '2',
        "DARK_AQUA" => '3',
        "DARK_RED" => '4',
        "DARK_PURPLE" => '5',
        "GOLD" => '6',
        "GRAY" => '7',
        "DARK_GRAY" => '8',
        "BLUE" => '9',
        "GREEN" => 'a',
        "AQUA" => 'b',
        "RED" => 'c',
        "LIGHT_PURPLE" => 'd',
        "YELLOW" => 'e',
        "WHITE" => 'f',
        "OBFUSCATED" => 'k',
        "BOLD" => 'l',
        "STRIKETHROUGH" => 'm',
        "UNDERLINE" => 'n',
        "ITALIC" => 'o',
        "RESET" => 'r',
        _ => return None,
    })
}

fn decode_java_string(raw: &str) -> String {
    let mut output = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            output.push(ch);
            continue;
        }
        let Some(escaped) = chars.next() else {
            output.push('\\');
            break;
        };
        match escaped {
            'n' => output.push('\n'),
            'r' => output.push('\r'),
            't' => output.push('\t'),
            'b' => output.push('\u{0008}'),
            'f' => output.push('\u{000c}'),
            '"' => output.push('"'),
            '\'' => output.push('\''),
            '\\' => output.push('\\'),
            'u' => {
                let mut digits = String::new();
                for _ in 0..4 {
                    if let Some(digit) = chars.next() { digits.push(digit); }
                }
                if let Ok(value) = u32::from_str_radix(&digits, 16) {
                    if let Some(value) = char::from_u32(value) {
                        output.push(value);
                    } else {
                        output.push_str("\\u");
                        output.push_str(&digits);
                    }
                } else {
                    output.push_str("\\u");
                    output.push_str(&digits);
                }
            }
            other => {
                // Preserve unknown Java escapes as written rather than losing
                // potentially meaningful mod-specific tooltip content.
                output.push('\\');
                output.push(other);
            }
        }
    }
    output
}

#[derive(Debug, Clone, Default)]
pub struct LegacyLanguageLoreIndex {
    descriptions: BTreeMap<String, Vec<String>>,
}

impl LegacyLanguageLoreIndex {
    /// Walk source roots once and index all English tooltip/description strings.
    pub fn load(source_roots: &[PathBuf], source_modid: &str) -> Self {
        let mut descriptions = BTreeMap::<String, Vec<String>>::new();
        let mut language_files = Vec::new();
        let mut java_files = Vec::new();

        for root in source_roots {
            if !root.exists() { continue; }
            for entry in WalkDir::new(root).follow_links(false).into_iter().filter_map(|entry| entry.ok()) {
                if !entry.file_type().is_file() { continue; }
                let path = entry.into_path();
                let name = path.file_name().and_then(|value| value.to_str()).unwrap_or("");
                if path.extension().and_then(|value| value.to_str()).is_some_and(|value| value.eq_ignore_ascii_case("lang"))
                    && path.file_stem().and_then(|value| value.to_str()).is_some_and(|value| value.eq_ignore_ascii_case("en_us")) {
                    language_files.push(path);
                } else if name.eq_ignore_ascii_case("en_us.json") {
                    language_files.push(path);
                } else if path.extension().and_then(|value| value.to_str()).is_some_and(|value| value.eq_ignore_ascii_case("java")) {
                    java_files.push(path);
                }
            }
        }
        language_files.sort();
        java_files.sort();

        for path in language_files {
            let Ok(bytes) = fs::read(&path) else { continue };
            if path.extension().and_then(|value| value.to_str()).is_some_and(|value| value.eq_ignore_ascii_case("json")) {
                let Ok(Value::Object(values)) = serde_json::from_slice::<Value>(&bytes) else { continue };
                for (key, value) in values {
                    if let Some(value) = value.as_str() {
                        add_description(&mut descriptions, &key, value);
                    }
                }
            } else {
                let text = String::from_utf8_lossy(&bytes);
                for line in text.lines() {
                    let line = line.trim_start_matches('\u{feff}').trim();
                    if line.is_empty() || line.starts_with('#') { continue; }
                    let Some((key, value)) = line.split_once('=') else { continue };
                    add_description(&mut descriptions, key, value);
                }
            }
        }

        // Older Forge code often creates localization entries in Java instead
        // of packaging en_US.lang; recover addStringLocalization once per mod.
        let localize = Regex::new(
            r#"(?is)\baddStringLocalization\s*\(\s*["']([^"']+)["']\s*,\s*["'](?:en_US|en_us)["']\s*,\s*["']((?:\\.|[^"'\\])*)["']\s*\)"#
        ).expect("static Java localization regex");
        for path in java_files {
            let Ok(bytes) = fs::read(path) else { continue };
            let source = crate::source::java::strip_java_comments(&String::from_utf8_lossy(&bytes));
            for capture in localize.captures_iter(&source) {
                let key = capture.get(1).map(|value| value.as_str()).unwrap_or("");
                let value = capture.get(2).map(|value| value.as_str()).unwrap_or("");
                add_description(&mut descriptions, key, &decode_java_string(value));
            }
        }

        let _ = source_modid;
        Self { descriptions }
    }

    /// Resolve pre-indexed descriptions for one registry entry without rescanning files.
    pub fn for_registry(&self, registry: &str) -> Vec<String> {
        let compact = compact_registry(registry);
        let mut variants = vec![compact.clone()];
        for suffix in ["ingot", "ore", "block", "item", "tool", "sword", "staff", "bow", "helmet", "chestplate", "leggings", "boots"] {
            if compact.ends_with(suffix) && compact.len() > suffix.len() {
                variants.push(compact[..compact.len() - suffix.len()].to_owned());
            }
        }
        let mut result = Vec::new();
        for variant in variants {
            if let Some(values) = self.descriptions.get(&variant) {
                for value in values {
                    if !result.contains(value) { result.push(value.clone()); }
                }
            }
        }
        result
    }
}

pub fn extract_language_descriptions(
    source_roots: &[PathBuf],
    registry: &str,
    source_modid: &str,
) -> Vec<String> {
    LegacyLanguageLoreIndex::load(source_roots, source_modid).for_registry(registry)
}

fn add_description(
    descriptions: &mut BTreeMap<String, Vec<String>>,
    translation_key: &str,
    value: &str,
) {
    if value.trim().is_empty() { return; }
    let lowered = translation_key.trim().to_ascii_lowercase();
    let tokens: Vec<&str> = lowered.split(['.', ':']).filter(|token| !token.is_empty()).collect();
    let is_description = tokens.iter().any(|token| {
        matches!(*token, "desc" | "description" | "tooltip" | "info")
    });
    if !is_description { return; }

    let mut candidates: Vec<&str> = tokens.clone();
    if candidates.last().is_some_and(|token| {
        matches!(*token, "name" | "desc" | "description" | "tooltip")
    }) {
        candidates.pop();
    }
    for candidate in candidates.into_iter()
        .filter(|token| !matches!(*token, "item" | "tile" | "block" | "entity" | "biome" | "dimension" | "tooltip"))
        .rev().take(2)
    {
        let key = compact_registry(candidate);
        if key.is_empty() { continue; }
        let values = descriptions.entry(key).or_default();
        let clean = value.trim().to_owned();
        if !values.contains(&clean) { values.push(clean); }
    }
}

fn compact_registry(value: &str) -> String {
    value.chars().filter(|ch| ch.is_ascii_alphanumeric()).flat_map(char::to_lowercase).collect()
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct LoreStyle {
    color: Option<&'static str>,
    bold: bool,
    italic: bool,
    underlined: bool,
    strikethrough: bool,
    obfuscated: bool,
}

pub fn lore_component_expressions(line: &str) -> Vec<String> {
    let mut output = Vec::new();
    let mut style = LoreStyle::default();
    let mut segment = String::new();
    let chars: Vec<char> = line.chars().collect();
    let mut index = 0usize;

    while index < chars.len() {
        if chars[index] == '\u{00a7}' && index + 1 < chars.len() {
            flush_segment(&mut output, &mut segment, style);
            index += 1;
            let code = chars[index].to_ascii_lowercase();
            if let Some(color) = color_name(code) {
                style = LoreStyle { color: Some(color), ..LoreStyle::default() };
            } else {
                match code {
                    'k' => style.obfuscated = true,
                    'l' => style.bold = true,
                    'm' => style.strikethrough = true,
                    'n' => style.underlined = true,
                    'o' => style.italic = true,
                    'r' => style = LoreStyle::default(),
                    _ => {
                        segment.push('\u{00a7}');
                        segment.push(chars[index]);
                    }
                }
            }
        } else {
            segment.push(chars[index]);
        }
        index += 1;
    }
    flush_segment(&mut output, &mut segment, style);
    output
}

fn color_name(code: char) -> Option<&'static str> {
    Some(match code {
        '0' => "BLACK",
        '1' => "DARK_BLUE",
        '2' => "DARK_GREEN",
        '3' => "DARK_AQUA",
        '4' => "DARK_RED",
        '5' => "DARK_PURPLE",
        '6' => "GOLD",
        '7' => "GRAY",
        '8' => "DARK_GRAY",
        '9' => "BLUE",
        'a' => "GREEN",
        'b' => "AQUA",
        'c' => "RED",
        'd' => "LIGHT_PURPLE",
        'e' => "YELLOW",
        'f' => "WHITE",
        _ => return None,
    })
}

fn flush_segment(output: &mut Vec<String>, segment: &mut String, style: LoreStyle) {
    if segment.is_empty() { return; }
    let escaped = java_string_escape(segment);
    let mut expression = format!("Component.literal(\"{escaped}\")");
    let mut modifiers = Vec::new();
    if let Some(color) = style.color {
        modifiers.push(format!("withColor(ChatFormatting.{color})"));
    }
    if style.bold { modifiers.push("withBold(true)".to_owned()); }
    if style.italic { modifiers.push("withItalic(true)".to_owned()); }
    if style.underlined { modifiers.push("withUnderlined(true)".to_owned()); }
    if style.strikethrough { modifiers.push("withStrikethrough(true)".to_owned()); }
    if style.obfuscated { modifiers.push("withObfuscated(true)".to_owned()); }
    if !modifiers.is_empty() {
        expression.push_str(&format!(".withStyle(style -> style.{})", modifiers.join(".")));
    }
    output.push(expression);
    segment.clear();
}

fn java_string_escape(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            ch if ch.is_control() => escaped.push_str(&format!("\\u{:04x}", ch as u32)),
            ch => escaped.push(ch),
        }
    }
    escaped
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_legacy_tooltip_lines_and_all_vanilla_formatting_codes() {
        let source = r#"
            public void addInformation(List<String> tooltip) {
                tooltip.add(TextFormatting.GOLD + "Legendary " + EnumChatFormatting.BOLD + "Ruby");
                tooltip.add(ChatFormatting.ITALIC + "Ancient");
                tooltip.add(Formatting.DARK_PURPLE + "Magic" + Formatting.RESET + " power");
                tooltip.add(TextFormatting.STRIKETHROUGH + "Deprecated");
                list.add("Plain text");
                par1List.add(EnumChatFormatting.YELLOW + "Unique");
                tooltip.add(TextFormatting.GOLD + "Legendary " + EnumChatFormatting.BOLD + "Ruby");
            }
        "#;
        assert_eq!(
            extract_legacy_lore(source),
            vec![
                "§6Legendary §lRuby",
                "§oAncient",
                "§5Magic§r power",
                "§mDeprecated",
                "Plain text",
                "§eUnique",
            ]
        );
    }

    #[test]
    fn recovers_legacy_language_description_keys_and_java_localization_calls() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("source");
        let lang = root.join("assets/demo/lang/en_US.lang");
        std::fs::create_dir_all(lang.parent().unwrap()).unwrap();
        std::fs::write(&lang, "item.ruby_sword.desc=First translated line\nitem.ruby_sword.tooltip=Second translated line\nitem.ruby_sword.name=Ruby Sword\n").unwrap();
        let json_lang = root.join("assets/demo/lang/en_us.json");
        std::fs::write(&json_lang, r#"{"item.ruby_sword.description":"Third translated line","item.ruby_sword.name":"Ruby Sword"}"#).unwrap();
        std::fs::write(root.join("Localization.java"), r#"
            LanguageRegistry.instance().addStringLocalization("item.ruby_sword.info", "en_US", "Fourth translated line");
        "#).unwrap();

        let lore = extract_language_descriptions(&[root], "ruby_sword", "demo");
        assert_eq!(lore, vec![
            "First translated line",
            "Second translated line",
            "Third translated line",
            "Fourth translated line",
        ]);
    }

    #[test]
    fn turns_legacy_format_codes_into_native_component_styles() {
        let expressions = lore_component_expressions("§6Gold §lBold§r Plain §oItalic");
        assert_eq!(expressions.len(), 4);
        assert!(expressions[0].contains("ChatFormatting.GOLD"));
        assert!(expressions[1].contains("withBold(true)"));
        assert_eq!(expressions[2], "Component.literal(\" Plain \")");
        assert!(expressions[3].contains("withItalic(true)"));
    }

    #[test]
    fn preserves_escaped_java_strings_and_ignores_unrecognized_format_constants() {
        let source = r#"
            tooltip.add(TextFormatting.GOLD + "Line\nOne");
            tooltip.add(TextFormatting.NOT_A_REAL_CODE + "Fallback");
        "#;
        let lore = extract_legacy_lore(source);
        assert_eq!(lore[0], "§6Line\nOne");
        assert_eq!(lore[1], "Fallback");
    }
}
