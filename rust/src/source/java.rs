use anyhow::{bail, Result};
use regex::Regex;
use std::collections::BTreeMap;

#[derive(Debug, thiserror::Error)]
pub enum JavaParseError {
    #[error("Expected '{opener}' at character {index}")]
    ExpectedOpener { opener: char, index: usize },
    #[error("Unclosed '{opener}' beginning at character {index}")]
    UnclosedDelimiter { opener: char, index: usize },
    #[error("Unsupported numeric expression: {0}")]
    UnsupportedNumber(String),
    #[error("Non-finite numeric result: {0}")]
    NonFinite(String),
}

/// Remove Java comments without modifying quoted strings or line positions.
pub fn strip_java_comments(text: &str) -> String {
    #[derive(Clone, Copy, PartialEq)]
    enum State { Code, String(char), LineComment, BlockComment }

    let mut output = String::with_capacity(text.len());
    let mut chars = text.char_indices().peekable();
    let mut state = State::Code;

    while let Some((_, ch)) = chars.next() {
        let next = chars.peek().map(|(_, c)| *c);
        match state {
            State::Code => {
                if ch == '"' || ch == '\'' {
                    state = State::String(ch);
                    output.push(ch);
                } else if ch == '/' && next == Some('/') {
                    chars.next();
                    output.push_str("  ");
                    state = State::LineComment;
                } else if ch == '/' && next == Some('*') {
                    chars.next();
                    output.push_str("  ");
                    state = State::BlockComment;
                } else {
                    output.push(ch);
                }
            }
            State::String(quote) => {
                output.push(ch);
                if ch == '\\' {
                    if let Some((_, escaped)) = chars.next() {
                        output.push(escaped);
                    }
                } else if ch == quote {
                    state = State::Code;
                }
            }
            State::LineComment => {
                if ch == '\n' {
                    output.push('\n');
                    state = State::Code;
                } else {
                    output.push(' ');
                }
            }
            State::BlockComment => {
                if ch == '*' && next == Some('/') {
                    chars.next();
                    output.push_str("  ");
                    state = State::Code;
                } else if ch == '\n' {
                    output.push('\n');
                } else {
                    output.push(' ');
                }
            }
        }
    }
    output
}

/// Return text inside a balanced matching delimiter and the byte index after it.
pub fn extract_balanced(text: &str, open_index: usize, opener: char, closer: char) -> Result<(String, usize)> {
    let Some(open_char) = text.get(open_index..).and_then(|s| s.chars().next()) else {
        return Err(JavaParseError::ExpectedOpener { opener, index: open_index }.into());
    };
    if open_char != opener {
        return Err(JavaParseError::ExpectedOpener { opener, index: open_index }.into());
    }

    let mut depth = 0usize;
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for (relative, ch) in text[open_index..].char_indices() {
        let absolute = open_index + relative;
        if let Some(q) = quote {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == q {
                quote = None;
            }
            continue;
        }
        if ch == '"' || ch == '\'' {
            quote = Some(ch);
        } else if ch == opener {
            depth += 1;
        } else if ch == closer {
            depth -= 1;
            if depth == 0 {
                return Ok((text[open_index + opener.len_utf8()..absolute].to_owned(), absolute + closer.len_utf8()));
            }
        }
    }
    Err(JavaParseError::UnclosedDelimiter { opener, index: open_index }.into())
}

/// Split on a delimiter only when outside nested (), [], {}, and generic <>.
pub fn split_top_level(text: &str, delimiter: char) -> Vec<String> {
    let mut result = Vec::new();
    let mut stack: Vec<char> = Vec::new();
    let mut quote: Option<char> = None;
    let mut escaped = false;
    let mut start = 0usize;
    let chars: Vec<(usize, char)> = text.char_indices().collect();

    for (i, (byte_index, ch)) in chars.iter().copied().enumerate() {
        if let Some(q) = quote {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == q {
                quote = None;
            }
            continue;
        }

        if ch == '"' || ch == '\'' {
            quote = Some(ch);
            continue;
        }

        let next = chars.get(i + 1).map(|(_, c)| *c);
        match ch {
            '(' => stack.push(')'),
            '[' => stack.push(']'),
            '{' => stack.push('}'),
            '<' => {
                // Match the old parser's guard: only treat '<' as a generic
                // bracket when the next char begins a type argument.
                if next.map(|c| c.is_ascii_alphabetic() || c == '_' || c == '$' || c == '?').unwrap_or(false) {
                    stack.push('>');
                }
            }
            ')' | ']' | '}' | '>' if stack.last() == Some(&ch) => { stack.pop(); }
            _ => {}
        }

        if ch == delimiter && stack.is_empty() {
            result.push(text[start..byte_index].trim().to_owned());
            start = byte_index + ch.len_utf8();
        }
    }
    result.push(text[start..].trim().to_owned());
    result
}

/// Unquote Java string literal syntax; unknown escapes are retained conservatively.
pub fn unquote_java_string(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.len() >= 2 && trimmed.starts_with('"') && trimmed.ends_with('"') {
        if let Ok(decoded) = serde_json::from_str::<String>(trimmed) {
            return decoded;
        }
        return trimmed[1..trimmed.len() - 1].to_owned();
    }
    trimmed.to_owned()
}

fn normalize_java_number(expression: &str) -> String {
    let cast = Regex::new(r"\((?:float|double|int|long|short|byte)\)\s*").unwrap();
    let suffix = Regex::new(r"(?i)([0-9])([fFdDlL])\b").unwrap();
    let mut expr = suffix.replace_all(&cast.replace_all(expression.trim(), ""), "$1").to_string();
    let replacements = [
        ("StrictMath.PI", std::f64::consts::PI),
        ("Math.PI", std::f64::consts::PI),
        ("Mth.PI", std::f64::consts::PI),
        ("Mth.HALF_PI", std::f64::consts::FRAC_PI_2),
        ("Mth.TWO_PI", std::f64::consts::TAU),
        ("Mth.DEG_TO_RAD", std::f64::consts::PI / 180.0),
        ("Mth.RAD_TO_DEG", 180.0 / std::f64::consts::PI),
    ];
    for (name, value) in replacements {
        expr = expr.replace(name, &format!("{value:.17}"));
    }
    expr
}

struct NumberParser<'a> {
    input: &'a str,
    position: usize,
    constants: &'a BTreeMap<String, f64>,
}

impl<'a> NumberParser<'a> {
    fn new(input: &'a str, constants: &'a BTreeMap<String, f64>) -> Self {
        Self { input, position: 0, constants }
    }

    fn skip_ws(&mut self) {
        while self.input.get(self.position..).and_then(|s| s.chars().next()).map(char::is_whitespace).unwrap_or(false) {
            self.position += self.input[self.position..].chars().next().unwrap().len_utf8();
        }
    }

    fn consume(&mut self, token: &str) -> bool {
        self.skip_ws();
        if self.input[self.position..].starts_with(token) {
            self.position += token.len();
            true
        } else { false }
    }

    fn parse(&mut self) -> Result<f64> {
        let value = self.parse_add_sub()?;
        self.skip_ws();
        if self.position != self.input.len() {
            bail!(JavaParseError::UnsupportedNumber(self.input.to_owned()));
        }
        if !value.is_finite() {
            bail!(JavaParseError::NonFinite(self.input.to_owned()));
        }
        Ok(value)
    }

    fn parse_add_sub(&mut self) -> Result<f64> {
        let mut value = self.parse_mul_div()?;
        loop {
            if self.consume("+") {
                value += self.parse_mul_div()?;
            } else if self.consume("-") {
                value -= self.parse_mul_div()?;
            } else { break; }
        }
        Ok(value)
    }

    fn parse_mul_div(&mut self) -> Result<f64> {
        let mut value = self.parse_unary()?;
        loop {
            if self.consume("*") {
                value *= self.parse_unary()?;
            } else if self.consume("/") {
                value /= self.parse_unary()?;
            } else if self.consume("%") {
                value %= self.parse_unary()?;
            } else { break; }
        }
        Ok(value)
    }

    fn parse_power(&mut self) -> Result<f64> {
        let value = self.parse_atom()?;
        if self.consume("**") {
            Ok(value.powf(self.parse_unary()?))
        } else { Ok(value) }
    }

    fn parse_unary(&mut self) -> Result<f64> {
        if self.consume("+") { self.parse_unary() }
        else if self.consume("-") { Ok(-self.parse_unary()?) }
        else { self.parse_power() }
    }

    fn parse_atom(&mut self) -> Result<f64> {
        self.skip_ws();
        if self.consume("(") {
            let value = self.parse_add_sub()?;
            if !self.consume(")") { bail!(JavaParseError::UnsupportedNumber(self.input.to_owned())); }
            return Ok(value);
        }

        let rest = &self.input[self.position..];
        let first = rest.chars().next().ok_or_else(|| anyhow::anyhow!(JavaParseError::UnsupportedNumber(self.input.to_owned())))?;
        if first.is_ascii_digit() || first == '.' {
            let mut end = 0usize;
            let mut seen_exp = false;
            for (idx, ch) in rest.char_indices() {
                if ch.is_ascii_digit() || ch == '.' {
                    end = idx + ch.len_utf8();
                } else if (ch == 'e' || ch == 'E') && !seen_exp {
                    seen_exp = true;
                    end = idx + 1;
                    if rest[end..].starts_with('+') || rest[end..].starts_with('-') { end += 1; }
                } else { break; }
            }
            if end == 0 { bail!(JavaParseError::UnsupportedNumber(self.input.to_owned())); }
            let number = rest[..end].parse::<f64>().map_err(|_| JavaParseError::UnsupportedNumber(self.input.to_owned()))?;
            self.position += end;
            return Ok(number);
        }

        if first.is_ascii_alphabetic() || first == '_' || first == '$' {
            let mut end = 0usize;
            for (idx, ch) in rest.char_indices() {
                if ch.is_ascii_alphanumeric() || ch == '_' || ch == '$' || ch == '.' {
                    end = idx + ch.len_utf8();
                } else { break; }
            }
            let identifier = &rest[..end];
            self.position += end;
            return self.constants.get(identifier).copied()
                .ok_or_else(|| JavaParseError::UnsupportedNumber(self.input.to_owned()).into());
        }
        bail!(JavaParseError::UnsupportedNumber(self.input.to_owned()))
    }
}

/// Evaluate the arithmetic expression subset used for model geometry and stats.
pub fn eval_number(expression: &str, constants: &BTreeMap<String, f64>) -> Result<f64> {
    let normalized = normalize_java_number(expression);
    NumberParser::new(&normalized, constants).parse()
}

/// Match Python round(value) semantics (ties to even) before converting to i32.
pub fn eval_int(expression: &str, constants: &BTreeMap<String, f64>) -> Result<i32> {
    let value = eval_number(expression, constants)?.round_ties_even();
    if value < i32::MIN as f64 || value > i32::MAX as f64 {
        bail!("Integer expression is outside i32 range: {expression}");
    }
    Ok(value as i32)
}

/// Recover primitive numeric constants over repeated passes, as in Beta 12.
pub fn find_numeric_constants(text: &str) -> BTreeMap<String, f64> {
    let pattern = Regex::new(
        r"(?:public|protected|private)?\s*(?:static\s+)?(?:final\s+)?(?:float|double|int|long|short|byte)\s+([A-Za-z_$][\w$]*)\s*=\s*([^;]+);"
    ).unwrap();
    let mut constants = BTreeMap::new();
    let mut pending: Vec<(String, String)> = pattern.captures_iter(text)
        .filter_map(|capture| Some((capture.get(1)?.as_str().to_owned(), capture.get(2)?.as_str().to_owned())))
        .collect();
    for _ in 0..4 {
        let mut remaining = Vec::new();
        let mut changed = false;
        for (name, expression) in pending {
            match eval_number(&expression, &constants) {
                Ok(value) => { constants.insert(name, value); changed = true; }
                Err(_) => remaining.push((name, expression)),
            }
        }
        pending = remaining;
        if !changed { break; }
    }
    constants
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_comments_but_keeps_strings_and_newlines() {
        let source = r#"String a = "// not comment"; // gone
/* block
comment */ int x = 1;"#;
        let cleaned = strip_java_comments(source);
        assert!(cleaned.contains(r#""// not comment""#));
        assert_eq!(source.matches('\n').count(), cleaned.matches('\n').count());
        assert!(!cleaned.contains("gone"));
    }

    #[test]
    fn extracts_nested_balanced_call() {
        let source = r#"foo(1, nested(" ) ", 3), [4, 5]) tail"#;
        let (inside, end) = extract_balanced(source, 3, '(', ')').unwrap();
        assert_eq!(inside, r#"1, nested(" ) ", 3), [4, 5]"#);
        assert_eq!(&source[end..], " tail");
    }

    #[test]
    fn splits_only_top_level_commas() {
        let values = split_top_level(r#"a, call(1, 2), new Pair<String, List<Integer>>(), "x,y""#, ',');
        assert_eq!(values.len(), 4);
        assert_eq!(values[1], "call(1, 2)");
    }

    #[test]
    fn evaluates_java_numeric_expressions_and_constants() {
        let mut constants = BTreeMap::new();
        constants.insert("WIDTH".into(), 16.0);
        assert_eq!(eval_number("(float) (WIDTH / 2 + 1.5F)", &constants).unwrap(), 9.5);
        assert!((eval_number("Mth.HALF_PI * 2", &constants).unwrap() - std::f64::consts::PI).abs() < 1e-9);
        assert_eq!(eval_int("2.5", &BTreeMap::new()).unwrap(), 2);
        assert_eq!(eval_int("3.5", &BTreeMap::new()).unwrap(), 4);
        assert_eq!(eval_number("-2 ** 2", &BTreeMap::new()).unwrap(), -4.0);
        assert_eq!(eval_number("2 ** 3 * 4", &BTreeMap::new()).unwrap(), 32.0);
    }

    #[test]
    fn resolves_forward_referenced_constants_across_passes() {
        let constants = find_numeric_constants("private static final float B = A * 2; static final int A = 5;");
        assert_eq!(constants.get("B"), Some(&10.0));
    }
}
