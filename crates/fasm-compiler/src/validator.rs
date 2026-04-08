use std::collections::HashSet;
use crate::ast::*;

/// Static validation pass — checks the AST for structural errors before emit.
pub fn validate(prog: &ProgramAst) -> Result<(), String> {
    // 1. No duplicate FUNC names
    let mut func_names = HashSet::new();
    for f in &prog.functions {
        if !func_names.insert(f.name.clone()) {
            return Err(format!("Line {}: duplicate function name '{}'", f.line, f.name));
        }
    }

    // 2. Main must exist
    if !func_names.contains("Main") {
        return Err("No 'Main' function found in program".into());
    }

    // 3. Validate each function
    for func in &prog.functions {
        validate_function(func, &func_names, prog)?;
    }

    Ok(())
}

fn validate_function(func: &Function, all_funcs: &HashSet<String>, prog: &ProgramAst) -> Result<(), String> {
    // Collect declared local names and param names
    let mut declared: HashSet<String> = prog.defines.iter().map(|d| d.name.clone()).collect();
    // Add builtin symbols
    declared.insert("$args".into());
    declared.insert("$ret".into());
    declared.insert("$fault_code".into());
    declared.insert("$fault_index".into());
    declared.insert("NULL".into());
    declared.insert("TRUE".into());
    declared.insert("FALSE".into());

    for p in &func.params {
        declared.insert(p.name.clone());
    }

    // Collect label names & check duplicates
    let mut labels: HashSet<String> = HashSet::new();
    collect_labels(&func.body, &mut labels)?;

    // Validate body
    let mut tmp_depth = 0;
    validate_statements(&func.body, &mut declared, all_funcs, &labels, &func.name, &mut tmp_depth)?;
    if tmp_depth > 0 {
        return Err(format!("Function '{}': unclosed TMP_BLOCK", func.name));
    }

    Ok(())
}

fn collect_labels(stmts: &[Statement], labels: &mut HashSet<String>) -> Result<(), String> {
    for stmt in stmts {
        match stmt {
            Statement::Label(name, line) => {
                if !labels.insert(name.clone()) {
                    return Err(format!("Line {}: duplicate label '{}'", line, name));
                }
            }
            Statement::Instr(instr) => {
                if instr.mnemonic == "TMP_BLOCK" || instr.mnemonic == "END_TMP" {
                    // labels can't be safely jumped across if they span blocks
                }
            }
            Statement::TryBlock { body, catch_body, .. } => {
                collect_labels(body, labels)?;
                collect_labels(catch_body, labels)?;
            }
            _ => {}
        }
    }
    Ok(())
}

fn validate_statements(
    stmts: &[Statement],
    declared: &mut HashSet<String>,
    all_funcs: &HashSet<String>,
    labels: &HashSet<String>,
    func_name: &str,
    tmp_depth: &mut usize,
) -> Result<(), String> {
    for stmt in stmts {
        match stmt {
            Statement::Local(decl) => {
                declared.insert(decl.name.clone());
            }
            Statement::Label(_name, _line) => {}
            Statement::Instr(instr) => {
                if instr.mnemonic == "TMP_BLOCK" { *tmp_depth += 1; }
                validate_instr(instr, declared, all_funcs, labels, func_name, *tmp_depth)?;
                if instr.mnemonic == "END_TMP" { 
                    if *tmp_depth == 0 { return Err(format!("Line {}: END_TMP without TMP_BLOCK", instr.line)); }
                    *tmp_depth -= 1; 
                }
            }
            Statement::TryBlock { body, catch_body, .. } => {
                validate_statements(body, declared, all_funcs, labels, func_name, tmp_depth)?;
                validate_statements(catch_body, declared, all_funcs, labels, func_name, tmp_depth)?;
            }
        }
    }
    Ok(())
}

fn validate_instr(
    instr: &Instr,
    declared: &HashSet<String>,
    all_funcs: &HashSet<String>,
    labels: &HashSet<String>,
    func_name: &str,
    tmp_depth: usize,
) -> Result<(), String> {
    match instr.mnemonic.as_str() {
        "JMP" | "JZ" | "JNZ" => {
            if tmp_depth > 0 {
                return Err(format!("Line {}: jump instruction not allowed inside TMP_BLOCK (must remain atomic)", instr.line));
            }
            // last operand should be a label
            if let Some(AstValue::Ident(label)) = instr.operands.last() {
                if !labels.contains(label.as_str()) {
                    return Err(format!("Line {}: undefined label '{}' in function '{}'", instr.line, label, func_name));
                }
            }
        }
        "CALL" | "ASYNC_CALL" => {
            if let Some(AstValue::Ident(name)) = instr.operands.first() {
                if !all_funcs.contains(name.as_str()) {
                    return Err(format!("Line {}: call to undefined function '{}'", instr.line, name));
                }
            }
        }
        _ => {}
    }

    // Check all ident references are declared
    for op in &instr.operands {
        match op {
            AstValue::Ident(name) => {
                // Skip type names, keywords, function names, label names
                if !declared.contains(name.as_str())
                    && !all_funcs.contains(name.as_str())
                    && !labels.contains(name.as_str())
                    && !is_type_name(name)
                    && !is_modifier(name)
                {
                    // Allow unknown idents as potential DEFINE names (lenient MVP validation)
                    // A stricter validator would error here
                }
            }
            AstValue::Deref(name) => {
                if !declared.contains(name.as_str()) {
                    return Err(format!("Line {}: dereference of undeclared slot '{}' in '{}'", instr.line, name, func_name));
                }
            }
            _ => {}
        }
    }
    Ok(())
}

fn is_type_name(s: &str) -> bool {
    matches!(s,
        "BOOL"|"INT8"|"INT16"|"INT32"|"INT64"|"UINT8"|"UINT16"|"UINT32"|"UINT64"|
        "FLOAT32"|"FLOAT64"|"REF_MUT"|"REF_IMM"|"VEC"|"STRUCT"|"STACK"|"QUEUE"|
        "HEAP_MIN"|"HEAP_MAX"|"OPTION"|"RESULT"|"FUTURE"|"NULL"
    )
}

fn is_modifier(s: &str) -> bool {
    matches!(s, "REQUIRED"|"OPTIONAL"|"AS")
}
