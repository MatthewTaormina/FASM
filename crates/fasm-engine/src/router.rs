//! Route table — maps `(method, path_pattern)` to a compiled FASM program and function name.
//!
//! Path patterns use `:param` segments (e.g. `/api/users/:id`).
//! Matched parameters are injected into the FASM `$args` struct as string fields
//! named by the parameter (field 0 = first path param, 1 = second, etc.).

use std::{
    collections::HashMap,
    path::Path,
    sync::Arc,
};
use fasm_bytecode::Program;
use fasm_compiler::compile_source;

use crate::config::RouteConfig;


// ── Route entry ────────────────────────────────────────────────────────────────

pub struct RouteEntry {
    pub method:   String,
    /// Parsed path segments (with param markers).
    segments:     Vec<Segment>,
    pub func:     String,
    pub program:  Arc<Program>,
}

#[derive(Clone, Debug)]
enum Segment {
    Literal(String),
    Param(String),
}

// ── RouteTable ─────────────────────────────────────────────────────────────────

pub struct RouteTable {
    routes: Vec<RouteEntry>,
}

/// Result of a successful route match.
pub struct MatchedRoute {
    pub func:    String,
    pub program: Arc<Program>,
    /// Extracted path parameters: param name → value.
    pub params:  HashMap<String, String>,
}

impl RouteTable {
    pub fn new() -> Self {
        Self { routes: Vec::new() }
    }

    /// Build a `RouteTable` from the list of route configs.
    ///
    /// Each `.fasm` source is compiled; `.fasmc` files are not yet supported
    /// in this version (treated as source).
    pub fn from_configs(configs: &[RouteConfig], base_dir: &Path) -> Result<Self, String> {
        let mut table = Self::new();
        for cfg in configs {
            let source_path = base_dir.join(&cfg.source);
            let program = compile_source_file(&source_path)?;
            table.add(
                cfg.method.to_uppercase(),
                &cfg.path,
                cfg.function.clone(),
                Arc::new(program),
            );
        }
        Ok(table)
    }

    fn add(&mut self, method: String, path: &str, func: String, program: Arc<Program>) {
        let segments = parse_path(path);
        self.routes.push(RouteEntry { method, segments, func, program });
    }

    /// Match an incoming `(method, path)` against the route table.
    pub fn match_route(&self, method: &str, path: &str) -> Option<MatchedRoute> {
        let method = method.to_uppercase();
        let request_segments: Vec<&str> = path.trim_matches('/').split('/').collect();

        for route in &self.routes {
            if route.method != method { continue; }
            if route.segments.len() != request_segments.len() { continue; }

            let mut params = HashMap::new();
            let mut matched = true;

            for (seg, req) in route.segments.iter().zip(request_segments.iter()) {
                match seg {
                    Segment::Literal(lit) => {
                        if lit != req { matched = false; break; }
                    }
                    Segment::Param(name) => {
                        params.insert(name.clone(), req.to_string());
                    }
                }
            }

            if matched {
                return Some(MatchedRoute {
                    func: route.func.clone(),
                    program: route.program.clone(),
                    params,
                });
            }
        }
        None
    }
}

// ── helpers ───────────────────────────────────────────────────────────────────

fn parse_path(path: &str) -> Vec<Segment> {
    path.trim_matches('/').split('/')
        .map(|s| {
            if s.starts_with(':') {
                Segment::Param(s[1..].to_string())
            } else {
                Segment::Literal(s.to_string())
            }
        })
        .collect()
}

fn compile_source_file(path: &Path) -> Result<Program, String> {
    let src = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read {:?}: {}", path, e))?;
    compile_source(&src)
          .map_err(|e| format!("compile error in {:?}: {}", path, e))
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal RouteTable from (method, pattern, func) triples.
    /// Does NOT read any files — compiles trivial inline FASM programs.
    fn table_from_routes(routes: &[(&str, &str, &str)]) -> RouteTable {
        let src = "FUNC Main\n    RET\nENDF\n";
        let program = Arc::new(compile_source(src).expect("compile inline FASM"));
        let mut table = RouteTable::new();
        for (method, path, func) in routes {
            table.add(method.to_string(), path, func.to_string(), program.clone());
        }
        table
    }

    #[test]
    fn test_exact_match() {
        let table = table_from_routes(&[("GET", "/ping", "Ping")]);
        let m = table.match_route("GET", "/ping").expect("should match /ping");
        assert_eq!(m.func, "Ping");
        assert!(m.params.is_empty());
    }

    #[test]
    fn test_param_extraction() {
        let table = table_from_routes(&[("GET", "/users/:id", "GetUser")]);
        let m = table.match_route("GET", "/users/42").expect("should match /users/42");
        assert_eq!(m.func, "GetUser");
        assert_eq!(m.params.get("id").map(String::as_str), Some("42"));
    }

    #[test]
    fn test_multi_param() {
        let table = table_from_routes(&[("GET", "/orders/:order_id/items/:item_id", "GetItem")]);
        let m = table.match_route("GET", "/orders/7/items/3").expect("should match");
        assert_eq!(m.params["order_id"], "7");
        assert_eq!(m.params["item_id"], "3");
    }

    #[test]
    fn test_method_mismatch_returns_none() {
        let table = table_from_routes(&[("GET", "/ping", "Ping")]);
        assert!(table.match_route("POST", "/ping").is_none(), "POST /ping should not match a GET route");
    }

    #[test]
    fn test_unknown_path_returns_none() {
        let table = table_from_routes(&[("GET", "/ping", "Ping")]);
        assert!(table.match_route("GET", "/no/such/path").is_none());
    }

    #[test]
    fn test_segment_count_mismatch_returns_none() {
        let table = table_from_routes(&[("GET", "/a/b/c", "Abc")]);
        assert!(table.match_route("GET", "/a/b").is_none());
        assert!(table.match_route("GET", "/a/b/c/d").is_none());
    }

    #[test]
    fn test_first_matching_route_wins() {
        let table = table_from_routes(&[
            ("GET", "/api/special", "Special"),
            ("GET", "/api/:slug",   "Generic"),
        ]);
        let m = table.match_route("GET", "/api/special").expect("should match");
        assert_eq!(m.func, "Special", "exact match should win over param route");
    }
}
