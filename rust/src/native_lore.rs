//! Legacy tooltip/lore recovery for native Minecraft items.
//!
//! Mirrors oldVer/mcreator_content_export.py::_extract_tooltip_strings:
//! collect tooltip/list add calls, preserve adjacent Java string fragments,
//! and retain vanilla formatting codes for faithful native tooltip rendering.

use regex::Regex;
use std::sync::OnceLock;

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
            rewritten.push('\u{00a7}');
            rewritten.push(code);
            cursor = full.end();
        }
        rewritten.push_str(&expression[cursor..]);

        let mut line = String::new();
        for matched in string_pattern.captures_iter(&rewritten) {
            let Some(raw) = matched.get(1) else { continue };
            line.push_str(&decode_java_string(raw.as_str()));
        }
        let line = line.trim().to_owned();
        if !line.is_empty() && !result.contains(&line) {
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
    let quoted = format!("\"{raw}\"");
    if let Ok(value) = serde_json::from_str::<String>(&quoted) {
        return value;
    }
    // Keep unknown Java-specific escapes intact rather than dropping lore.
    raw.replace("\\n", "\n")
        .replace("\\r", "\r")
        .replace("\\t", "\t")
        .replace("\\\"", "\"")
        .replace("\\\\", "\\")
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
            tooltip.add(TextFormatting.GOLD + "Line\\nOne");
            tooltip.add(TextFormatting.NOT_A_REAL_CODE + "Fallback");
        "#;
        let lore = extract_legacy_lore(source);
        assert_eq!(lore[0], "§6Line\nOne");
        assert_eq!(lore[1], "Fallback");
    }
}
