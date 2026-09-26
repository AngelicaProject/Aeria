//! `SeString` bytes and macro text, compatible with Lumina 7.7.0.
//!
//! [`decode`] prints bytes the way Lumina 7.7.0's `ToMacroString()` does, and
//! [`encode`] parses macro text the way its `ReadOnlySeString.FromMacroString`
//! does with default options. Both reproduce Lumina's behavior exactly,
//! including its lossy corners, so that text Harmonia Atlas extracted and
//! bytes it encoded stay identical. Deliberate differences from Lumina are
//! listed in `docs/architecture/strings.md`.
//!
//! The formats involved:
//!
//! - A string is text bytes (UTF-8) and macro payloads:
//!   `STX, code, length, body, ETX`, where `length` is an encoded integer.
//! - A macro body is a sequence of expressions: integers, strings
//!   (`0xFF, length, bytes`), nullary placeholders, unary parameter
//!   expressions, and binary comparisons.

use std::fmt::Write as _;

use crate::KnownMacro;

/// The macro-text and byte dialect this codec follows, as recorded where
/// strings were encoded (the pack manifest's `exporter.atlas`).
pub const STRING_DIALECT: &str = "lumina-7.7.0";

const STX: u8 = 0x02;
const ETX: u8 = 0x03;
const STRING_EXPRESSION: u8 = 0xFF;

/// Nullary expression names, by expression type.
fn nullary_name(kind: u8) -> Option<&'static str> {
    Some(match kind {
        0xD8 => "t_msec",
        0xD9 => "t_sec",
        0xDA => "t_min",
        0xDB => "t_hour",
        0xDC => "t_day",
        0xDD => "t_wday",
        0xDE => "t_mon",
        0xDF => "t_year",
        0xEC => "stackcolor",
        _ => return None,
    })
}

/// Unary expression names, by expression type.
fn unary_name(kind: u8) -> Option<&'static str> {
    Some(match kind {
        0xE8 => "lnum",
        0xE9 => "gnum",
        0xEA => "lstr",
        0xEB => "gstr",
        _ => return None,
    })
}

// ---------------------------------------------------------------------------
// Integers

/// Reads an encoded unsigned integer and its length.
fn decode_uint(bytes: &[u8]) -> Option<(u32, usize)> {
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

/// Reads an encoded integer as Lumina's `TryDecodeInt` does: the unsigned
/// value reinterpreted as signed.
fn decode_int(bytes: &[u8]) -> Option<(i32, usize)> {
    decode_uint(bytes).map(|(value, length)| (value.cast_signed(), length))
}

/// Appends an encoded unsigned integer.
fn encode_uint(out: &mut Vec<u8>, value: u32) {
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

// ---------------------------------------------------------------------------
// Expression lengths

/// A string expression's content and total length.
fn decode_string_expression(bytes: &[u8]) -> Option<(&[u8], usize)> {
    if bytes.len() < 2 || bytes[0] != STRING_EXPRESSION {
        return None;
    }
    let (length, length_bytes) = decode_int(&bytes[1..])?;
    let length = usize::try_from(length).ok()?;
    let total = 1 + length_bytes + length;
    (total <= bytes.len()).then(|| (&bytes[1 + length_bytes..total], total))
}

fn is_nullary(kind: u8) -> bool {
    matches!(kind, 0xD0..=0xDF | 0xEC)
}

/// A unary expression's type, operand, and total length.
fn decode_unary(bytes: &[u8]) -> Option<(u8, &[u8], usize)> {
    let kind = *bytes.first()?;
    if !(0xE8..=0xEB).contains(&kind) {
        return None;
    }
    let rest = &bytes[1..];
    let length = expression_length(rest)?;
    Some((kind, &rest[..length], length + 1))
}

/// A binary expression's type, operands, and total length.
fn decode_binary(bytes: &[u8]) -> Option<(u8, &[u8], &[u8], usize)> {
    let kind = *bytes.first()?;
    if !(0xE0..=0xE5).contains(&kind) {
        return None;
    }
    let rest = &bytes[1..];
    let first = expression_length(rest)?;
    let second = expression_length(&rest[first..])?;
    Some((
        kind,
        &rest[..first],
        &rest[first..first + second],
        1 + first + second,
    ))
}

/// The length of the expression at the start of `bytes`, as Lumina's
/// `TryDecodeLength`.
fn expression_length(bytes: &[u8]) -> Option<usize> {
    if bytes.is_empty() {
        return None;
    }
    if let Some((_, length)) = decode_uint(bytes) {
        return Some(length);
    }
    if let Some((_, length)) = decode_string_expression(bytes) {
        return Some(length);
    }
    if is_nullary(bytes[0]) {
        return Some(1);
    }
    if let Some((_, _, length)) = decode_unary(bytes) {
        return Some(length);
    }
    decode_binary(bytes).map(|(_, _, _, length)| length)
}

// ---------------------------------------------------------------------------
// Decoding

/// Prints `SeString` bytes as macro text, exactly as Lumina 7.7.0's
/// `ReadOnlySeString.ToMacroString()`.
///
/// Every input has a printed form. Invalid UTF-8 prints as U+FFFD, payloads
/// that are not well formed print as `<payload: XX …>`, and expressions that
/// cannot be read print as `<expr: XX …>`. Such text does not encode back to
/// the same bytes.
#[must_use]
pub fn decode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len());
    write_string(&mut out, bytes, false);
    out
}

fn write_string(out: &mut String, bytes: &[u8], for_expression: bool) {
    let mut index = 0;
    while index < bytes.len() {
        let rest = &bytes[index..];
        match rest[0] {
            0 => {
                write_invalid(out, &rest[..1]);
                index += 1;
            }
            STX => {
                if let Some((code, body, length)) = read_macro(rest) {
                    write_macro(out, code, body);
                    index += length;
                } else {
                    write_invalid(out, &rest[..1]);
                    index += 1;
                }
            }
            _ => {
                let end = rest
                    .iter()
                    .position(|byte| *byte == STX || *byte == 0)
                    .unwrap_or(rest.len());
                write_text(out, &rest[..end], for_expression);
                index += end;
            }
        }
    }
}

/// A well-formed macro payload at the start of `bytes`: code, body, length.
fn read_macro(bytes: &[u8]) -> Option<(u8, &[u8], usize)> {
    if bytes.len() < 4 {
        return None;
    }
    let code = bytes[1];
    let (length, length_bytes) = decode_int(&bytes[2..])?;
    let length = usize::try_from(length).ok()?;
    let end = 2 + length_bytes + length;
    if end + 1 > bytes.len() || bytes[end] != ETX {
        return None;
    }
    Some((code, &bytes[2 + length_bytes..end], end + 1))
}

fn write_text(out: &mut String, bytes: &[u8], for_expression: bool) {
    for character in String::from_utf8_lossy(bytes).chars() {
        let escape = if for_expression {
            matches!(character, '<' | '>' | '[' | ']' | '(' | ')' | ',' | '\\')
        } else {
            matches!(character, '<' | '\\')
        };
        if escape {
            out.push('\\');
        }
        out.push(character);
    }
}

fn write_invalid(out: &mut String, body: &[u8]) {
    if body.is_empty() {
        out.push_str("<payload: invalid empty>");
        return;
    }
    out.push_str("<payload:");
    for byte in body {
        let _ = write!(out, " {byte:02X}");
    }
    out.push('>');
}

fn write_macro(out: &mut String, code: u8, body: &[u8]) {
    out.push('<');
    match KnownMacro::from_code(code) {
        Some(known) => out.push_str(known.as_str()),
        None => {
            let _ = write!(out, "payload:{code:02X}");
        }
    }
    let mut index = 0;
    let mut first = true;
    while index < body.len() {
        let rest = &body[index..];
        let length = expression_length(rest).unwrap_or(1);
        out.push(if first { '(' } else { ',' });
        first = false;
        write_expression(out, &rest[..length]);
        index += length;
    }
    if !first {
        out.push(')');
    }
    out.push('>');
}

fn write_expression(out: &mut String, body: &[u8]) {
    if body.is_empty() {
        out.push_str("<expr: invalid empty>");
        return;
    }
    if let Some((value, _)) = decode_uint(body) {
        let _ = write!(out, "{value}");
        return;
    }
    if let Some((text, _)) = decode_string_expression(body) {
        write_string(out, text, true);
        return;
    }
    let kind = body[0];
    if is_nullary(kind) {
        match nullary_name(kind) {
            Some(name) => out.push_str(name),
            None => {
                let _ = write!(out, "<expr: 0x{kind:02X} is unsupported>");
            }
        }
        return;
    }
    if let Some((kind, operand, _)) = decode_unary(body) {
        out.push_str(unary_name(kind).unwrap_or_default());
        write_expression(out, operand);
        return;
    }
    if let Some((kind, left, right, _)) = decode_binary(body) {
        out.push('[');
        write_expression(out, left);
        out.push_str(match kind {
            0xE0 => ">=",
            0xE1 => ">",
            0xE2 => "<=",
            0xE3 => "<",
            0xE4 => "==",
            _ => "!=",
        });
        write_expression(out, right);
        out.push(']');
        return;
    }
    out.push_str("<expr:");
    for byte in body {
        let _ = write!(out, " {byte:02X}");
    }
    out.push('>');
}

// ---------------------------------------------------------------------------
// Encoding

/// Macro text that Lumina's parser would reject.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EncodeError {
    /// What was expected or not supported.
    pub message: String,
    /// Byte offset into the macro text.
    pub offset: usize,
}

impl std::fmt::Display for EncodeError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{} at byte {}", self.message, self.offset)
    }
}

impl std::error::Error for EncodeError {}

/// Encodes macro text as `SeString` bytes, exactly as Lumina 7.7.0's
/// `ReadOnlySeString.FromMacroString` with default options.
///
/// Encoding is not the inverse of [`decode`] for every text: numbers may be
/// typed in other forms (`0x10`, `+5`, `1_000`), and a string argument that
/// looks like a number becomes a number. Callers that need a stable result
/// check it with [`encode_checked`].
///
/// # Errors
///
/// Returns an error where Lumina's parser throws: an unknown macro name, a
/// missing delimiter, or an unsupported comparison.
pub fn encode(text: &str) -> Result<Vec<u8>, EncodeError> {
    let mut builder = Builder::new();
    let mut parser = Parser {
        text,
        builder: &mut builder,
    };
    parser.parse_string(0, false, b"")?;
    Ok(builder.finish())
}

/// Why macro text cannot be written into the game.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CheckedEncodeError {
    Empty,
    Invalid(EncodeError),
    ContainsNul,
    TooLong { length: usize },
    NotRoundTrip,
}

impl std::fmt::Display for CheckedEncodeError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => formatter.write_str("the macro text encodes to no bytes"),
            Self::Invalid(error) => error.fmt(formatter),
            Self::ContainsNul => formatter.write_str("the encoded string contains a NUL byte"),
            Self::TooLong { length } => write!(
                formatter,
                "the encoded string is {length} bytes, the limit is {MAX_ENCODED_LENGTH}"
            ),
            Self::NotRoundTrip => formatter
                .write_str("the encoded bytes do not survive a decode and encode round trip"),
        }
    }
}

impl std::error::Error for CheckedEncodeError {}

/// Longest encoded string the game accepts.
pub const MAX_ENCODED_LENGTH: usize = 65535;

/// Encodes macro text for the game with the checks of Harmonia Atlas's
/// `encode` command: the bytes are not empty, contain no NUL, fit the
/// length limit, and encode again to the same bytes after decoding.
///
/// # Errors
///
/// Returns the first check that fails.
pub fn encode_checked(text: &str) -> Result<Vec<u8>, CheckedEncodeError> {
    if text.is_empty() {
        return Err(CheckedEncodeError::Empty);
    }
    let bytes = encode(text).map_err(CheckedEncodeError::Invalid)?;
    if bytes.is_empty() {
        return Err(CheckedEncodeError::Empty);
    }
    if bytes.contains(&0) {
        return Err(CheckedEncodeError::ContainsNul);
    }
    if bytes.len() > MAX_ENCODED_LENGTH {
        return Err(CheckedEncodeError::TooLong {
            length: bytes.len(),
        });
    }
    match encode(&decode(&bytes)) {
        Ok(again) if again == bytes => Ok(bytes),
        _ => Err(CheckedEncodeError::NotRoundTrip),
    }
}

/// One open scope of the byte builder, as in Lumina's `SeStringBuilder`.
enum Frame {
    /// Text and payloads; `None` is the root string.
    String(Vec<u8>),
    /// A macro body of expressions.
    Macro { code: u8, body: Vec<u8> },
    /// A binary comparison: its type byte and operands.
    Binary { bytes: Vec<u8> },
}

struct Builder {
    frames: Vec<Frame>,
}

impl Builder {
    fn new() -> Self {
        Self {
            frames: vec![Frame::String(Vec::new())],
        }
    }

    fn finish(mut self) -> Vec<u8> {
        match self.frames.pop() {
            Some(Frame::String(bytes)) if self.frames.is_empty() => bytes,
            _ => unreachable!("the parser closes every scope it opens"),
        }
    }

    fn text(&mut self) -> &mut Vec<u8> {
        match self.frames.last_mut() {
            Some(Frame::String(bytes)) => bytes,
            _ => unreachable!("text is only parsed inside a string"),
        }
    }

    /// The expression target: a macro body or a binary expression.
    fn expressions(&mut self) -> &mut Vec<u8> {
        match self.frames.last_mut() {
            Some(Frame::Macro { body, .. }) => body,
            Some(Frame::Binary { bytes }) => bytes,
            _ => unreachable!("expressions are only parsed inside a macro"),
        }
    }

    fn begin_macro(&mut self, code: u8) {
        self.frames.push(Frame::Macro {
            code,
            body: Vec::new(),
        });
    }

    fn end_macro(&mut self) {
        let Some(Frame::Macro { code, body }) = self.frames.pop() else {
            unreachable!("no macro is open");
        };
        let text = self.text();
        text.push(STX);
        text.push(code);
        encode_uint(text, u32::try_from(body.len()).unwrap_or(u32::MAX));
        text.extend_from_slice(&body);
        text.push(ETX);
    }

    fn begin_binary(&mut self, kind: u8) {
        self.frames.push(Frame::Binary { bytes: vec![kind] });
    }

    fn change_binary(&mut self, kind: u8) {
        if let Some(Frame::Binary { bytes }) = self.frames.last_mut() {
            bytes[0] = kind;
        }
    }

    fn end_binary(&mut self) {
        let Some(Frame::Binary { bytes }) = self.frames.pop() else {
            unreachable!("no binary expression is open");
        };
        self.expressions().extend_from_slice(&bytes);
    }

    fn begin_string_expression(&mut self) {
        self.frames.push(Frame::String(Vec::new()));
    }

    fn end_string_expression(&mut self) {
        let Some(Frame::String(bytes)) = self.frames.pop() else {
            unreachable!("no string expression is open");
        };
        let target = self.expressions();
        target.push(STRING_EXPRESSION);
        encode_uint(target, u32::try_from(bytes.len()).unwrap_or(u32::MAX));
        target.extend_from_slice(&bytes);
    }

    /// Discards the open scope.
    fn abort(&mut self) {
        self.frames.pop();
    }
}

/// Macro characters that end plain text and need a backslash in it.
const fn requires_escape(character: char) -> bool {
    matches!(character, '\\' | ',' | '<' | '>' | '(' | ')' | '[' | ']')
}

fn is_terminator(character: char, terminators: &[u8]) -> bool {
    u8::try_from(u32::from(character)).is_ok_and(|byte| terminators.contains(&byte))
}

struct Parser<'a> {
    text: &'a str,
    builder: &'a mut Builder,
}

impl Parser<'_> {
    fn error<T>(message: impl Into<String>, offset: usize) -> Result<T, EncodeError> {
        Err(EncodeError {
            message: message.into(),
            offset,
        })
    }

    fn char_at(&self, offset: usize) -> Option<char> {
        self.text[offset..].chars().next()
    }

    /// Parses text and macros from `offset`; returns the bytes consumed.
    fn parse_string(
        &mut self,
        start: usize,
        stop_on_escaped: bool,
        terminators: &[u8],
    ) -> Result<usize, EncodeError> {
        let mut offset = start;
        while let Some(character) = self.char_at(offset) {
            match character {
                '\\' => offset += self.parse_text(offset, terminators),
                _ if is_terminator(character, terminators) => break,
                '<' => offset += self.parse_macro(offset, terminators)?,
                _ if requires_escape(character) => {
                    if stop_on_escaped {
                        break;
                    }
                    #[allow(clippy::cast_possible_truncation)]
                    self.builder.text().push(character as u8);
                    offset += 1;
                }
                _ => offset += self.parse_text(offset, terminators),
            }
        }
        Ok(offset - start)
    }

    /// Parses plain text with backslash escapes; returns the bytes consumed.
    fn parse_text(&mut self, start: usize, terminators: &[u8]) -> usize {
        let mut escaped = false;
        let mut buffer = [0; 4];
        for (position, character) in self.text[start..].char_indices() {
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
                continue;
            } else if requires_escape(character) || is_terminator(character, terminators) {
                return position;
            }
            self.builder
                .text()
                .extend_from_slice(character.encode_utf8(&mut buffer).as_bytes());
        }
        self.text.len() - start
    }

    fn parse_macro(&mut self, start: usize, terminators: &[u8]) -> Result<usize, EncodeError> {
        let mut offset = start;
        self.consume(&mut offset, "<", "\"<\"")?;
        self.skip_whitespace(&mut offset);
        let code = self.parse_macro_code(&mut offset)?;
        self.skip_whitespace(&mut offset);
        self.builder.begin_macro(code);
        let result = self.parse_macro_rest(&mut offset, terminators);
        if result.is_err() {
            self.builder.abort();
        }
        result?;
        self.builder.end_macro();
        Ok(offset - start)
    }

    fn parse_macro_rest(
        &mut self,
        offset: &mut usize,
        terminators: &[u8],
    ) -> Result<(), EncodeError> {
        if self.try_consume(offset, "(") {
            let mut first = true;
            loop {
                if *offset >= self.text.len() {
                    return Self::error("Unexpected EOF", *offset);
                }
                if self.try_consume(offset, ")") {
                    break;
                }
                if first {
                    first = false;
                } else {
                    self.consume(offset, ",", "\",\" or \")\"")?;
                }
                *offset += self.parse_expression(*offset, terminators)?;
            }
            self.skip_whitespace(offset);
        }
        self.consume(offset, ">", "\">\"")
    }

    fn parse_expression(&mut self, start: usize, terminators: &[u8]) -> Result<usize, EncodeError> {
        let mut offset = start;
        if self.try_consume(&mut offset, "[") {
            self.builder.begin_binary(0xE4);
            let result = self.parse_binary_rest(&mut offset, terminators);
            if result.is_err() {
                self.builder.abort();
            }
            result?;
            self.builder.end_binary();
            return Ok(offset - start);
        }
        self.builder.begin_string_expression();
        let length = match self.parse_string(offset, true, terminators) {
            Ok(length) => length,
            Err(error) => {
                self.builder.abort();
                return Err(error);
            }
        };
        let text = &self.text.as_bytes()[offset..offset + length];
        self.end_string_expression(text);
        Ok(length)
    }

    fn parse_binary_rest(
        &mut self,
        offset: &mut usize,
        terminators: &[u8],
    ) -> Result<(), EncodeError> {
        *offset += self.parse_expression(*offset, b"=!<>")?;
        let mut operators = self.text[*offset..]
            .chars()
            .take_while(|character| matches!(character, '!' | '=' | '<' | '>'))
            .take(2);
        let first = operators.next();
        let second = operators.next();
        let (kind, length) = match (first, second) {
            (Some('='), Some('=')) => (0xE4, 2),
            (Some('!'), Some('=')) => (0xE5, 2),
            (Some('<'), Some('=')) => (0xE2, 2),
            (Some('<'), _) => (0xE3, 1),
            (Some('>'), Some('=')) => (0xE0, 2),
            (Some('>'), _) => (0xE1, 1),
            _ => {
                let spelled: String = first.into_iter().chain(second).collect();
                return Self::error(format!("Unsupported binary expression: {spelled}"), *offset);
            }
        };
        *offset += length;
        self.builder.change_binary(kind);
        *offset += self.parse_expression(*offset, terminators)?;
        self.consume(offset, "]", "\"]\"")
    }

    /// Ends a string expression, replacing it with a number, parameter, or
    /// placeholder when its source text spells one.
    fn end_string_expression(&mut self, text: &[u8]) {
        let parameter = |prefix: &[u8], kind: u8| {
            text.strip_prefix(prefix)
                .and_then(parse_int)
                .map(|value| (kind, value))
        };
        let replacement = parameter(b"lnum", 0xE8)
            .or_else(|| parameter(b"lstr", 0xEA))
            .or_else(|| parameter(b"gnum", 0xE9))
            .or_else(|| parameter(b"gstr", 0xEB));
        if let Some((kind, value)) = replacement {
            self.builder.abort();
            let target = self.builder.expressions();
            target.push(kind);
            encode_uint(target, value.cast_unsigned());
            return;
        }
        let nullary = match text {
            b"t_msec" => Some(0xD8),
            b"t_sec" => Some(0xD9),
            b"t_min" => Some(0xDA),
            b"t_hour" => Some(0xDB),
            b"t_day" => Some(0xDC),
            b"t_wday" => Some(0xDD),
            b"t_mon" => Some(0xDE),
            b"t_year" => Some(0xDF),
            b"stackcolor" => Some(0xEC),
            _ => None,
        };
        if let Some(kind) = nullary {
            self.builder.abort();
            self.builder.expressions().push(kind);
            return;
        }
        if let Some(value) = parse_int(text) {
            self.builder.abort();
            encode_uint(self.builder.expressions(), value.cast_unsigned());
            return;
        }
        self.builder.end_string_expression();
    }

    fn parse_macro_code(&mut self, offset: &mut usize) -> Result<u8, EncodeError> {
        let mut name = String::new();
        let mut consumed = self.text.len() - *offset;
        let mut escaped = false;
        for (position, character) in self.text[*offset..].char_indices() {
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
                continue;
            } else if requires_escape(character) || character.is_whitespace() {
                consumed = position;
                break;
            }
            if name.encode_utf16().count() + character.len_utf16() > 64 {
                return Self::error(format!("Unsupported MacroCode: \"{name}...\""), *offset);
            }
            name.push(character);
        }
        match KnownMacro::from_name(&name) {
            Some(known) => {
                *offset += consumed;
                Ok(known.code())
            }
            None => Self::error(format!("Unsupported MacroCode: \"{name}\""), *offset),
        }
    }

    fn skip_whitespace(&self, offset: &mut usize) {
        let rest = &self.text[*offset..];
        *offset += rest.len() - rest.trim_start_matches(char::is_whitespace).len();
    }

    fn try_consume(&self, offset: &mut usize, expected: &str) -> bool {
        if self.text[*offset..].starts_with(expected) {
            *offset += expected.len();
            true
        } else {
            false
        }
    }

    fn consume(
        &self,
        offset: &mut usize,
        expected: &str,
        spelled: &str,
    ) -> Result<(), EncodeError> {
        if self.try_consume(offset, expected) {
            Ok(())
        } else {
            Self::error(format!("Expected {spelled}"), *offset)
        }
    }
}

/// Parses an integer as Lumina's macro parser: any leading `+` and `-`
/// signs, an optional `0x`, `0o`, `0b`, or `0d` prefix, and `_` or `'`
/// digit separators. Overflow wraps.
fn parse_int(text: &[u8]) -> Option<i32> {
    let mut data = text;
    if data.is_empty() {
        return None;
    }
    let mut negative = false;
    loop {
        match data.first()? {
            b'-' => negative = !negative,
            b'+' => {}
            b'0'..=b'9' => break,
            _ => return None,
        }
        data = &data[1..];
        if data.is_empty() {
            break;
        }
    }
    let mut radix = 10_u32;
    if data.len() > 2 && data[0] == b'0' && !matches!(data[1], b'_' | b'\'' | b'0'..=b'9') {
        radix = match data[1] {
            b'x' | b'X' => 16,
            b'o' | b'O' => 8,
            b'b' | b'B' => 2,
            b'd' | b'D' => 10,
            _ => return None,
        };
        data = &data[2..];
    }
    if data.is_empty() {
        return None;
    }
    let mut value = 0_u32;
    for byte in data {
        if matches!(byte, b'_' | b'\'') {
            continue;
        }
        let digit = char::from(*byte).to_digit(16).filter(|_| byte.is_ascii())?;
        if digit >= radix {
            return None;
        }
        value = value.wrapping_mul(radix).wrapping_add(digit);
    }
    let value = value.cast_signed();
    Some(if negative {
        value.wrapping_neg()
    } else {
        value
    })
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
    fn macros_and_expressions_decode_like_lumina() {
        // <color(0xFFEE00)>: an integer with three significant bytes.
        assert_eq!(decode(&hex("021304F5FFEE03")), "<color(16772608)>");
        // if(gnum1 == gnum2, "A", ""): unary operands inside a comparison.
        assert_eq!(
            decode(&hex("02080BE4E902E903FF0241FF0103")),
            "<if([gnum1==gnum2],A,)>"
        );
        assert_eq!(decode(b"a<b\\c"), "a\\<b\\\\c");
        assert_eq!(decode(&[0x61, 0x00, 0x62]), "a<payload: 00>b");
        assert_eq!(decode(&[0x02, 0x10, 0x01]), "<payload: 02>\u{10}\u{1}");
        assert_eq!(decode(&hex("02FA0103")), "<payload:FA>");
        assert_eq!(decode(&hex("0220020103")), "<num(0)>");
    }

    #[test]
    fn invalid_utf8_prints_as_replacement_characters() {
        assert_eq!(decode(&[0x61, 0xFF, 0x62]), "a\u{FFFD}b");
    }

    #[test]
    fn encoding_parses_numbers_parameters_and_nested_strings() {
        let bytes = encode("<if([gnum1==gnum2],A,)>").expect("encode");
        assert_eq!(decode(&bytes), "<if([gnum1==gnum2],A,)>");
        assert_eq!(
            encode("<num(0x10)>").expect("hex"),
            encode("<num(16)>").expect("decimal")
        );
        assert_eq!(
            encode("<num(-1)>").expect("negative"),
            hex("022006FEFFFFFFFF03")
        );
        assert_eq!(decode(&encode("<br>").expect("br")), "<br>");
        // Spaces around the name and after `)` are skipped; inside an
        // argument they are text, so ` 5 ` stays a string, not a number.
        assert_eq!(
            decode(&encode("< color ( 5 ) >x").expect("spaces")),
            "<color( 5 )>x"
        );
        assert_eq!(decode(&encode("a\\<b").expect("escape")), "a\\<b");
    }

    #[test]
    fn encoding_rejects_what_lumina_rejects() {
        assert!(encode("<nope>").is_err());
        assert!(encode("<num(1").is_err());
        assert!(encode("<if([1=2],a,b)>").is_err());
        assert!(encode("<payload:FA>").is_err());
        assert_eq!(
            encode_checked("<string(5)>x"),
            Ok(encode("<string(5)>x").expect("encode"))
        );
        assert_eq!(encode_checked(""), Err(CheckedEncodeError::Empty));
    }

    #[test]
    fn integers_parse_with_signs_prefixes_and_separators() {
        assert_eq!(parse_int(b"1_000"), Some(1000));
        assert_eq!(parse_int(b"--5"), Some(5));
        assert_eq!(parse_int(b"0x1F"), Some(31));
        assert_eq!(parse_int(b"0b101"), Some(5));
        assert_eq!(parse_int(b"0x"), None);
        assert_eq!(parse_int(b"-"), None);
        assert_eq!(parse_int(b"12a"), None);
        assert_eq!(parse_int(b"4294967295"), Some(-1));
    }
}
