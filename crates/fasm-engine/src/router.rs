//! Route table — maps `(method, path_pattern)` to a compiled FASM program and function name.
//!
//! Path patterns use `:param` segments (e.g. `/api/users/:id`).
//! Matched parameters are injected into the FASM `$args` struct as string fields
//! named by the parameter (field 0 = first path param, 1 = second, etc.).

use fasm_bytecode::Program;
use fasm_compiler::compile_source;
use fasm_jit::FasmJit;
use std::{collections::HashMap, path::Path, sync::Arc};
use uuid::Uuid;

use crate::config::RouteConfig;

// ── Route entry ────────────────────────────────────────────────────────────────

pub struct RouteEntry {
    /// Stable unique ID — used to remove a specific route later.
    pub id: Uuid,
    pub method: String,
    /// Parsed path segments (with param markers).
    segments: Vec<Segment>,
    pub func: String,
    pub program: Arc<Program>,
    /// Pre-compiled JIT cache for this program.  `None` if no eligible functions.
    pub jit: Option<Arc<FasmJit>>,
    /// `true` = registered via the management API (can be hot-removed).
    /// `false` = loaded from static config (treated as immutable at runtime).
    pub managed: bool,
}

#[derive(Clone, Debug)]
enum Segment {
    Literal(String),
    Param(String),
}

// ── RouteTable ─────────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct RouteTable {
    routes: Vec<RouteEntry>,
}

/// Result of a successful route match.
pub struct MatchedRoute {
    pub func: String,
    pub program: Arc<Program>,
    pub jit: Option<Arc<FasmJit>>,
    /// Extracted path parameters: param name → value.
    pub params: HashMap<String, String>,
}

impl RouteTable {
    pub fn new() -> Self {
        Self::default()
    }

    /// Build a `RouteTable` from the list of static route configs.
    ///
    /// Each `.fasm` source is compiled at startup.
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
                false, // static route
            );
        }
        Ok(table)
    }

    fn add(
        &mut self,
        method: String,
        path: &str,
        func: String,
        program: Arc<Program>,
        managed: bool,
    ) -> Uuid {
        let id = Uuid::new_v4();
        let segments = parse_path(path);
        let jit = FasmJit::compile(&program).map(Arc::new);
        self.routes.push(RouteEntry {
            id,
            method,
            segments,
            func,
            program,
            jit,
            managed,
        });
        id
    }

    /// Register a new route at runtime (management API).
    ///
    /// Returns `Err` if the `(method, path)` combination is already occupied.
    pub fn add_route_dyn(
        &mut self,
        method: &str,
        path: &str,
        func: String,
        program: Arc<Program>,
    ) -> Result<Uuid, String> {
        let method_up = method.to_uppercase();
        let segments = parse_path(path);

        // Collision check — same method and identical segment pattern.
        for existing in &self.routes {
            if existing.method == method_up && segments_equal(&existing.segments, &segments) {
                return Err(format!(
                    "route conflict: {} {} is already registered (id={})",
                    method_up, path, existing.id
                ));
            }
        }

        let id = Uuid::new_v4();
        let jit = FasmJit::compile(&program).map(Arc::new);
        self.routes.push(RouteEntry {
            id,
            method: method_up,
            segments,
            func,
            program,
            jit,
            managed: true,
        });
        Ok(id)
    }

    /// Remove a managed route by ID.  Returns `false` if not found or not managed.
    pub fn remove_route(&mut self, id: Uuid) -> bool {
        if let Some(pos) = self.routes.iter().position(|r| r.id == id && r.managed) {
            self.routes.remove(pos);
            true
        } else {
            false
        }
    }

    /// Match an incoming `(method, path)` against the route table.
    pub fn match_route(&self, method: &str, path: &str) -> Option<MatchedRoute> {
        let method = method.to_uppercase();
        let request_segments: Vec<&str> = path.trim_matches('/').split('/').collect();

        for route in &self.routes {
            if route.method != method {
                continue;
            }
            if route.segments.len() != request_segments.len() {
                continue;
            }

            let mut params = HashMap::new();
            let mut matched = true;

            for (seg, req) in route.segments.iter().zip(request_segments.iter()) {
                match seg {
                    Segment::Literal(lit) => {
                        if lit != req {
                            matched = false;
                            break;
                        }
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
                    jit: route.jit.clone(),
                    params,
                });
            }
        }
        None
    }

    /// Return a snapshot list of all routes (for admin introspection).
    pub fn list(&self) -> Vec<RouteSnapshot> {
        self.routes
            .iter()
            .map(|r| RouteSnapshot {
                id: r.id,
                method: r.method.clone(),
                path: segments_to_path(&r.segments),
                func: r.func.clone(),
                managed: r.managed,
            })
            .collect()
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Lightweight, serialisable view of a route (no Arc<Program>).
#[derive(serde::Serialize)]
pub struct RouteSnapshot {
    pub id: Uuid,
    pub method: String,
    pub path: String,
    pub func: String,
    pub managed: bool,
}

fn parse_path(path: &str) -> Vec<Segment> {
    path.trim_matches('/')
        .split('/')
        .map(|s| {
            if let Some(rest) = s.strip_prefix(':') {
                Segment::Param(rest.to_string())
            } else {
                Segment::Literal(s.to_string())
            }
        })
        .collect()
}

fn segments_equal(a: &[Segment], b: &[Segment]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b.iter()).all(|(x, y)| match (x, y) {
        (Segment::Literal(a), Segment::Literal(b)) => a == b,
        (Segment::Param(_), Segment::Param(_)) => true, // same arity counts as conflict
        _ => false,
    })
}

fn segments_to_path(segs: &[Segment]) -> String {
    let parts: Vec<String> = segs
        .iter()
        .map(|s| match s {
            Segment::Literal(l) => l.clone(),
            Segment::Param(p) => format!(":{}", p),
        })
        .collect();
    format!("/{}", parts.join("/"))
}

pub fn compile_source_file(path: &Path) -> Result<Program, String> {
    let src =
        std::fs::read_to_string(path).map_err(|e| format!("cannot read {:?}: {}", path, e))?;
    compile_source(&src).map_err(|e| format!("compile error in {:?}: {}", path, e))
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
            table.add(
                method.to_string(),
                path,
                func.to_string(),
                program.clone(),
                false,
            );
        }
        table
    }

    #[test]
    fn test_exact_match() {
        let table = table_from_routes(&[("GET", "/ping", "Ping")]);
        let m = table
            .match_route("GET", "/ping")
            .expect("should match /ping");
        assert_eq!(m.func, "Ping");
        assert!(m.params.is_empty());
    }

    #[test]
    fn test_param_extraction() {
        let table = table_from_routes(&[("GET", "/users/:id", "GetUser")]);
        let m = table
            .match_route("GET", "/users/42")
            .expect("should match /users/42");
        assert_eq!(m.func, "GetUser");
        assert_eq!(m.params.get("id").map(String::as_str), Some("42"));
    }

    #[test]
    fn test_multi_param() {
        let table = table_from_routes(&[("GET", "/orders/:order_id/items/:item_id", "GetItem")]);
        let m = table
            .match_route("GET", "/orders/7/items/3")
            .expect("should match");
        assert_eq!(m.params["order_id"], "7");
        assert_eq!(m.params["item_id"], "3");
    }

    #[test]
    fn test_method_mismatch_returns_none() {
        let table = table_from_routes(&[("GET", "/ping", "Ping")]);
        assert!(
            table.match_route("POST", "/ping").is_none(),
            "POST /ping should not match a GET route"
        );
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
            ("GET", "/api/:slug", "Generic"),
        ]);
        let m = table
            .match_route("GET", "/api/special")
            .expect("should match");
        assert_eq!(m.func, "Special", "exact match should win over param route");
    }

    #[test]
    fn test_add_route_dyn_and_match() {
        let src = "FUNC Main\n    RET\nENDF\n";
        let program = Arc::new(compile_source(src).unwrap());
        let mut table = RouteTable::new();
        let id = table
            .add_route_dyn("POST", "/fn/hello", "Hello".into(), program)
            .unwrap();
        let m = table
            .match_route("POST", "/fn/hello")
            .expect("should match dynamically added route");
        assert_eq!(m.func, "Hello");
        // route should appear in list with matching id
        let snap = table.list();
        assert!(snap.iter().any(|s| s.id == id && s.managed));
    }

    #[test]
    fn test_add_route_dyn_collision_returns_err() {
        let src = "FUNC Main\n    RET\nENDF\n";
        let program = Arc::new(compile_source(src).unwrap());
        let mut table = RouteTable::new();
        table
            .add_route_dyn("GET", "/hello", "H1".into(), program.clone())
            .unwrap();
        let err = table.add_route_dyn("GET", "/hello", "H2".into(), program);
        assert!(err.is_err(), "duplicate route should return Err");
    }

    #[test]
    fn test_remove_route_hot_unloads() {
        let src = "FUNC Main\n    RET\nENDF\n";
        let program = Arc::new(compile_source(src).unwrap());
        let mut table = RouteTable::new();
        let id = table
            .add_route_dyn("DELETE", "/rm-me", "Rm".into(), program)
            .unwrap();
        assert!(table.match_route("DELETE", "/rm-me").is_some());
        assert!(table.remove_route(id));
        assert!(
            table.match_route("DELETE", "/rm-me").is_none(),
            "route should be gone after removal"
        );
    }

    #[test]
    fn test_cannot_remove_static_route() {
        let src = "FUNC Main\n    RET\nENDF\n";
        let program = Arc::new(compile_source(src).unwrap());
        let mut table = RouteTable::new();
        let id = table.add("GET".into(), "/static", "S".into(), program, false);
        assert!(
            !table.remove_route(id),
            "static routes must not be removable via remove_route"
        );
    }
}
