//! Integration tests for fasm-compiler: lexer, validator, and end-to-end pipeline.

use fasm_compiler::parser::parse;
use fasm_compiler::token::TokenKind;
use fasm_compiler::{compile_source, lexer::tokenize, validator::validate};

// ── Lexer tests ───────────────────────────────────────────────────────────────
// Note: the lexer always appends an EOF token at the end of the output.

#[test]
fn test_lexer_integer_literal() {
    let tokens = tokenize("42").unwrap();
    // tokens = [Integer(42), Eof]
    assert_eq!(tokens.len(), 2);
    assert!(matches!(tokens[0].kind, TokenKind::Integer(42)));
    assert!(matches!(tokens[1].kind, TokenKind::Eof));
}

#[test]
fn test_lexer_negative_integer() {
    let tokens = tokenize("-7").unwrap();
    assert_eq!(tokens.len(), 2);
    assert!(matches!(tokens[0].kind, TokenKind::Integer(-7)));
}

#[test]
fn test_lexer_hex_literal() {
    let tokens = tokenize("0xFF").unwrap();
    assert_eq!(tokens.len(), 2);
    assert!(matches!(tokens[0].kind, TokenKind::HexInteger(0xFF)));
}

#[test]
fn test_lexer_float_literal() {
    let tokens = tokenize("1.5").unwrap();
    assert_eq!(tokens.len(), 2);
    match &tokens[0].kind {
        TokenKind::Float(f) => assert!((*f - 1.5).abs() < 1e-10),
        _ => panic!("expected float token"),
    }
}

#[test]
fn test_lexer_string_literal() {
    let tokens = tokenize("\"hello\"").unwrap();
    assert_eq!(tokens.len(), 2);
    assert!(matches!(&tokens[0].kind, TokenKind::StringLit(s) if s == "hello"));
}

#[test]
fn test_lexer_string_escape_sequences() {
    let tokens = tokenize("\"a\\nb\\tc\"").unwrap();
    assert_eq!(tokens.len(), 2);
    match &tokens[0].kind {
        TokenKind::StringLit(s) => {
            assert_eq!(s, "a\nb\tc");
        }
        _ => panic!("expected string literal"),
    }
}

#[test]
fn test_lexer_identifier() {
    let tokens = tokenize("FUNC").unwrap();
    assert_eq!(tokens.len(), 2);
    assert!(matches!(&tokens[0].kind, TokenKind::Ident(s) if s == "FUNC"));
}

#[test]
fn test_lexer_punctuation() {
    let tokens = tokenize(", & .").unwrap();
    // 3 punctuation tokens + EOF
    assert_eq!(tokens.len(), 4);
    assert!(matches!(tokens[0].kind, TokenKind::Comma));
    assert!(matches!(tokens[1].kind, TokenKind::Ampersand));
    assert!(matches!(tokens[2].kind, TokenKind::Dot));
}

#[test]
fn test_lexer_line_comment_ignored() {
    let tokens = tokenize("// this is a comment\n42").unwrap();
    // comment is consumed; remaining tokens: [Integer(42), Eof]
    assert_eq!(tokens.len(), 2);
    assert!(matches!(tokens[0].kind, TokenKind::Integer(42)));
}

#[test]
fn test_lexer_multiline_with_line_tracking() {
    let tokens = tokenize("FUNC\nMain").unwrap();
    // [Ident("FUNC"), Ident("Main"), Eof]
    assert_eq!(tokens.len(), 3);
    assert_eq!(tokens[0].line, 1);
    assert_eq!(tokens[1].line, 2);
}

#[test]
fn test_lexer_unterminated_string_error() {
    let result = tokenize("\"hello");
    assert!(result.is_err(), "unterminated string should error");
}

#[test]
fn test_lexer_unexpected_slash_error() {
    let result = tokenize("5 / 2");
    assert!(result.is_err(), "bare '/' should error");
}

#[test]
fn test_lexer_negative_no_digit_error() {
    let result = tokenize("-x");
    assert!(result.is_err(), "'-' not followed by digit should error");
}

#[test]
fn test_lexer_eof_token() {
    // Empty input produces only EOF
    let tokens = tokenize("").unwrap();
    assert_eq!(tokens.len(), 1);
    assert!(matches!(tokens[0].kind, TokenKind::Eof));
}

// ── Validator tests ───────────────────────────────────────────────────────────

fn parse_src(src: &str) -> fasm_compiler::ast::ProgramAst {
    let tokens = tokenize(src).expect("lex");
    parse(tokens).expect("parse")
}

#[test]
fn test_validator_rejects_missing_main() {
    let ast = parse_src("FUNC Helper\n    RET\nENDF\n");
    let result = validate(&ast);
    assert!(result.is_err());
    assert!(
        result.unwrap_err().contains("Main"),
        "error must mention 'Main'"
    );
}

#[test]
fn test_validator_accepts_main() {
    let ast = parse_src("FUNC Main\n    RET\nENDF\n");
    assert!(validate(&ast).is_ok());
}

#[test]
fn test_validator_rejects_duplicate_function_names() {
    let src = "
FUNC Main
    RET
ENDF

FUNC Main
    RET
ENDF
";
    let ast = parse_src(src);
    let result = validate(&ast);
    assert!(result.is_err());
    let msg = result.unwrap_err();
    assert!(msg.contains("duplicate"), "error: {}", msg);
}

#[test]
fn test_validator_rejects_duplicate_labels() {
    let src = "
FUNC Main
    LOCAL 0, BOOL, flag
    STORE TRUE, flag
    JNZ flag, MyLabel
    LABEL MyLabel
    LABEL MyLabel
    RET
ENDF
";
    let result = compile_source(src);
    assert!(result.is_err());
    let msg = result.unwrap_err();
    assert!(msg.contains("duplicate"), "error: {}", msg);
}

#[test]
fn test_validator_rejects_undefined_label() {
    let src = "
FUNC Main
    LOCAL 0, BOOL, flag
    STORE TRUE, flag
    JNZ flag, NoSuchLabel
    RET
ENDF
";
    let result = compile_source(src);
    assert!(result.is_err());
    let msg = result.unwrap_err();
    assert!(msg.contains("NoSuchLabel"), "error: {}", msg);
}

#[test]
fn test_validator_rejects_call_to_undefined_function() {
    let src = "
FUNC Main
    LOCAL 0, STRUCT, args
    RESERVE 0, STRUCT, NULL
    CALL Ghost, args
    RET
ENDF
";
    let result = compile_source(src);
    assert!(result.is_err());
    let msg = result.unwrap_err();
    assert!(msg.contains("Ghost"), "error: {}", msg);
}

#[test]
fn test_validator_rejects_jump_inside_tmp_block() {
    let src = "
FUNC Main
    LOCAL 0, BOOL, flag
    STORE FALSE, flag
    TMP_BLOCK
        JNZ flag, Somewhere
    END_TMP
    LABEL Somewhere
    RET
ENDF
";
    let result = compile_source(src);
    assert!(result.is_err());
    let msg = result.unwrap_err();
    assert!(
        msg.contains("TMP_BLOCK") || msg.contains("jump"),
        "error: {}",
        msg
    );
}

#[test]
fn test_validator_rejects_deref_of_undeclared_slot() {
    let src = "
FUNC Main
    STORE 1, &ghost
    RET
ENDF
";
    let result = compile_source(src);
    assert!(result.is_err());
    let msg = result.unwrap_err();
    assert!(msg.contains("ghost"), "error: {}", msg);
}

// ── compile_source end-to-end tests ──────────────────────────────────────────

#[test]
fn test_compile_minimal_program() {
    let src = "FUNC Main\n    RET\nENDF\n";
    let prog = compile_source(src).expect("compile must succeed");
    assert_eq!(prog.version, 0x01);
    assert!(prog.get_function("Main").is_some());
}

#[test]
fn test_compile_define_constant() {
    let src = "
DEFINE MY_CONST, 42

FUNC Main
    LOCAL 0, INT32, x
    STORE MY_CONST, x
    RET x
ENDF
";
    let prog = compile_source(src).expect("compile must succeed");
    let main = prog.get_function("Main").unwrap();
    assert!(!main.instructions.is_empty());
}

#[test]
fn test_compile_multiple_functions() {
    let src = "
FUNC Helper
    RET
ENDF

FUNC Main
    LOCAL 0, STRUCT, args
    RESERVE 0, STRUCT, NULL
    CALL Helper, args
    RET
ENDF
";
    let prog = compile_source(src).expect("compile must succeed");
    assert!(prog.get_function("Helper").is_some());
    assert!(prog.get_function("Main").is_some());
}

#[test]
fn test_compile_params() {
    let src = "
FUNC Greet
    PARAM 0, INT32, n, REQUIRED
    LOCAL 0, INT32, val
    GET_FIELD $args, 0, val
    RET val
ENDF

FUNC Main
    RET
ENDF
";
    let prog = compile_source(src).expect("compile must succeed");
    let greet = prog.get_function("Greet").unwrap();
    assert_eq!(greet.params.len(), 1);
    assert_eq!(greet.params[0].name, "n");
    assert!(greet.params[0].required);
}

#[test]
fn test_compile_global_reserve() {
    // Top-level RESERVE declares global variables (not the GLOBAL keyword)
    let src = "
RESERVE 0, INT32, 0
RESERVE 1, BOOL, FALSE

FUNC Main
    RET
ENDF
";
    let prog = compile_source(src).expect("compile must succeed");
    assert_eq!(prog.global_inits.len(), 2);
}

#[test]
fn test_compile_string_literal() {
    let src = r#"
FUNC Main
    LOCAL 0, VEC, msg
    STORE "hello", msg
    RET msg
ENDF
"#;
    let prog = compile_source(src).expect("compile must succeed");
    assert!(prog.get_function("Main").is_some());
}

#[test]
fn test_compile_try_catch_block() {
    // TRY/CATCH/ENDTRY — no label operands; labels are auto-generated by the parser
    let src = "
FUNC Main
    LOCAL 0, INT32, a
    LOCAL 1, INT32, z
    LOCAL 2, INT32, result
    STORE 1, a
    STORE 0, z
    TRY
        DIV a, z, result
    CATCH
        STORE 0, result
    ENDTRY
    RET result
ENDF
";
    let prog = compile_source(src).expect("compile must succeed");
    let main = prog.get_function("Main").unwrap();
    assert!(!main.instructions.is_empty());
}

#[test]
fn test_compile_error_on_syntax() {
    // A lone '@' is not valid FASM syntax
    let result = compile_source("FUNC Main\n    @ invalid\nENDF");
    assert!(result.is_err());
}
