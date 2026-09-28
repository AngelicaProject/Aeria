//! A reader for the game's compiled Lua 5.1 scripts.
//!
//! Quest scripts are Lua 5.1 chunks compiled for a little-endian machine
//! with 4-byte `int`, `size_t`, and instructions and 8-byte floating-point
//! numbers. Any other chunk format is refused, never reinterpreted. The
//! reader checks every length against the data, so a malformed file is an
//! error rather than a panic.

use std::fmt;

/// The first bytes of a supported chunk: the signature, version 5.1,
/// the official format, little endian, and the sizes of `int`, `size_t`,
/// `Instruction`, and `lua_Number`, which is not integral.
const HEADER: [u8; 12] = [0x1b, b'L', b'u', b'a', 0x51, 0, 1, 4, 4, 4, 8, 0];

/// How deeply functions may nest in a chunk.
const MAX_NESTING: usize = 64;

/// Why a chunk could not be read.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChunkError(pub String);

impl std::error::Error for ChunkError {}

impl fmt::Display for ChunkError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

fn error(message: impl Into<String>) -> ChunkError {
    ChunkError(message.into())
}

/// A constant of a function.
#[derive(Clone, Debug, PartialEq)]
pub enum Constant {
    Nil,
    Boolean(bool),
    Number(f64),
    /// A string, decoded lossily; scripts name functions and fields in
    /// ASCII.
    String(String),
}

/// One compiled function.
#[derive(Clone, Debug, PartialEq)]
pub struct Function {
    /// How many parameters it takes, in its first registers.
    pub parameters: u8,
    /// How many registers it uses. The ones after its parameters start as
    /// `nil`, which the compiler relies on instead of writing `LOADNIL`.
    pub stack: u8,
    pub code: Vec<u32>,
    pub constants: Vec<Constant>,
    pub functions: Vec<Function>,
}

impl Function {
    /// The constant at `index`, if the function has one.
    #[must_use]
    pub fn constant(&self, index: u32) -> Option<&Constant> {
        self.constants.get(usize::try_from(index).ok()?)
    }

    /// The string constant at `index`.
    #[must_use]
    pub fn string(&self, index: u32) -> Option<&str> {
        match self.constant(index)? {
            Constant::String(text) => Some(text),
            _ => None,
        }
    }
}

/// Reads a chunk and returns its main function.
///
/// # Errors
///
/// Returns an error when the data is not a supported Lua 5.1 chunk or is
/// truncated or inconsistent.
pub fn read_chunk(data: &[u8]) -> Result<Function, ChunkError> {
    if data.len() < HEADER.len() || !data.starts_with(&HEADER[..4]) {
        return Err(error("not a compiled Lua script"));
    }
    if data[..HEADER.len()] != HEADER {
        return Err(error("a compiled Lua script in an unsupported format"));
    }
    let mut reader = Reader {
        data,
        position: HEADER.len(),
    };
    let function = reader.function(0)?;
    if reader.position != data.len() {
        return Err(error("data after the compiled Lua script"));
    }
    Ok(function)
}

struct Reader<'a> {
    data: &'a [u8],
    position: usize,
}

impl Reader<'_> {
    fn bytes(&mut self, count: usize) -> Result<&[u8], ChunkError> {
        let end = self
            .position
            .checked_add(count)
            .filter(|end| *end <= self.data.len())
            .ok_or_else(|| error("the compiled Lua script is truncated"))?;
        let bytes = &self.data[self.position..end];
        self.position = end;
        Ok(bytes)
    }

    fn byte(&mut self) -> Result<u8, ChunkError> {
        Ok(self.bytes(1)?[0])
    }

    fn u32(&mut self) -> Result<u32, ChunkError> {
        let bytes = self.bytes(4)?;
        Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    fn i32(&mut self) -> Result<i32, ChunkError> {
        Ok(self.u32()?.cast_signed())
    }

    /// A count of items that are each at least `item_size` bytes long.
    fn count(&mut self, item_size: usize) -> Result<usize, ChunkError> {
        let count = usize::try_from(self.i32()?)
            .map_err(|_| error("a negative count in the compiled Lua script"))?;
        if count.saturating_mul(item_size) > self.data.len() - self.position {
            return Err(error("the compiled Lua script is truncated"));
        }
        Ok(count)
    }

    fn string(&mut self) -> Result<Option<String>, ChunkError> {
        let length = usize::try_from(self.u32()?)
            .map_err(|_| error("a string too long for this computer"))?;
        if length == 0 {
            return Ok(None);
        }
        let bytes = self.bytes(length)?;
        let (text, terminator) = bytes.split_at(length - 1);
        if terminator != [0] {
            return Err(error("an unterminated string in the compiled Lua script"));
        }
        Ok(Some(String::from_utf8_lossy(text).into_owned()))
    }

    fn function(&mut self, depth: usize) -> Result<Function, ChunkError> {
        if depth > MAX_NESTING {
            return Err(error(
                "functions nested too deeply in the compiled Lua script",
            ));
        }
        self.string()?; // source name
        self.bytes(8)?; // first and last line
        let header = self.bytes(4)?; // upvalues, parameters, vararg flag, stack size
        let (parameters, stack) = (header[1], header[3]);
        let code_count = self.count(4)?;
        let code = (0..code_count)
            .map(|_| self.u32())
            .collect::<Result<Vec<_>, _>>()?;
        let constant_count = self.count(1)?;
        let mut constants = Vec::with_capacity(constant_count);
        for _ in 0..constant_count {
            constants.push(match self.byte()? {
                0 => Constant::Nil,
                1 => Constant::Boolean(self.byte()? != 0),
                3 => {
                    let bytes = self.bytes(8)?;
                    let mut number = [0; 8];
                    number.copy_from_slice(bytes);
                    Constant::Number(f64::from_le_bytes(number))
                }
                4 => Constant::String(self.string()?.unwrap_or_default()),
                other => return Err(error(format!("an unknown constant type {other}"))),
            });
        }
        let function_count = self.count(1)?;
        let functions = (0..function_count)
            .map(|_| self.function(depth + 1))
            .collect::<Result<Vec<_>, _>>()?;
        // Debug information: line numbers, local names, upvalue names.
        let lines = self.count(4)?;
        self.bytes(lines * 4)?;
        for _ in 0..self.count(12)? {
            self.string()?;
            self.bytes(8)?;
        }
        for _ in 0..self.count(4)? {
            self.string()?;
        }
        Ok(Function {
            parameters,
            stack,
            code,
            constants,
            functions,
        })
    }
}

/// A Lua 5.1 opcode.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Op {
    Move,
    LoadK,
    LoadBool,
    LoadNil,
    GetUpval,
    GetGlobal,
    GetTable,
    SetGlobal,
    SetUpval,
    SetTable,
    NewTable,
    /// `SELF`: looks up a method and passes the object as the first argument.
    Method,
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Pow,
    Unm,
    Not,
    Len,
    Concat,
    Jmp,
    Eq,
    Lt,
    Le,
    Test,
    TestSet,
    Call,
    TailCall,
    Return,
    ForLoop,
    ForPrep,
    TForLoop,
    SetList,
    Close,
    Closure,
    VarArg,
}

const OPS: [Op; 38] = [
    Op::Move,
    Op::LoadK,
    Op::LoadBool,
    Op::LoadNil,
    Op::GetUpval,
    Op::GetGlobal,
    Op::GetTable,
    Op::SetGlobal,
    Op::SetUpval,
    Op::SetTable,
    Op::NewTable,
    Op::Method,
    Op::Add,
    Op::Sub,
    Op::Mul,
    Op::Div,
    Op::Mod,
    Op::Pow,
    Op::Unm,
    Op::Not,
    Op::Len,
    Op::Concat,
    Op::Jmp,
    Op::Eq,
    Op::Lt,
    Op::Le,
    Op::Test,
    Op::TestSet,
    Op::Call,
    Op::TailCall,
    Op::Return,
    Op::ForLoop,
    Op::ForPrep,
    Op::TForLoop,
    Op::SetList,
    Op::Close,
    Op::Closure,
    Op::VarArg,
];

/// Arguments at or above this value of a `B` or `C` field name constants.
pub const CONSTANT_BIT: u32 = 256;

/// One decoded instruction.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Instruction {
    pub op: Op,
    pub a: u32,
    pub b: u32,
    pub c: u32,
    pub bx: u32,
}

impl Instruction {
    /// Decodes an instruction; `None` for an unknown opcode.
    #[must_use]
    pub fn decode(word: u32) -> Option<Self> {
        Some(Self {
            op: *OPS.get(usize::try_from(word & 0x3f).ok()?)?,
            a: (word >> 6) & 0xff,
            c: (word >> 14) & 0x1ff,
            b: (word >> 23) & 0x1ff,
            bx: word >> 14,
        })
    }

    /// The signed jump offset of `JMP`, `FORLOOP`, and `FORPREP`.
    #[must_use]
    pub fn sbx(self) -> i64 {
        i64::from(self.bx) - 131_071
    }
}

#[cfg(test)]
pub mod assemble {
    //! Writes chunks for tests.

    use super::{Constant, HEADER, OPS, Op};

    /// Encodes an `A B C` instruction.
    #[must_use]
    pub fn abc(op: Op, a: u32, b: u32, c: u32) -> u32 {
        opcode(op) | (a << 6) | (c << 14) | (b << 23)
    }

    /// Encodes an `A Bx` instruction.
    #[must_use]
    pub fn abx(op: Op, a: u32, bx: u32) -> u32 {
        opcode(op) | (a << 6) | (bx << 14)
    }

    /// Encodes an `A sBx` instruction.
    #[must_use]
    pub fn asbx(op: Op, a: u32, sbx: i32) -> u32 {
        let bx = u32::try_from(sbx + 131_071).expect("jump in range");
        abx(op, a, bx)
    }

    fn opcode(op: Op) -> u32 {
        u32::try_from(OPS.iter().position(|known| *known == op).expect("known")).expect("small")
    }

    /// A function of a test chunk.
    #[derive(Clone, Debug, Default)]
    pub struct Builder {
        pub parameters: u8,
        pub stack: u8,
        pub code: Vec<u32>,
        pub constants: Vec<Constant>,
        pub functions: Vec<Builder>,
    }

    impl Builder {
        /// The index of a constant, added when new.
        pub fn k(&mut self, constant: Constant) -> u32 {
            let index = self
                .constants
                .iter()
                .position(|known| *known == constant)
                .unwrap_or_else(|| {
                    self.constants.push(constant);
                    self.constants.len() - 1
                });
            u32::try_from(index).expect("few constants")
        }

        /// The index of a string constant.
        pub fn s(&mut self, text: &str) -> u32 {
            self.k(Constant::String(text.to_owned()))
        }

        fn write(&self, bytes: &mut Vec<u8>) {
            bytes.extend_from_slice(&0_u32.to_le_bytes());
            bytes.extend_from_slice(&[0; 8]);
            bytes.extend_from_slice(&[0, self.parameters, 0, self.stack]);
            count(bytes, self.code.len());
            for word in &self.code {
                bytes.extend_from_slice(&word.to_le_bytes());
            }
            count(bytes, self.constants.len());
            for constant in &self.constants {
                match constant {
                    Constant::Nil => bytes.push(0),
                    Constant::Boolean(value) => bytes.extend_from_slice(&[1, u8::from(*value)]),
                    Constant::Number(value) => {
                        bytes.push(3);
                        bytes.extend_from_slice(&value.to_le_bytes());
                    }
                    Constant::String(text) => {
                        bytes.push(4);
                        count(bytes, text.len() + 1);
                        bytes.extend_from_slice(text.as_bytes());
                        bytes.push(0);
                    }
                }
            }
            count(bytes, self.functions.len());
            for function in &self.functions {
                function.write(bytes);
            }
            bytes.extend_from_slice(&[0; 12]);
        }

        /// The chunk with this function as its main function.
        #[must_use]
        pub fn chunk(&self) -> Vec<u8> {
            let mut bytes = HEADER.to_vec();
            self.write(&mut bytes);
            bytes
        }
    }

    fn count(bytes: &mut Vec<u8>, value: usize) {
        bytes.extend_from_slice(&u32::try_from(value).expect("small").to_le_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::assemble::{Builder, abc};
    use super::*;

    #[test]
    fn a_chunk_round_trips_its_code_constants_and_functions() {
        let mut inner = Builder::default();
        inner.s("OnScene00000");
        inner.code.push(abc(Op::Return, 0, 1, 0));
        let mut main = Builder::default();
        main.k(Constant::Number(1.5));
        main.k(Constant::Boolean(true));
        main.k(Constant::Nil);
        main.functions.push(inner);
        main.code.push(abc(Op::Return, 0, 1, 0));
        let function = read_chunk(&main.chunk()).expect("chunk");
        assert_eq!(
            function.constants,
            [
                Constant::Number(1.5),
                Constant::Boolean(true),
                Constant::Nil
            ]
        );
        assert_eq!(function.functions[0].string(0), Some("OnScene00000"));
        let instruction = Instruction::decode(function.code[0]).expect("known op");
        assert_eq!(
            (instruction.op, instruction.a, instruction.b),
            (Op::Return, 0, 1)
        );
    }

    #[test]
    fn other_formats_and_truncated_chunks_are_refused() {
        let mut main = Builder::default();
        main.code.push(abc(Op::Return, 0, 1, 0));
        let chunk = main.chunk();
        assert!(read_chunk(b"print('hi')").is_err());
        let mut big_endian = chunk.clone();
        big_endian[6] = 0;
        assert_eq!(
            read_chunk(&big_endian),
            Err(error("a compiled Lua script in an unsupported format"))
        );
        for length in [12, 20, chunk.len() - 1] {
            assert!(read_chunk(&chunk[..length]).is_err(), "{length}");
        }
        let mut trailing = chunk;
        trailing.push(0);
        assert!(read_chunk(&trailing).is_err());
    }
}
