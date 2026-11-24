use eframe::egui::{self, Color32, FontFamily, FontId, RichText};

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
            color, bold: false, italic: false, underline: false,
            strikethrough: false, obfuscated: false,
        }
    }
}

pub fn parse_minecraft_text(input: &str, default_color: Color32) -> Vec<TextRun> {
    let chars: Vec<char> = input.chars().collect();
    let mut style = TextStyle::new(default_color);
    let mut runs = Vec::<TextRun>::new();
    let mut buffer = String::new();
    let mut index = 0usize;

    let flush = |buffer: &mut String, style: TextStyle, runs: &mut Vec<TextRun>| {
        if buffer.is_empty() { return; }
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
    };

    while index < chars.len() {
        if chars[index] == '§' && index + 1 < chars.len() {
            let code = chars[index + 1].to_ascii_lowercase();
            flush(&mut buffer, style, &mut runs);
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
                    style = TextStyle::new(parse_hex(&hex).unwrap_or(default_color));
                    index += 14;
                    continue;
                }
            }

            match code {
                '0'..='9' | 'a'..='f' => {
                    style = TextStyle::new(legacy_color(code).unwrap_or(default_color));
                }
                'k' => style.obfuscated = true,
                'l' => style.bold = true,
                'm' => style.strikethrough = true,
                'n' => style.underline = true,
                'o' => style.italic = true,
                'r' => style = TextStyle::new(default_color),
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
    flush(&mut buffer, style, &mut runs);
    runs
}

pub fn draw_formatted_text(
    ui: &mut egui::Ui,
    text: &str,
    default_color: Color32,
    use_minecraftia: bool,
) {
    ui.horizontal_wrapped(|ui| {
        for run in parse_minecraft_text(text, default_color) {
            let mut rich = RichText::new(run.text).color(run.color);
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
        let runs = parse_minecraft_text(
            "§cDanger §lBold§r Normal",
            Color32::WHITE,
        );
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
}
