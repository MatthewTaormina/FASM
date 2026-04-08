/// All token kinds produced by the FASM lexer.
#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    // Literals
    Integer(i64),
    HexInteger(u64),
    Float(f64),
    StringLit(String),

    // Identifiers and keywords (all upper-case in FASM)
    Ident(String),

    // Punctuation
    Comma,
    Ampersand, // & prefix for deref
    Dot,       // for library.Function

    // Comments consume their line, not emitted as tokens
    Eof,
}

#[derive(Debug, Clone)]
pub struct Token {
    pub kind: TokenKind,
    pub line: usize,
}

impl Token {
    pub fn new(kind: TokenKind, line: usize) -> Self {
        Self { kind, line }
    }
}
