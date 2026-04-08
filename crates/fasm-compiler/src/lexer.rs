use crate::token::{Token, TokenKind};

pub fn tokenize(source: &str) -> Result<Vec<Token>, String> {
    let mut tokens = Vec::new();
    let mut chars = source.chars().peekable();
    let mut line = 1usize;

    while let Some(&ch) = chars.peek() {
        match ch {
            '\n' => { line += 1; chars.next(); }
            ' ' | '\t' | '\r' => { chars.next(); }

            '/' => {
                chars.next();
                if chars.peek() == Some(&'/') {
                    // Line comment — consume to end of line
                    while let Some(&c) = chars.peek() {
                        chars.next();
                        if c == '\n' { line += 1; break; }
                    }
                } else {
                    return Err(format!("Line {}: unexpected '/'", line));
                }
            }

            ',' => { chars.next(); tokens.push(Token::new(TokenKind::Comma, line)); }
            '&' => { chars.next(); tokens.push(Token::new(TokenKind::Ampersand, line)); }
            '.' => { chars.next(); tokens.push(Token::new(TokenKind::Dot, line)); }

            '"' => {
                chars.next();
                let mut s = String::new();
                loop {
                    match chars.next() {
                        None => return Err(format!("Line {}: unterminated string", line)),
                        Some('"') => break,
                        Some('\\') => match chars.next() {
                            Some('n') => s.push('\n'),
                            Some('t') => s.push('\t'),
                            Some('"') => s.push('"'),
                            Some('\\') => s.push('\\'),
                            _ => {}
                        },
                        Some(c) => s.push(c),
                    }
                }
                tokens.push(Token::new(TokenKind::StringLit(s), line));
            }

            '0'..='9' | '-' => {
                let negative = ch == '-';
                if negative {
                    chars.next();
                    if !chars.peek().map(|c| c.is_ascii_digit()).unwrap_or(false) {
                        return Err(format!("Line {}: expected digit after '-'", line));
                    }
                }

                let mut num = String::new();
                if negative { num.push('-'); }

                // Check for hex
                if chars.peek() == Some(&'0') {
                    num.push('0');
                    chars.next();
                    if chars.peek() == Some(&'x') || chars.peek() == Some(&'X') {
                        chars.next();
                        let mut hex = String::new();
                        while let Some(&c) = chars.peek() {
                            if c.is_ascii_hexdigit() { hex.push(c); chars.next(); } else { break; }
                        }
                        let val = u64::from_str_radix(&hex, 16)
                            .map_err(|_| format!("Line {}: invalid hex literal", line))?;
                        tokens.push(Token::new(TokenKind::HexInteger(val), line));
                        continue;
                    }
                }

                while let Some(&c) = chars.peek() {
                    if c.is_ascii_digit() { num.push(c); chars.next(); }
                    else { break; }
                }

                if chars.peek() == Some(&'.') {
                    num.push('.');
                    chars.next();
                    while let Some(&c) = chars.peek() {
                        if c.is_ascii_digit() { num.push(c); chars.next(); } else { break; }
                    }
                    let f: f64 = num.parse().map_err(|_| format!("Line {}: bad float", line))?;
                    tokens.push(Token::new(TokenKind::Float(f), line));
                } else {
                    let i: i64 = num.parse().map_err(|_| format!("Line {}: bad integer '{}'", line, num))?;
                    tokens.push(Token::new(TokenKind::Integer(i), line));
                }
            }

            c if c.is_alphabetic() || c == '_' || c == '$' => {
                let mut ident = String::new();
                while let Some(&c) = chars.peek() {
                    if c.is_alphanumeric() || c == '_' || c == '$' {
                        ident.push(c); chars.next();
                    } else { break; }
                }
                tokens.push(Token::new(TokenKind::Ident(ident), line));
            }

            _ => {
                return Err(format!("Line {}: unexpected character '{}'", line, ch));
            }
        }
    }

    tokens.push(Token::new(TokenKind::Eof, line));
    Ok(tokens)
}
