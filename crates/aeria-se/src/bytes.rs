//! `SeString` bytes and the string model.
//!
//! [`decode`] reads any bytes into [`Node`]s and [`encode`] writes them back.
//! The conversion is lossless: `encode(&decode(bytes)) == bytes` for every
//! input. Bytes that are not a canonical text run, payload, or expression
//! are kept as [`Node::Raw`] or [`Expr::Raw`].
//!
//! The byte format:
//!
//! - A string is UTF-8 text and macro payloads:
//!   `STX, code, length, body, ETX`, where `length` is an encoded integer.
//! - A macro body is a sequence of expressions: integers, strings
//!   (`0xFF, length, bytes`), nullary values, parameters (a type byte and an
//!   operand), and comparisons (a type byte and two operands).

pub(crate) const STX: u8 = 0x02;
pub(crate) const ETX: u8 = 0x03;
const STRING_EXPRESSION: u8 = 0xFF;

/// One piece of a string.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub enum Node {
    /// Text.
    Text(String),
    /// A macro payload.
    Macro(Macro),
    /// Bytes kept exactly because they are not canonical text or a
    /// canonical payload.
    Raw(Vec<u8>),
}

/// A macro payload: its code and its argument expressions.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct Macro {
    pub code: u8,
    pub args: Vec<Expr>,
}

/// One macro argument.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub enum Expr {
    Int(u32),
    Str(Vec<Node>),
    /// A named nullary value such as the hour of the set time.
    Nullary(u8),
    /// A parameter: its type byte and operand.
    Param(u8, Box<Expr>),
    /// A comparison: its type byte and operands.
    Compare(u8, Box<Expr>, Box<Expr>),
    /// Expression bytes kept exactly.
    Raw(Vec<u8>),
}

// ---------------------------------------------------------------------------
// Integers

/// Reads an encoded unsigned integer and its length.
pub(crate) fn decode_uint(bytes: &[u8]) -> Option<(u32, usize)> {
    let first = *bytes.first()?;
    match first {
        0x01..=0xCF => Some((u32::from(first) - 1, 1)),
        0xF0..=0xFE if bytes.len() >= 2 => {
            let flags = (first.wrapping_add(1)) & 0x0F;
            let mut value = 0_u32;
            let mut length = 1;
            for (flag, shift) in [(8, 24), (4, 16), (2, 8), (1, 0)] {
                if flags & flag != 0 {
                    let byte = *bytes.get(length)?;
                    if byte == 0 {
                        return None;
                    }
                    value |= u32::from(byte) << shift;
                    length += 1;
                }
            }
            Some((value, length))
        }
        _ => None,
    }
}

/// Appends an encoded unsigned integer in its canonical form.
pub(crate) fn encode_uint(out: &mut Vec<u8>, value: u32) {
    if value < 0xCF {
        #[allow(clippy::cast_possible_truncation)]
        out.push(value as u8 + 1);
        return;
    }
    let marker = out.len();
    out.push(0xF0);
    for (flag, shift) in [(8_u8, 24), (4, 16), (2, 8), (1, 0)] {
        #[allow(clippy::cast_possible_truncation)]
        let byte = (value >> shift) as u8;
        if byte != 0 {
            out.push(byte);
            out[marker] |= flag;
        }
    }
    out[marker] -= 1;
}

fn is_nullary(kind: u8) -> bool {
    matches!(kind, 0xD0..=0xDF | 0xEC)
}

fn is_param(kind: u8) -> bool {
    (0xE8..=0xEB).contains(&kind)
}

fn is_compare(kind: u8) -> bool {
    (0xE0..=0xE5).contains(&kind)
}

/// The length of the expression at the start of `bytes`.
fn expression_length(bytes: &[u8]) -> Option<usize> {
    let kind = *bytes.first()?;
    if let Some((_, length)) = decode_uint(bytes) {
        return Some(length);
    }
    if kind == STRING_EXPRESSION {
        let (length, length_bytes) = decode_uint(&bytes[1..])?;
        let total = 1 + length_bytes + usize::try_from(length).ok()?;
        return (total <= bytes.len()).then_some(total);
    }
    if is_nullary(kind) {
        return Some(1);
    }
    if is_param(kind) {
        return Some(1 + expression_length(&bytes[1..])?);
    }
    if is_compare(kind) {
        let first = expression_length(&bytes[1..])?;
        let second = expression_length(&bytes[1 + first..])?;
        return Some(1 + first + second);
    }
    None
}

// ---------------------------------------------------------------------------
// Decoding

/// Reads bytes into nodes. Every input decodes, and [`encode`] writes the
/// same bytes back.
#[must_use]
pub fn decode(bytes: &[u8]) -> Vec<Node> {
    let mut nodes = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        let rest = &bytes[index..];
        match rest[0] {
            STX => {
                if let Some((node, length)) = decode_payload(rest) {
                    nodes.push(node);
                    index += length;
                } else {
                    push_raw(&mut nodes, &rest[..1]);
                    index += 1;
                }
            }
            0 => {
                push_raw(&mut nodes, &rest[..1]);
                index += 1;
            }
            _ => {
                let end = rest
                    .iter()
                    .position(|byte| matches!(*byte, STX | 0))
                    .unwrap_or(rest.len());
                decode_text(&mut nodes, &rest[..end]);
                index += end;
            }
        }
    }
    nodes
}

fn push_raw(nodes: &mut Vec<Node>, bytes: &[u8]) {
    if let Some(Node::Raw(previous)) = nodes.last_mut() {
        previous.extend_from_slice(bytes);
    } else {
        nodes.push(Node::Raw(bytes.to_vec()));
    }
}

fn push_text(nodes: &mut Vec<Node>, text: &str) {
    if let Some(Node::Text(previous)) = nodes.last_mut() {
        previous.push_str(text);
    } else {
        nodes.push(Node::Text(text.to_owned()));
    }
}

fn decode_text(nodes: &mut Vec<Node>, mut bytes: &[u8]) {
    while !bytes.is_empty() {
        match std::str::from_utf8(bytes) {
            Ok(text) => {
                push_text(nodes, text);
                return;
            }
            Err(error) => {
                let valid = error.valid_up_to();
                if valid > 0 {
                    push_text(
                        nodes,
                        std::str::from_utf8(&bytes[..valid]).unwrap_or_default(),
                    );
                }
                let invalid = error.error_len().unwrap_or(bytes.len() - valid);
                push_raw(nodes, &bytes[valid..valid + invalid]);
                bytes = &bytes[valid + invalid..];
            }
        }
    }
}

/// A payload at the start of `bytes`, as a macro when it is canonical and
/// as raw bytes when it is framed but not canonical.
fn decode_payload(bytes: &[u8]) -> Option<(Node, usize)> {
    if bytes.len() < 4 {
        return None;
    }
    let code = bytes[1];
    let (length, length_bytes) = decode_uint(&bytes[2..])?;
    let start = 2 + length_bytes;
    let end = start + usize::try_from(length).ok()?;
    if end >= bytes.len() || bytes[end] != ETX {
        return None;
    }
    let payload = &bytes[..=end];
    let node = decode_args(&bytes[start..end])
        .map(|args| Macro { code, args })
        .filter(|decoded| encode_macro_bytes(decoded) == payload)
        .map_or_else(|| Node::Raw(payload.to_vec()), Node::Macro);
    Some((node, end + 1))
}

fn decode_args(mut body: &[u8]) -> Option<Vec<Expr>> {
    let mut args = Vec::new();
    while !body.is_empty() {
        let length = expression_length(body)?;
        args.push(decode_expr(&body[..length]));
        body = &body[length..];
    }
    Some(args)
}

/// Decodes one expression of known length, keeping it raw unless it
/// encodes back to the same bytes.
fn decode_expr(bytes: &[u8]) -> Expr {
    let decoded = decode_expr_shape(bytes);
    match decoded {
        Some(expr) if encode_expr_bytes(&expr) == bytes => expr,
        _ => Expr::Raw(bytes.to_vec()),
    }
}

fn decode_expr_shape(bytes: &[u8]) -> Option<Expr> {
    let kind = *bytes.first()?;
    if let Some((value, _)) = decode_uint(bytes) {
        return Some(Expr::Int(value));
    }
    if kind == STRING_EXPRESSION {
        let (_, length_bytes) = decode_uint(&bytes[1..])?;
        return Some(Expr::Str(decode(&bytes[1 + length_bytes..])));
    }
    if is_nullary(kind) {
        return crate::catalog::NULLARY
            .iter()
            .any(|spec| spec.code == kind)
            .then_some(Expr::Nullary(kind));
    }
    if is_param(kind) {
        return Some(Expr::Param(kind, Box::new(decode_expr(&bytes[1..]))));
    }
    if is_compare(kind) {
        let first = expression_length(&bytes[1..])?;
        return Some(Expr::Compare(
            kind,
            Box::new(decode_expr(&bytes[1..=first])),
            Box::new(decode_expr(&bytes[1 + first..])),
        ));
    }
    None
}

// ---------------------------------------------------------------------------
// Encoding

/// Writes nodes as bytes.
#[must_use]
pub fn encode(nodes: &[Node]) -> Vec<u8> {
    let mut out = Vec::new();
    encode_into(&mut out, nodes);
    out
}

fn encode_into(out: &mut Vec<u8>, nodes: &[Node]) {
    for node in nodes {
        match node {
            Node::Text(text) => out.extend_from_slice(text.as_bytes()),
            Node::Macro(macro_node) => out.extend_from_slice(&encode_macro_bytes(macro_node)),
            Node::Raw(bytes) => out.extend_from_slice(bytes),
        }
    }
}

fn encode_macro_bytes(macro_node: &Macro) -> Vec<u8> {
    let mut body = Vec::new();
    for arg in &macro_node.args {
        body.extend_from_slice(&encode_expr_bytes(arg));
    }
    let mut out = vec![STX, macro_node.code];
    encode_uint(&mut out, u32::try_from(body.len()).unwrap_or(u32::MAX));
    out.extend_from_slice(&body);
    out.push(ETX);
    out
}

fn encode_expr_bytes(expr: &Expr) -> Vec<u8> {
    let mut out = Vec::new();
    match expr {
        Expr::Int(value) => encode_uint(&mut out, *value),
        Expr::Str(nodes) => {
            let text = encode(nodes);
            out.push(STRING_EXPRESSION);
            encode_uint(&mut out, u32::try_from(text.len()).unwrap_or(u32::MAX));
            out.extend_from_slice(&text);
        }
        Expr::Nullary(kind) => out.push(*kind),
        Expr::Param(kind, operand) => {
            out.push(*kind);
            out.extend_from_slice(&encode_expr_bytes(operand));
        }
        Expr::Compare(kind, left, right) => {
            out.push(*kind);
            out.extend_from_slice(&encode_expr_bytes(left));
            out.extend_from_slice(&encode_expr_bytes(right));
        }
        Expr::Raw(bytes) => out.extend_from_slice(bytes),
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(text: &str) -> Vec<u8> {
        (0..text.len())
            .step_by(2)
            .map(|index| u8::from_str_radix(&text[index..index + 2], 16).expect("hex"))
            .collect()
    }

    #[test]
    fn integers_use_the_single_byte_and_flagged_forms() {
        for value in [0, 1, 0xCE, 0xCF, 0x100, 0xFF00, 0x0102_0304, u32::MAX] {
            let mut bytes = Vec::new();
            encode_uint(&mut bytes, value);
            assert_eq!(
                decode_uint(&bytes),
                Some((value, bytes.len())),
                "{value:#x}"
            );
        }
        let mut bytes = Vec::new();
        encode_uint(&mut bytes, 0x100);
        assert_eq!(bytes, [0xF1, 0x01]);
        assert_eq!(decode_uint(&[0xF1, 0x00]), None);
        assert_eq!(decode_uint(&[0xF1]), None);
    }

    #[test]
    fn payloads_decode_into_macros_with_expressions() {
        // <if ($n1 == 1)>a<else>b</if>
        let bytes = hex("02080BE4E80202FF0261FF026203");
        assert_eq!(
            decode(&bytes),
            [Node::Macro(Macro {
                code: 0x08,
                args: vec![
                    Expr::Compare(
                        0xE4,
                        Box::new(Expr::Param(0xE8, Box::new(Expr::Int(1)))),
                        Box::new(Expr::Int(1)),
                    ),
                    Expr::Str(vec![Node::Text("a".to_owned())]),
                    Expr::Str(vec![Node::Text("b".to_owned())]),
                ],
            })]
        );
        assert_eq!(encode(&decode(&bytes)), bytes);
    }

    #[test]
    fn anything_that_is_not_canonical_stays_raw_and_round_trips() {
        for bytes in [
            hex("41FF42"),       // invalid UTF-8
            hex("4100"),         // NUL
            hex("0210"),         // truncated payload
            hex("0210020003"),   // a payload whose body is not expressions
            hex("021303F00103"), // a non-canonical integer
            hex("021302D003"),   // an unnamed nullary expression
            hex("02FE0103"),     // an unknown code with an empty body
            hex("0202"),         // STX STX
        ] {
            assert_eq!(encode(&decode(&bytes)), bytes, "{bytes:02X?}");
        }
        assert_eq!(
            decode(&hex("0210020003")),
            [Node::Raw(hex("0210020003"))],
            "framed payloads stay whole"
        );
        assert_eq!(
            decode(&hex("021302D003")),
            [Node::Macro(Macro {
                code: 0x13,
                args: vec![Expr::Raw(vec![0xD0])],
            })]
        );
    }
}
