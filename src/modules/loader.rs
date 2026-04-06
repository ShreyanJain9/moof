/// Module loader — discover, parse, sort, load.
///
/// Coordinates module loading:
/// 1. Discover .moof files in a seed directory
/// 2. Parse module headers
/// 3. Build dependency graph
/// 4. Load in topological order (sandboxed)
/// 5. Merge exports into root env for REPL

use std::path::{Path, PathBuf};
use std::fs;
use std::collections::HashMap;

use crate::reader::lexer::Lexer;
use crate::reader::parser::Parser;
use crate::compiler::compile::Compiler;
use crate::runtime::value::{Value, HeapObject};
use crate::runtime::heap::Heap;
use crate::vm::exec::VM;
use super::{ModuleDescriptor, graph::ModuleGraph, sandbox};

pub struct ModuleLoader {
    /// The dependency graph
    pub graph: ModuleGraph,
    /// Module name -> environment id (after loading)
    pub loaded_envs: HashMap<String, u32>,
    /// Module name -> list of (symbol_name, value) exports
    pub exports: HashMap<String, Vec<(String, Value)>>,
    /// Module name -> full source text (for persistence)
    pub source_texts: HashMap<String, String>,
    /// Image directory (.moof/)
    pub image_dir: PathBuf,
}

impl ModuleLoader {
    /// Discover all .moof files in a directory, parse headers, build graph.
    pub fn discover(dir: &Path, heap: &mut Heap) -> Result<Self, String> {
        let mut descriptors = Vec::new();

        let entries = fs::read_dir(dir)
            .map_err(|e| format!("cannot read {}: {}", dir.display(), e))?;

        for entry in entries {
            let entry = entry.map_err(|e| format!("readdir: {}", e))?;
            let path = entry.path();
            if path.extension().map_or(true, |e| e != "moof") {
                continue;
            }

            let source = fs::read_to_string(&path)
                .map_err(|e| format!("cannot read {}: {}", path.display(), e))?;

            match parse_header(&source, &path, heap) {
                Ok(desc) => descriptors.push(desc),
                Err(_) => continue,
            }
        }

        let graph = ModuleGraph::build(descriptors)?;

        Ok(ModuleLoader {
            graph,
            loaded_envs: HashMap::new(),
            exports: HashMap::new(),
            source_texts: HashMap::new(),
            image_dir: PathBuf::from(".moof"),
        })
    }

    /// Get the topological load order.
    pub fn load_order(&self) -> Result<Vec<String>, String> {
        self.graph.topo_sort()
    }

    /// Load a single module by name. Public so the caller can control
    /// the loading sequence (e.g., load bootstrap, register natives, then continue).
    pub fn load_one(&mut self, name: &str, vm: &mut VM) -> Result<(), String> {
        self.load_module(name, vm)
    }

    /// Load a single module into its sandboxed environment.
    fn load_module(&mut self, name: &str, vm: &mut VM) -> Result<(), String> {
        let desc = self.graph.modules.get(name)
            .ok_or_else(|| format!("unknown module: {}", name))?
            .clone();

        // Read source — from path if available, else from stored source_texts
        let source = if let Some(ref path) = desc.path {
            fs::read_to_string(path)
                .map_err(|e| format!("cannot read {}: {}", path.display(), e))?
        } else if let Some(src) = self.source_texts.get(name) {
            src.clone()
        } else {
            return Err(format!("{}: no source available", name));
        };

        // Store source text for persistence
        self.source_texts.insert(name.to_string(), source.clone());

        // Get the body (everything after the module header)
        let body = &source[desc.body_offset..];

        // Build imports from required modules' exports
        let mut imports: HashMap<String, Value> = HashMap::new();
        for req in &desc.requires {
            if let Some(req_exports) = self.exports.get(req) {
                for (sym_name, val) in req_exports {
                    imports.insert(sym_name.clone(), *val);
                }
            }
        }

        // Determine compiler mode: bootstrap and unrestricted modules get full access
        let is_unrestricted = name == "bootstrap" || desc.unrestricted;

        // Create the module's evaluation environment
        let env_id = if is_unrestricted {
            sandbox::create_unrestricted_env(vm, &imports)
        } else {
            sandbox::create_sandbox_env(vm, &imports)
        };

        // Lex + parse the body
        let mut lexer = Lexer::new(body);
        let tokens = lexer.tokenize()
            .map_err(|e| format!("{}: lex error: {}", name, e))?;
        let mut parser = Parser::new(tokens);
        let exprs = parser.parse_all(&mut vm.heap)
            .map_err(|e| format!("{}: parse error: {}", name, e))?;

        // Compile and execute each expression
        for expr in exprs {
            let mut compiler = if is_unrestricted {
                Compiler::new()
            } else {
                Compiler::new_sandboxed()
            };
            let chunk = compiler.compile_expr(&mut vm.heap, expr)
                .map_err(|e| format!("{}: compile error: {}", name, e))?;
            let chunk_id = vm.heap.alloc_chunk(chunk);
            vm.execute(chunk_id, env_id)
                .map_err(|e| format!("{}: runtime error: {}", name, e))?;
        }

        // Collect exports: look up each `provides` symbol in the module env
        let mut module_exports = Vec::new();
        for provide in &desc.provides {
            let sym = vm.heap.intern(provide);
            match vm.env_lookup(env_id, sym) {
                Ok(val) => {
                    module_exports.push((provide.clone(), val));
                }
                Err(_) => {
                    return Err(format!(
                        "{}: provides '{}' but it is not defined after loading",
                        name, provide
                    ));
                }
            }
        }

        self.loaded_envs.insert(name.to_string(), env_id);
        self.exports.insert(name.to_string(), module_exports);

        // ── Create heap-resident ModuleImage + Definition objects ──
        // Only if the ModuleImage prototype exists (i.e., bootstrap has loaded).
        self.register_module_on_heap(name, &desc, &source, env_id, vm);

        eprintln!("(loaded module: {})", name);
        Ok(())
    }

    /// Create a ModuleImage and its Definition objects on the heap,
    /// and register it in the Modules registry.
    /// No-op if ModuleImage prototype or Modules object don't exist yet.
    pub fn register_module_on_heap(
        &self,
        name: &str,
        desc: &super::ModuleDescriptor,
        source: &str,
        env_id: u32,
        vm: &mut VM,
    ) {
        let root_env = match vm.vat.root_env {
            Some(r) => r,
            None => return,
        };

        // Look up ModuleImage prototype
        let sym_module_image = vm.heap.intern("ModuleImage");
        let module_image_proto = match vm.env_lookup(root_env, sym_module_image) {
            Ok(Value::Object(id)) => id,
            _ => return, // bootstrap hasn't loaded yet
        };

        // Look up Definition prototype
        let sym_definition = vm.heap.intern("Definition");
        let def_proto = match vm.env_lookup(root_env, sym_definition) {
            Ok(Value::Object(id)) => id,
            _ => return,
        };

        // Build requires list on the heap
        let requires_val = {
            let mut list = Value::Nil;
            for req in desc.requires.iter().rev() {
                let s = vm.heap.alloc_string(req);
                list = vm.heap.cons(s, list);
            }
            list
        };

        // Build provides list on the heap
        let provides_val = {
            let mut list = Value::Nil;
            for prov in desc.provides.iter().rev() {
                let s = vm.heap.alloc_string(prov);
                list = vm.heap.cons(s, list);
            }
            list
        };

        // Split source body into definitions
        let body = &source[desc.body_offset..];
        let defs = split_into_definitions(body);

        // Create Definition objects
        let mut definitions_list = Value::Nil;
        let name_sym = vm.heap.intern("name");
        let source_sym = vm.heap.intern("source");
        let kind_sym = vm.heap.intern("kind");
        let module_name_sym = vm.heap.intern("module-name");
        let mod_name_val = vm.heap.alloc_string(name);

        for (def_name, def_source) in defs.iter().rev() {
            // All definitions from module source files are code
            let kind = "code";

            let def_name_val = vm.heap.alloc_string(def_name);
            let def_source_val = vm.heap.alloc_string(def_source);
            let kind_val = Value::Symbol(vm.heap.intern(kind));

            let def_id = vm.heap.alloc(HeapObject::GeneralObject {
                parent: Value::Object(def_proto),
                slots: vec![
                    (module_name_sym, mod_name_val),
                    (name_sym, def_name_val),
                    (source_sym, def_source_val),
                    (kind_sym, kind_val),
                ],
                handlers: Vec::new(),
            });
            definitions_list = vm.heap.cons(Value::Object(def_id), definitions_list);
        }

        // Create ModuleImage object
        let mod_name_sym = vm.heap.intern("name");
        let requires_sym = vm.heap.intern("requires");
        let provides_sym = vm.heap.intern("provides");
        let definitions_sym = vm.heap.intern("definitions");
        let durable_objects_sym = vm.heap.intern("durable-objects");
        let env_sym = vm.heap.intern("env");
        let unrestricted_sym = vm.heap.intern("unrestricted");
        let mod_name_val = vm.heap.alloc_string(name);

        let mod_id = vm.heap.alloc(HeapObject::GeneralObject {
            parent: Value::Object(module_image_proto),
            slots: vec![
                (mod_name_sym, mod_name_val),
                (requires_sym, requires_val),
                (provides_sym, provides_val),
                (definitions_sym, definitions_list),
                (durable_objects_sym, Value::Nil),
                (env_sym, Value::Object(env_id)),
                (unrestricted_sym, if desc.unrestricted { Value::True } else { Value::False }),
            ],
            handlers: Vec::new(),
        });

        // Register in Modules registry via its Assoc (if Modules exists)
        if let Some(modules_obj) = vm.find_module_registry() {
            let registry_val = vm.read_slot(modules_obj, "_registry");
            if let Value::Object(registry_id) = registry_val {
                let sel_set = vm.heap.intern("set:to:");
                let name_val = vm.heap.alloc_string(name);
                let _ = vm.message_send(
                    Value::Object(registry_id),
                    sel_set,
                    &[name_val, Value::Object(mod_id)],
                );
            }
        }
    }

    /// Merge all module exports into the root environment.
    /// Reads provides + env from heap ModuleImage objects.
    pub fn merge_into_root(&self, vm: &mut VM, root_env: u32) {
        let modules = vm.all_module_ids();
        for (_, mod_id) in &modules {
            let provides_val = vm.read_slot(*mod_id, "provides");
            let env_val = vm.read_slot(*mod_id, "env");
            let env_id = match env_val {
                Value::Object(id) => id,
                _ => continue,
            };
            let provides = vm.read_string_list(provides_val);
            for prov_name in provides {
                let sym = vm.heap.intern(&prov_name);
                if let Ok(val) = vm.env_lookup(env_id, sym) {
                    vm.env_define_helper(root_env, sym, val);
                }
            }
        }
    }

}

/// Find the byte offset of the end of a top-level form starting at the beginning of `s`.
fn find_form_end(s: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    if bytes.is_empty() || bytes[0] != b'(' { return None; }

    let mut depth = 0i32;
    let mut in_string = false;
    let mut escape = false;
    let mut i = 0;

    while i < bytes.len() {
        let ch = bytes[i];
        if escape { escape = false; i += 1; continue; }
        if in_string {
            if ch == b'\\' { escape = true; }
            else if ch == b'"' { in_string = false; }
            i += 1;
            continue;
        }
        match ch {
            b'"' => in_string = true,
            b';' => { while i < bytes.len() && bytes[i] != b'\n' { i += 1; } continue; }
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => {
                depth -= 1;
                if depth == 0 { return Some(i + 1); }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// Split a module body into individual top-level definition forms.
/// Returns (def_name, form_source) for each `(def ...)` or `(defmethod ...)` form.
/// Non-definition forms (bare expressions, comments) are returned with name "__expr_N".
fn split_into_definitions(body: &str) -> Vec<(String, String)> {
    let mut result = Vec::new();
    let mut pos = 0;
    let mut expr_count = 0;
    let bytes = body.as_bytes();

    while pos < bytes.len() {
        // Skip whitespace
        while pos < bytes.len() && bytes[pos].is_ascii_whitespace() {
            pos += 1;
        }
        if pos >= bytes.len() { break; }

        // Skip comment lines (collect them as prefix for the next form)
        let mut comment_start = pos;
        let mut has_comments = false;
        while pos < bytes.len() && bytes[pos] == b';' {
            has_comments = true;
            while pos < bytes.len() && bytes[pos] != b'\n' {
                pos += 1;
            }
            if pos < bytes.len() { pos += 1; } // skip the newline
            // Skip whitespace between comment lines
            while pos < bytes.len() && (bytes[pos] == b' ' || bytes[pos] == b'\t') {
                pos += 1;
            }
        }

        if pos >= bytes.len() { break; }

        // If we hit a non-paren after comments, skip the line
        if bytes[pos] != b'(' && bytes[pos] != b'[' && bytes[pos] != b'{' {
            while pos < bytes.len() && bytes[pos] != b'\n' {
                pos += 1;
            }
            continue;
        }

        let form_start = if has_comments { comment_start } else { pos };

        if let Some(end) = find_form_end(&body[pos..]) {
            let form_text_start = pos;
            let form_end = pos + end;
            let form_source = body[form_start..form_end].trim().to_string();

            // Extract the name: look for (def NAME or (defmethod TYPE SELECTOR
            let inner = &body[form_text_start..form_end];
            let name = extract_def_name_from_form(inner);

            match name {
                Some(n) => result.push((n, form_source)),
                None => {
                    expr_count += 1;
                    result.push((format!("__expr_{}", expr_count), form_source));
                }
            }

            pos = form_end;
        } else {
            break;
        }
    }

    result
}

/// Extract the definition name from a form like `(def NAME ...)` or `(defmethod TYPE SEL ...)`.
fn extract_def_name_from_form(form: &str) -> Option<String> {
    let trimmed = form.trim();
    if trimmed.starts_with("(def ") || trimmed.starts_with("(def\n") || trimmed.starts_with("(def\t") {
        // (def NAME ...)
        let after_def = &trimmed[4..].trim_start();
        let end = after_def.find(|c: char| c.is_whitespace() || c == '(' || c == '{' || c == '[')
            .unwrap_or(after_def.len());
        let name = &after_def[..end];
        if !name.is_empty() { Some(name.to_string()) } else { None }
    } else if trimmed.starts_with("(defmethod ") {
        // (defmethod TYPE SEL ...) — name it as "TYPE.SEL" for identification
        let after = &trimmed[11..].trim_start();
        let type_end = after.find(|c: char| c.is_whitespace()).unwrap_or(after.len());
        let type_name = &after[..type_end];
        let rest = after[type_end..].trim_start();
        let sel_end = rest.find(|c: char| c.is_whitespace() || c == '(').unwrap_or(rest.len());
        let sel_name = &rest[..sel_end];
        Some(format!("{}.{}", type_name, sel_name))
    } else {
        None
    }
}

/// Parse the module header from a .moof file's source text.
///
/// Expected form: (module NAME (requires DEP1 DEP2 ...) (provides SYM1 SYM2 ...))
pub fn parse_header(source: &str, path: &Path, heap: &mut Heap) -> Result<ModuleDescriptor, String> {
    let body_offset = find_first_form_end(source)?;

    let header_source = &source[..body_offset];
    let mut lexer = Lexer::new(header_source);
    let tokens = lexer.tokenize()
        .map_err(|e| format!("header lex error: {}", e))?;
    let mut parser = Parser::new(tokens);
    let exprs = parser.parse_all(heap)
        .map_err(|e| format!("header parse error: {}", e))?;

    if exprs.is_empty() {
        return Err("empty file".into());
    }

    let header_expr = exprs[0];
    let elements = heap.list_to_vec(header_expr);
    if elements.is_empty() {
        return Err("expected (module ...) form".into());
    }

    match elements[0] {
        Value::Symbol(sym) if heap.symbol_name(sym) == "module" => {}
        _ => return Err("first form is not (module ...)".into()),
    }

    if elements.len() < 2 {
        return Err("(module) missing name".into());
    }

    let name = match elements[1] {
        Value::Symbol(sym) => heap.symbol_name(sym).to_string(),
        _ => return Err("module name must be a symbol".into()),
    };

    let mut requires = Vec::new();
    let mut provides = Vec::new();
    let mut unrestricted = false;

    for &element in &elements[2..] {
        if let Value::Symbol(sym) = element {
            let kw = heap.symbol_name(sym).to_string();
            if kw == "unrestricted" {
                unrestricted = true;
                continue;
            }
        }

        let sub = heap.list_to_vec(element);
        if sub.is_empty() { continue; }

        match sub[0] {
            Value::Symbol(sym) => {
                let kw = heap.symbol_name(sym).to_string();
                match kw.as_str() {
                    "requires" => {
                        for &item in &sub[1..] {
                            match item {
                                Value::Symbol(s) => requires.push(heap.symbol_name(s).to_string()),
                                _ => return Err("requires entries must be symbols".into()),
                            }
                        }
                    }
                    "provides" => {
                        for &item in &sub[1..] {
                            match item {
                                Value::Symbol(s) => provides.push(heap.symbol_name(s).to_string()),
                                _ => return Err("provides entries must be symbols".into()),
                            }
                        }
                    }
                    "unrestricted" => {
                        unrestricted = true;
                    }
                    other => return Err(format!("unknown module clause: {}", other)),
                }
            }
            _ => return Err("module clause must start with a symbol".into()),
        }
    }

    let source_hash = String::new();

    Ok(ModuleDescriptor {
        name,
        requires,
        provides,
        path: Some(path.to_path_buf()),
        source_hash,
        body_offset,
        unrestricted,
    })
}

/// Find the byte offset just past the end of the first top-level s-expression.
fn find_first_form_end(source: &str) -> Result<usize, String> {
    let bytes = source.as_bytes();
    let mut i = 0;

    loop {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i < bytes.len() && bytes[i] == b';' {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
        } else {
            break;
        }
    }

    if i >= bytes.len() || bytes[i] != b'(' {
        return Err("no opening paren found for module header".into());
    }

    let mut depth = 0i32;
    let mut in_string = false;
    let mut escape = false;

    while i < bytes.len() {
        let ch = bytes[i];
        if escape { escape = false; i += 1; continue; }
        if in_string {
            if ch == b'\\' { escape = true; }
            else if ch == b'"' { in_string = false; }
            i += 1;
            continue;
        }
        match ch {
            b'"' => in_string = true,
            b';' => {
                while i < bytes.len() && bytes[i] != b'\n' { i += 1; }
                continue;
            }
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => {
                depth -= 1;
                if depth == 0 { return Ok(i + 1); }
            }
            _ => {}
        }
        i += 1;
    }

    Err("unmatched paren in module header".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_first_form_end() {
        let source = "(module foo (requires) (provides bar))\n(def bar 42)";
        let offset = find_first_form_end(source).unwrap();
        assert_eq!(source[offset..].trim(), "(def bar 42)");
    }

    #[test]
    fn test_find_first_form_end_with_comments() {
        let source = "; header comment\n(module foo (requires) (provides))\n(def x 1)";
        let offset = find_first_form_end(source).unwrap();
        assert!(source[offset..].contains("(def x 1)"));
    }

    #[test]
    fn test_parse_header() {
        let mut heap = Heap::new();
        let source = "(module collections (requires bootstrap) (provides Assoc))\n(def Assoc 42)";
        let path = PathBuf::from("lib/collections.moof");
        let desc = parse_header(source, &path, &mut heap).unwrap();

        assert_eq!(desc.name, "collections");
        assert_eq!(desc.requires, vec!["bootstrap"]);
        assert_eq!(desc.provides, vec!["Assoc"]);
        assert!(desc.body_offset > 0);
        assert!(source[desc.body_offset..].contains("(def Assoc 42)"));
    }

    #[test]
    fn test_parse_header_no_deps() {
        let mut heap = Heap::new();
        let source = "(module bootstrap (requires) (provides fn do let))\n(def fn 1)";
        let path = PathBuf::from("lib/bootstrap.moof");
        let desc = parse_header(source, &path, &mut heap).unwrap();

        assert_eq!(desc.name, "bootstrap");
        assert!(desc.requires.is_empty());
        assert_eq!(desc.provides, vec!["fn", "do", "let"]);
    }
}
