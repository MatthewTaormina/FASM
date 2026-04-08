use crate::ast::*;
use crate::token::{Token, TokenKind};
use fasm_bytecode::types::FasmType;

pub fn parse(tokens: Vec<Token>) -> Result<ProgramAst, String> {
    let mut p = Parser::new(tokens);
    p.parse_program()
}

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    fn new(tokens: Vec<Token>) -> Self {
        Self { tokens, pos: 0 }
    }

    fn peek(&self) -> &Token {
        &self.tokens[self.pos]
    }

    fn advance(&mut self) -> &Token {
        let t = &self.tokens[self.pos];
        if self.pos + 1 < self.tokens.len() {
            self.pos += 1;
        }
        t
    }

    fn line(&self) -> usize {
        self.peek().line
    }

    fn expect_ident(&mut self) -> Result<String, String> {
        let t = self.advance().clone();
        match t.kind {
            TokenKind::Ident(s) => Ok(s),
            _ => Err(format!(
                "Line {}: expected identifier, got {:?}",
                t.line, t.kind
            )),
        }
    }

    fn expect_comma(&mut self) -> Result<(), String> {
        let t = self.advance().clone();
        if t.kind == TokenKind::Comma {
            Ok(())
        } else {
            Err(format!("Line {}: expected ',', got {:?}", t.line, t.kind))
        }
    }

    fn parse_program(&mut self) -> Result<ProgramAst, String> {
        let mut prog = ProgramAst::default();

        while self.peek().kind != TokenKind::Eof {
            let kw = match &self.peek().kind {
                TokenKind::Ident(s) => s.clone(),
                _ => {
                    return Err(format!(
                        "Line {}: unexpected token {:?}",
                        self.line(),
                        self.peek().kind
                    ))
                }
            };

            match kw.as_str() {
                "DEFINE" => {
                    let ln = self.line();
                    self.advance();
                    let name = self.expect_ident()?;
                    self.expect_comma()?;
                    let val = self.parse_value()?;
                    prog.defines.push(Define {
                        name,
                        value: val,
                        line: ln,
                    });
                }
                "IMPORT" => {
                    let ln = self.line();
                    self.advance();
                    let path = self.expect_string()?;
                    self.expect_ident()?; // AS
                    let alias = self.expect_ident()?;
                    prog.imports.push(Import {
                        path,
                        alias,
                        line: ln,
                    });
                }
                "RESERVE" => {
                    let ln = self.line();
                    self.advance();
                    let idx = self.parse_u32()?;
                    self.expect_comma()?;
                    let t = self.parse_type()?;
                    self.expect_comma()?;
                    let init = self.parse_value()?;
                    prog.global_reserves.push(GlobalReserve {
                        index: idx,
                        fasm_type: t,
                        init,
                        line: ln,
                    });
                }
                "FUNC" => {
                    let f = self.parse_function()?;
                    prog.functions.push(f);
                }
                _ => {
                    return Err(format!(
                        "Line {}: unexpected top-level keyword '{}'",
                        self.line(),
                        kw
                    ));
                }
            }
        }
        Ok(prog)
    }

    fn parse_function(&mut self) -> Result<Function, String> {
        let ln = self.line();
        self.advance(); // consume FUNC
        let name = self.expect_ident()?;
        let mut params = Vec::new();
        let mut body = Vec::new();

        loop {
            match self.peek().kind.clone() {
                TokenKind::Eof => return Err(format!("Line {}: unterminated FUNC '{}'", ln, name)),
                TokenKind::Ident(ref kw) if kw == "ENDF" => {
                    self.advance();
                    break;
                }
                TokenKind::Ident(ref kw) if kw == "PARAM" => {
                    params.push(self.parse_param()?);
                }
                _ => {
                    let stmt = self.parse_statement()?;
                    body.push(stmt);
                }
            }
        }
        Ok(Function {
            name,
            params,
            body,
            line: ln,
        })
    }

    fn parse_param(&mut self) -> Result<ParamDecl, String> {
        let ln = self.line();
        self.advance(); // PARAM
        let key = self.parse_value()?;
        self.expect_comma()?;
        let t = self.parse_type()?;
        self.expect_comma()?;
        let name = self.expect_ident()?;
        self.expect_comma()?;
        let req_str = self.expect_ident()?;
        let required = match req_str.as_str() {
            "REQUIRED" => true,
            "OPTIONAL" => false,
            _ => {
                return Err(format!(
                    "Line {}: expected REQUIRED or OPTIONAL, got '{}'",
                    ln, req_str
                ))
            }
        };
        Ok(ParamDecl {
            key,
            fasm_type: t,
            name,
            required,
            line: ln,
        })
    }

    fn parse_statement(&mut self) -> Result<Statement, String> {
        let ln = self.line();
        let kw = match &self.peek().kind {
            TokenKind::Ident(s) => s.clone(),
            _ => {
                return Err(format!(
                    "Line {}: expected statement, got {:?}",
                    ln,
                    self.peek().kind
                ))
            }
        };

        // TRY block
        if kw == "TRY" {
            return self.parse_try_block();
        }

        // LOCAL declaration
        if kw == "LOCAL" {
            self.advance();
            let idx = self.parse_u8()?;
            self.expect_comma()?;
            let t = self.parse_type()?;
            self.expect_comma()?;
            let name = self.expect_ident()?;
            return Ok(Statement::Local(LocalDecl {
                index: idx,
                fasm_type: t,
                name,
                line: ln,
            }));
        }

        // Label: if next peek after ident is NOT a comma, it might be a label name
        // Labels are defined as LABEL mnemonic on their own line
        if kw == "LABEL" {
            self.advance();
            let name = self.expect_ident()?;
            return Ok(Statement::Label(name, ln));
        }

        // Check if this is a label declaration (Name:) — handled as Instr with special mnemonic
        // In FASM syntax labels appear as "Name:" — but since we tokenise : as unknown,
        // let's support them as just IDENT followed by no comma (they become labels in parser)
        // We detect them by looking one token ahead for a colon in the source.
        // Since colon isn't a token, we tokenise labels as "LabelName:" — handle via LABEL keyword above.

        // Otherwise: generic instruction
        self.advance(); // consume mnemonic
        let mnemonic = kw.clone();

        // Handle ASYNC prefix for ASYNC CALL / ASYNC SYSCALL
        if mnemonic == "ASYNC" {
            let next = self.expect_ident()?;
            let full_mnemonic = format!("ASYNC_{}", next);
            let operands = self.parse_operand_list()?;
            return Ok(Statement::Instr(Instr {
                mnemonic: full_mnemonic,
                operands,
                line: ln,
            }));
        }

        let operands = self.parse_operand_list()?;
        Ok(Statement::Instr(Instr {
            mnemonic,
            operands,
            line: ln,
        }))
    }

    fn parse_try_block(&mut self) -> Result<Statement, String> {
        let ln = self.line();
        self.advance(); // TRY
        let mut body = Vec::new();
        let mut catch_body = Vec::new();
        let mut in_catch = false;

        loop {
            match self.peek().kind.clone() {
                TokenKind::Eof => return Err(format!("Line {}: unterminated TRY block", ln)),
                TokenKind::Ident(ref kw) if kw == "CATCH" => {
                    self.advance();
                    in_catch = true;
                }
                TokenKind::Ident(ref kw) if kw == "ENDTRY" => {
                    self.advance();
                    break;
                }
                _ => {
                    let stmt = self.parse_statement()?;
                    if in_catch {
                        catch_body.push(stmt);
                    } else {
                        body.push(stmt);
                    }
                }
            }
        }
        // Generate synthetic label names — resolved in emitter
        let catch_label = format!("__catch_{}", ln);
        let end_label = format!("__endtry_{}", ln);
        Ok(Statement::TryBlock {
            catch_label,
            end_label,
            body,
            catch_body,
            line: ln,
        })
    }

    fn parse_operand_list(&mut self) -> Result<Vec<AstValue>, String> {
        let mut ops = Vec::new();
        // Operands are comma-separated on the same line (we don't enforce newlines in tokens,
        // but FASM is newline-significant — check for Eof/keyword as terminator)
        loop {
            match &self.peek().kind {
                TokenKind::Eof => break,
                TokenKind::Ident(s) if is_keyword(s) => break,
                _ => {}
            }
            // Check for deref: & before ident
            if self.peek().kind == TokenKind::Ampersand {
                self.advance();
                let name = self.expect_ident()?;
                ops.push(AstValue::Deref(name));
            } else {
                ops.push(self.parse_value()?);
            }

            // After each operand look for comma
            if self.peek().kind == TokenKind::Comma {
                self.advance();
            } else {
                break;
            }
        }
        Ok(ops)
    }

    fn parse_value(&mut self) -> Result<AstValue, String> {
        let t = self.advance().clone();
        match t.kind {
            TokenKind::Integer(n) => Ok(AstValue::Integer(n)),
            TokenKind::HexInteger(n) => Ok(AstValue::HexInt(n)),
            TokenKind::Float(f) => Ok(AstValue::Float(f)),
            TokenKind::StringLit(s) => Ok(AstValue::Str(s)),
            TokenKind::Ident(s) => match s.as_str() {
                "NULL" => Ok(AstValue::Null),
                "TRUE" => Ok(AstValue::True),
                "FALSE" => Ok(AstValue::False),
                _ => Ok(AstValue::Ident(s)),
            },
            TokenKind::Ampersand => {
                let name = self.expect_ident()?;
                Ok(AstValue::Deref(name))
            }
            _ => Err(format!("Line {}: expected value, got {:?}", t.line, t.kind)),
        }
    }

    fn parse_type(&mut self) -> Result<FasmType, String> {
        let t = self.advance().clone();
        match t.kind {
            TokenKind::Ident(s) => {
                parse_fasm_type(&s).ok_or_else(|| format!("Line {}: unknown type '{}'", t.line, s))
            }
            _ => Err(format!(
                "Line {}: expected type name, got {:?}",
                t.line, t.kind
            )),
        }
    }

    fn parse_u32(&mut self) -> Result<u32, String> {
        let t = self.advance().clone();
        match t.kind {
            TokenKind::Integer(n) if n >= 0 => Ok(n as u32),
            TokenKind::HexInteger(n) => Ok(n as u32),
            _ => Err(format!("Line {}: expected u32, got {:?}", t.line, t.kind)),
        }
    }

    fn parse_u8(&mut self) -> Result<u8, String> {
        let v = self.parse_u32()?;
        if v > 255 {
            Err(format!("Local slot index {} out of range (0-255)", v))
        } else {
            Ok(v as u8)
        }
    }

    fn expect_string(&mut self) -> Result<String, String> {
        let t = self.advance().clone();
        match t.kind {
            TokenKind::StringLit(s) => Ok(s),
            _ => Err(format!(
                "Line {}: expected string literal, got {:?}",
                t.line, t.kind
            )),
        }
    }
}

fn parse_fasm_type(s: &str) -> Option<FasmType> {
    match s {
        "BOOL" => Some(FasmType::Bool),
        "INT8" => Some(FasmType::Int8),
        "INT16" => Some(FasmType::Int16),
        "INT32" => Some(FasmType::Int32),
        "INT64" => Some(FasmType::Int64),
        "UINT8" => Some(FasmType::Uint8),
        "UINT16" => Some(FasmType::Uint16),
        "UINT32" => Some(FasmType::Uint32),
        "UINT64" => Some(FasmType::Uint64),
        "FLOAT32" => Some(FasmType::Float32),
        "FLOAT64" => Some(FasmType::Float64),
        "REF_MUT" => Some(FasmType::RefMut),
        "REF_IMM" => Some(FasmType::RefImm),
        "VEC" => Some(FasmType::Vec),
        "STRUCT" => Some(FasmType::Struct),
        "STACK" => Some(FasmType::Stack),
        "QUEUE" => Some(FasmType::Queue),
        "HEAP_MIN" => Some(FasmType::HeapMin),
        "HEAP_MAX" => Some(FasmType::HeapMax),
        "SPARSE" => Some(FasmType::Sparse),
        "BTREE" => Some(FasmType::BTree),
        "SLICE" => Some(FasmType::Slice),
        "DEQUE" => Some(FasmType::Deque),
        "BITSET" => Some(FasmType::Bitset),
        "BITVEC" => Some(FasmType::Bitvec),
        "OPTION" => Some(FasmType::Option),
        "RESULT" => Some(FasmType::Result),
        "FUTURE" => Some(FasmType::Future),
        _ => None,
    }
}

fn is_keyword(s: &str) -> bool {
    matches!(
        s,
        "FUNC"|"ENDF"|"PARAM"|"LOCAL"|"CALL"|"ASYNC"|"RET"|"SYSCALL"|"AWAIT"|
        "RESERVE"|"RELEASE"|"MOV"|"STORE"|"ADDR"|"ADD"|"SUB"|"MUL"|"DIV"|"MOD"|"NEG"|
        "EQ"|"NEQ"|"LT"|"LTE"|"GT"|"GTE"|"AND"|"OR"|"XOR"|"NOT"|"SHL"|"SHR"|
        "JMP"|"JZ"|"JNZ"|"LABEL"|"PUSH"|"POP"|"ENQUEUE"|"DEQUEUE"|"PEEK"|
        "TMP_BLOCK"|"END_TMP"|
        "GET_IDX"|"SET_IDX"|"GET_FIELD"|"SET_FIELD"|"HAS_FIELD"|"DEL_FIELD"|"LEN"|
        "CAST"|"TRY"|"CATCH"|"ENDTRY"|"IMPORT"|"INCLUDE"|"DEFINE"|"IFDEF"|
        "IFNDEF"|"ELSE"|"ENDIF"|"MACRO"|"ENDM"|"ASSERT"|
        "SOME"|"IS_SOME"|"UNWRAP"|"OK"|"ERR"|"IS_OK"|"UNWRAP_OK"|"UNWRAP_ERR"|
        "HALT"|"TAIL_CALL"|
        // DEQUE
        "PREPEND"|"POP_BACK"|"PEEK_BACK"|
        // VEC native ops
        "VEC_SORT"|"VEC_FILTER"|"VEC_MERGE"|"VEC_SLICE"|
        // SPARSE
        "SPARSE_GET"|"SPARSE_SET"|"SPARSE_DEL"|"SPARSE_HAS"|
        // BTREE
        "BTREE_GET"|"BTREE_SET"|"BTREE_DEL"|"BTREE_HAS"|"BTREE_MIN"|"BTREE_MAX"|
        // BITSET
        "BIT_SET"|"BIT_CLR"|"BIT_GET"|"BIT_FLIP"|"BIT_COUNT"|"BIT_AND"|"BIT_OR"|"BIT_XOR"|"BIT_GROW"|
        // BITVEC
        "BITVEC_READ"|"BITVEC_WRITE"|"BITVEC_PUSH"
    )
}
