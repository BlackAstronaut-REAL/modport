use anyhow::{bail, Result};
use std::convert::TryInto;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Instruction {
    pub offset: usize,
    pub opcode: u8,
    pub mnemonic: &'static str,
    /// Operand bytes, excluding the opcode byte.
    pub operands: Vec<u8>,
}

fn mnemonic(op: u8) -> &'static str {
    match op {
        0x00 => "nop", 0x01 => "aconst_null", 0x02..=0x0f => "constant",
        0x10 => "bipush", 0x11 => "sipush", 0x12 => "ldc", 0x13 => "ldc_w", 0x14 => "ldc2_w",
        0x15..=0x19 => "load", 0x1a..=0x35 => "load_or_array_load",
        0x36..=0x56 => "store_or_array_store", 0x57 => "pop", 0x58 => "pop2",
        0x59..=0x5f => "dup", 0x60..=0x83 => "arithmetic_or_bitwise", 0x84 => "iinc",
        0x85..=0x98 => "convert_or_compare", 0x99..=0xa8 => "branch", 0xa9 => "ret",
        0xaa => "tableswitch", 0xab => "lookupswitch", 0xac..=0xb1 => "return",
        0xb2..=0xb5 => "field", 0xb6 => "invokevirtual", 0xb7 => "invokespecial",
        0xb8 => "invokestatic", 0xb9 => "invokeinterface", 0xba => "invokedynamic",
        0xbb => "new", 0xbc => "newarray", 0xbd => "anewarray", 0xbe => "arraylength",
        0xbf => "athrow", 0xc0 => "checkcast", 0xc1 => "instanceof",
        0xc2 => "monitorenter", 0xc3 => "monitorexit", 0xc4 => "wide",
        0xc5 => "multianewarray", 0xc6 => "ifnull", 0xc7 => "ifnonnull",
        0xc8 => "goto_w", 0xc9 => "jsr_w", 0xfe => "impdep1", 0xff => "impdep2",
        _ => "opcode",
    }
}

fn fixed_operands(op: u8) -> usize {
    match op {
        0x10 | 0x12 | 0x15..=0x19 | 0x36..=0x3a | 0xa9 | 0xbc => 1,
        0x11 | 0x13 | 0x14 | 0x84 | 0x99..=0xa8 | 0xb2..=0xb8 |
        0xbb | 0xbd | 0xc0 | 0xc1 | 0xc6 | 0xc7 => 2,
        0xb9 | 0xba | 0xc8 | 0xc9 => 4,
        0xc5 => 3,
        _ => 0,
    }
}

/// Decode JVM instruction boundaries, including variable-width switches and WIDE.
pub fn decode_instructions(code: &[u8]) -> Result<Vec<Instruction>> {
    let mut result = Vec::new();
    let mut offset = 0usize;
    while offset < code.len() {
        let opcode = code[offset];
        if (0xcb..=0xfd).contains(&opcode) {
            bail!("Reserved JVM opcode 0x{opcode:02x} at byte {offset}");
        }
        let start = offset + 1;
        let end = match opcode {
            0xaa => {
                let mut p = start;
                while p % 4 != 0 { p += 1; }
                if p + 12 > code.len() { bail!("Truncated tableswitch at byte {offset}"); }
                let low = i32::from_be_bytes(code[p + 4..p + 8].try_into()?);
                let high = i32::from_be_bytes(code[p + 8..p + 12].try_into()?);
                if high < low { bail!("Invalid tableswitch range at byte {offset}"); }
                let count = (high as i64 - low as i64 + 1) as usize;
                p.checked_add(12).and_then(|n| n.checked_add(count.checked_mul(4)?))
                    .ok_or_else(|| anyhow::anyhow!("tableswitch size overflow"))?
            }
            0xab => {
                let mut p = start;
                while p % 4 != 0 { p += 1; }
                if p + 8 > code.len() { bail!("Truncated lookupswitch at byte {offset}"); }
                let pairs = i32::from_be_bytes(code[p + 4..p + 8].try_into()?);
                if pairs < 0 { bail!("Negative lookupswitch pair count at byte {offset}"); }
                p.checked_add(8).and_then(|n| n.checked_add((pairs as usize).checked_mul(8)?))
                    .ok_or_else(|| anyhow::anyhow!("lookupswitch size overflow"))?
            }
            0xc4 => {
                if start >= code.len() { bail!("Truncated wide instruction at byte {offset}"); }
                match code[start] {
                    0x84 => offset + 6,
                    0x15..=0x19 | 0x36..=0x3a | 0xa9 => offset + 4,
                    value => bail!("Invalid wide opcode 0x{value:02x} at byte {offset}"),
                }
            }
            _ => start + fixed_operands(opcode),
        };
        if end > code.len() { bail!("Truncated {} instruction at byte {offset}", mnemonic(opcode)); }
        result.push(Instruction { offset, opcode, mnemonic: mnemonic(opcode), operands: code[start..end].to_vec() });
        offset = end;
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_fixed_width_and_wide_instructions() {
        let code = [0x10, 0x05, 0x84, 0x01, 0x02, 0xc4, 0x15, 0x00, 0x02, 0xb1];
        let decoded = decode_instructions(&code).unwrap();
        assert_eq!(decoded.iter().map(|i| i.offset).collect::<Vec<_>>(), vec![0, 2, 5, 9]);
        assert_eq!(decoded[2].mnemonic, "wide");
    }

    #[test]
    fn decodes_tableswitch_boundaries() {
        let mut code = vec![0xaa, 0, 0, 0];
        code.extend(0i32.to_be_bytes());
        code.extend(0i32.to_be_bytes());
        code.extend(0i32.to_be_bytes());
        code.extend(0i32.to_be_bytes());
        code.push(0xb1);
        let decoded = decode_instructions(&code).unwrap();
        assert_eq!(decoded.len(), 2);
        assert_eq!(decoded[1].offset, 20);
    }

    #[test]
    fn rejects_truncated_instructions() {
        assert!(decode_instructions(&[0xb7, 0x00]).is_err());
    }
}
