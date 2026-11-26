use anyhow::{bail, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JvmType {
    Byte, Char, Double, Float, Int, Long, Short, Boolean, Void,
    Object(String),
    Array(Box<JvmType>),
}

impl JvmType {
    /// JVM operand/local slot width. Long and double occupy two slots.
    pub fn slots(&self) -> usize {
        match self {
            Self::Long | Self::Double => 2,
            Self::Void => 0,
            _ => 1,
        }
    }

    pub fn is_reference(&self) -> bool {
        matches!(self, Self::Object(_) | Self::Array(_))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MethodDescriptor {
    pub parameters: Vec<JvmType>,
    pub return_type: JvmType,
}

impl MethodDescriptor {
    /// Number of JVM values on the operand stack, independent of wide-slot size.
    pub fn argument_value_count(&self) -> usize { self.parameters.len() }

    /// Number of local-variable slots occupied by parameters.
    pub fn argument_slot_count(&self) -> usize {
        self.parameters.iter().map(JvmType::slots).sum()
    }
}

struct DescriptorParser<'a> {
    input: &'a str,
    position: usize,
}

impl<'a> DescriptorParser<'a> {
    fn new(input: &'a str) -> Self { Self { input, position: 0 } }

    fn parse_type(&mut self, allow_void: bool) -> Result<JvmType> {
        let byte = *self.input.as_bytes().get(self.position)
            .ok_or_else(|| anyhow::anyhow!("Unexpected end of JVM descriptor: {}", self.input))?;
        self.position += 1;
        let kind = match byte {
            b'B' => JvmType::Byte,
            b'C' => JvmType::Char,
            b'D' => JvmType::Double,
            b'F' => JvmType::Float,
            b'I' => JvmType::Int,
            b'J' => JvmType::Long,
            b'S' => JvmType::Short,
            b'Z' => JvmType::Boolean,
            b'V' if allow_void => JvmType::Void,
            b'V' => bail!("Void is not valid in a JVM parameter or array descriptor"),
            b'L' => {
                let start = self.position;
                let end = self.input[start..].find(';')
                    .ok_or_else(|| anyhow::anyhow!("Unterminated object type in JVM descriptor: {}", self.input))?;
                if end == 0 { bail!("Empty object name in JVM descriptor: {}", self.input); }
                self.position = start + end + 1;
                JvmType::Object(self.input[start..start + end].to_owned())
            }
            b'[' => JvmType::Array(Box::new(self.parse_type(false)?)),
            other => bail!("Unknown JVM descriptor type '{}' in {}", other as char, self.input),
        };
        Ok(kind)
    }

    fn parse_method(mut self) -> Result<MethodDescriptor> {
        if !self.input.starts_with('(') { bail!("Method descriptor must begin with '(': {}", self.input); }
        self.position = 1;
        let mut parameters = Vec::new();
        while self.input.as_bytes().get(self.position) != Some(&b')') {
            if self.position >= self.input.len() { bail!("Unterminated parameter list in JVM descriptor: {}", self.input); }
            parameters.push(self.parse_type(false)?);
        }
        self.position += 1;
        let return_type = self.parse_type(true)?;
        if self.position != self.input.len() { bail!("Trailing data in JVM descriptor: {}", self.input); }
        Ok(MethodDescriptor { parameters, return_type })
    }
}

pub fn parse_field_descriptor(input: &str) -> Result<JvmType> {
    let mut parser = DescriptorParser::new(input);
    let value = parser.parse_type(false)?;
    if parser.position != input.len() { bail!("Trailing data in JVM field descriptor: {input}"); }
    Ok(value)
}

pub fn parse_method_descriptor(input: &str) -> Result<MethodDescriptor> {
    DescriptorParser::new(input).parse_method()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_mixed_method_descriptor_and_slot_widths() {
        let descriptor = parse_method_descriptor("(Ljava/lang/Object;IJ[D)Z").unwrap();
        assert_eq!(descriptor.parameters, vec![
            JvmType::Object("java/lang/Object".into()),
            JvmType::Int,
            JvmType::Long,
            JvmType::Array(Box::new(JvmType::Double)),
        ]);
        assert_eq!(descriptor.argument_value_count(), 4);
        assert_eq!(descriptor.argument_slot_count(), 5);
        assert_eq!(descriptor.return_type, JvmType::Boolean);
    }

    #[test]
    fn parses_nested_arrays_and_rejects_void_parameters() {
        assert_eq!(
            parse_field_descriptor("[[Ljava/lang/String;").unwrap(),
            JvmType::Array(Box::new(JvmType::Array(Box::new(JvmType::Object("java/lang/String".into())))))
        );
        assert!(parse_method_descriptor("(V)V").is_err());
    }
}
