use eframe::egui::{self, Color32, FontFamily, FontId, RichText};
use serde_json::Value;

#[derive(Debug, Clone)]
pub struct TextRun {
    pub text: String,
    pub color: Color32,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strikethrough: bool,
}

#[derive(Debug, Clone, Copy)]
struct TextStyle {
    color: Color32,
    bold: bool,
    italic: bool,
    underline: bool,
    strikethrough: bool,
    obfuscated: bool,
}

impl TextStyle {
    fn new(color: Color32) -> Self {
        Self {
            color,
            bold: false,
            italic: false,
            underline: false,
            strikethrough: false,
            obfuscated: false,
        }
    }
}

pub fn parse_minecraft_text(input: &str, default_color: Color32) -> Vec<TextRun> {
    parse_formatted(input, TextStyle::new(default_color))
}

fn push_run(buffer: &mut String, style: TextStyle, runs: &mut Vec<TextRun>) {
    if buffer.is_empty() {
        return;
    }
    let text = if style.obfuscated {
        obfuscate(buffer)
    } else {
        buffer.clone()
    };
    runs.push(TextRun {
        text,
        color: style.color,
        bold: style.bold,
        italic: style.italic,
        underline: style.underline,
        strikethrough: style.strikethrough,
    });
    buffer.clear();
}

fn parse_formatted(input: &str, inherited: TextStyle) -> Vec<TextRun> {
    let chars: Vec<char> = input.chars().collect();
    let mut style = inherited;
    let mut runs = Vec::<TextRun>::new();
    let mut buffer = String::new();
    let mut index = 0usize;

    while index < chars.len() {
        if chars[index] == '§' && index + 1 < chars.len() {
            let code = chars[index + 1].to_ascii_lowercase();
            push_run(&mut buffer, style, &mut runs);
            if code == 'x' && index + 13 < chars.len() {
                let mut hex = String::new();
                let mut valid = true;
                for part in 0..6 {
                    let separator = index + 2 + part * 2;
                    let digit = separator + 1;
                    if chars[separator] != '§' || !chars[digit].is_ascii_hexdigit() {
                        valid = false;
                        break;
                    }
                    hex.push(chars[digit]);
                }
                if valid {
                    style = TextStyle::new(parse_hex(&hex).unwrap_or(inherited.color));
                    index += 14;
                    continue;
                }
            }

            match code {
                '0'..='9' | 'a'..='f' => {
                    style = TextStyle::new(legacy_color(code).unwrap_or(inherited.color));
                }
                'k' => style.obfuscated = true,
                'l' => style.bold = true,
                'm' => style.strikethrough = true,
                'n' => style.underline = true,
                'o' => style.italic = true,
                'r' => style = inherited,
                _ => {
                    buffer.push('§');
                    buffer.push(chars[index + 1]);
                }
            }
            index += 2;
        } else {
            buffer.push(chars[index]);
            index += 1;
        }
    }
    push_run(&mut buffer, style, &mut runs);
    runs
}

pub fn draw_formatted_text(
    ui: &mut egui::Ui,
    text: &str,
    default_color: Color32,
    use_minecraftia: bool,
) {
    draw_runs(ui, &parse_minecraft_text(text, default_color), use_minecraftia);
}

/// Render a Java Edition text component or a component-shaped JSON value.
/// Supports string/array/object forms, inherited styles, named/hex colors,
/// extra siblings, and the six classic formatting flags.
pub fn draw_component_json(
    ui: &mut egui::Ui,
    value: &Value,
    default_color: Color32,
    use_minecraftia: bool,
) {
    let base = TextStyle::new(default_color);
    let mut runs = Vec::new();

    // If this is an item component map, render its custom name and lore rather
    // than dumping implementation keys into the tooltip.
    let custom_name = value.get("minecraft:custom_name")
        .or_else(|| value.get("custom_name"));
    let lore = value.get("minecraft:lore")
        .or_else(|| value.get("lore"));

    if custom_name.is_some() || lore.is_some() {
        if let Some(name) = custom_name {
            append_component(name, base, &mut runs);
        }
        if let Some(lines) = lore {
            if let Some(lines) = lines.as_array() {
                for line in lines {
                    runs.push(TextRun {
                        text: "\n".to_owned(),
                        color: default_color,
                        bold: false,
                        italic: false,
                        underline: false,
                        strikethrough: false,
                    });
                    append_component(line, base, &mut runs);
                }
            } else {
                append_component(lines, base, &mut runs);
            }
        }
    } else {
        append_component(value, base, &mut runs);
    }

    draw_runs(ui, &runs, use_minecraftia);
}

fn append_component(value: &Value, inherited: TextStyle, runs: &mut Vec<TextRun>) {
    match value {
        Value::String(text) => runs.extend(parse_formatted(text, inherited)),
        Value::Array(entries) => {
            for entry in entries {
                append_component(entry, inherited, runs);
            }
        }
        Value::Object(object) => {
            let mut style = inherited;
            if let Some(color) = object.get("color").and_then(Value::as_str) {
                style.color = component_color(color).unwrap_or(style.color);
            }
            if let Some(value) = object.get("bold").and_then(Value::as_bool) { style.bold = value; }
            if let Some(value) = object.get("italic").and_then(Value::as_bool) { style.italic = value; }
            if let Some(value) = object.get("underlined").or_else(|| object.get("underline")).and_then(Value::as_bool) {
                style.underline = value;
            }
            if let Some(value) = object.get("strikethrough").and_then(Value::as_bool) { style.strikethrough = value; }
            if let Some(value) = object.get("obfuscated").and_then(Value::as_bool) { style.obfuscated = value; }

            if let Some(text) = object.get("text").and_then(Value::as_str) {
                runs.extend(parse_formatted(text, style));
            } else if let Some(translate) = object.get("translate").and_then(Value::as_str) {
                // Without Minecraft's language table we can only show the
                // translation key; this is an honest preview fallback.
                runs.extend(parse_formatted(translate, style));
            }
            if let Some(extra) = object.get("extra") {
                append_component(extra, style, runs);
            }
        }
        _ => {}
    }
}

fn draw_runs(ui: &mut egui::Ui, runs: &[TextRun], use_minecraftia: bool) {
    ui.horizontal_wrapped(|ui| {
        for run in runs {
            let mut rich = RichText::new(run.text.clone()).color(run.color);
            if run.bold { rich = rich.strong(); }
            if run.italic { rich = rich.italics(); }
            if run.underline { rich = rich.underline(); }
            if run.strikethrough { rich = rich.strikethrough(); }
            if use_minecraftia {
                rich = rich.font(FontId::new(10.0, FontFamily::Name("minecraftia".into())));
            }
            ui.label(rich);
        }
    });
}

fn component_color(value: &str) -> Option<Color32> {
    if let Some(hex) = value.strip_prefix('#') {
        return parse_hex(hex);
    }
    let code = match value.to_ascii_lowercase().as_str() {
        "black" => '0',
        "dark_blue" => '1',
        "dark_green" => '2',
        "dark_aqua" => '3',
        "dark_red" => '4',
        "dark_purple" => '5',
        "gold" => '6',
        "gray" => '7',
        "dark_gray" => '8',
        "blue" => '9',
        "green" => 'a',
        "aqua" => 'b',
        "red" => 'c',
        "light_purple" => 'd',
        "yellow" => 'e',
        "white" => 'f',
        _ => return None,
    };
    legacy_color(code)
}

fn legacy_color(code: char) -> Option<Color32> {
    let (r, g, b) = match code {
        '0' => (0x00, 0x00, 0x00),
        '1' => (0x00, 0x00, 0xAA),
        '2' => (0x00, 0xAA, 0x00),
        '3' => (0x00, 0xAA, 0xAA),
        '4' => (0xAA, 0x00, 0x00),
        '5' => (0xAA, 0x00, 0xAA),
        '6' => (0xFF, 0xAA, 0x00),
        '7' => (0xAA, 0xAA, 0xAA),
        '8' => (0x55, 0x55, 0x55),
        '9' => (0x55, 0x55, 0xFF),
        'a' => (0x55, 0xFF, 0x55),
        'b' => (0x55, 0xFF, 0xFF),
        'c' => (0xFF, 0x55, 0x55),
        'd' => (0xFF, 0x55, 0xFF),
        'e' => (0xFF, 0xFF, 0x55),
        'f' => (0xFF, 0xFF, 0xFF),
        _ => return None,
    };
    Some(Color32::from_rgb(r, g, b))
}

fn parse_hex(value: &str) -> Option<Color32> {
    if value.len() != 6 { return None; }
    let red = u8::from_str_radix(&value[0..2], 16).ok()?;
    let green = u8::from_str_radix(&value[2..4], 16).ok()?;
    let blue = u8::from_str_radix(&value[4..6], 16).ok()?;
    Some(Color32::from_rgb(red, green, blue))
}

fn obfuscate(value: &str) -> String {
    const GLYPHS: &[char] = &['#', '%', '+', '*', 'x', 'X', '▒', '▓', '░', '§'];
    value.chars().enumerate().map(|(index, ch)| {
        if ch.is_whitespace() { return ch; }
        let slot = (ch as usize).wrapping_mul(31).wrapping_add(index * 17) % GLYPHS.len();
        GLYPHS[slot]
    }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_legacy_colors_and_styles_and_resets() {
        let runs = parse_minecraft_text("§cDanger §lBold§r Normal", Color32::WHITE);
        assert_eq!(runs.len(), 3);
        assert_eq!(runs[0].color, Color32::from_rgb(0xFF, 0x55, 0x55));
        assert!(runs[1].bold);
        assert_eq!(runs[2].color, Color32::WHITE);
        assert!(!runs[2].bold);
    }

    #[test]
    fn parses_hex_color_and_obfuscated_text() {
        let runs = parse_minecraft_text("§x§1§2§3§4§5§6Hex §ksecret", Color32::WHITE);
        assert_eq!(runs[0].color, Color32::from_rgb(0x12, 0x34, 0x56));
        assert_ne!(runs[1].text, "secret");
    }

    #[test]
    fn inherits_component_styles_and_supports_named_colors() {
        let value = serde_json::json!({
            "text": "Rare ",
            "color": "gold",
            "bold": true,
            "extra": [{"text":"title","italic":true}]
        });
        let mut runs = Vec::new();
        append_component(&value, TextStyle::new(Color32::WHITE), &mut runs);
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].color, Color32::from_rgb(0xFF, 0xAA, 0x00));
        assert!(runs[0].bold);
        assert!(runs[1].bold && runs[1].italic);
    }
}
