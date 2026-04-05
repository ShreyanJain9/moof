/// Module loader — discover, parse, sort, load, reload, remove.
///
/// Coordinates the full module lifecycle:
/// 1. Discover .moof files (from image dir or seed dir)
/// 2. Parse module headers
/// 3. Build dependency graph
/// 4. Load in topological order (sandboxed)
/// 5. Merge exports into root env for REPL

use std::path::{Path, PathBuf};
use std::fs;
use std::collections::{HashMap, BTreeMap};

use crate::reader::lexer::Lexer;
use crate::reader::parser::Parser;
use crate::compiler::compile::Compiler;
use crate::runtime::value::Value;
use crate::runtime::heap::Heap;
use crate::vm::exec::VM;
use crate::persistence::image;

use super::{ModuleDescriptor, graph::ModuleGraph, sandbox, cache};

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
        let env_id = sandbox::create_sandbox_env(vm, &imports);

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

        eprintln!("(loaded module: {})", name);
        Ok(())
    }

    /// Merge all module exports into the root environment.
    pub fn merge_into_root(&self, vm: &mut VM, root_env: u32) {
        for (_module_name, module_exports) in &self.exports {
            for (sym_name, val) in module_exports {
                let sym = vm.heap.intern(sym_name);
                vm.env_define_helper(root_env, sym, *val);
            }
        }
    }

    /// Reload a module and all its transitive dependents.
    pub fn reload(&mut self, name: &str, vm: &mut VM, root_env: u32) -> Result<(), String> {
        if !self.graph.modules.contains_key(name) {
            return Err(format!("unknown module: {}", name));
        }

        let dependents = self.graph.transitive_dependents(name);

        self.load_module(name, vm)?;

        for dep in &dependents {
            self.load_module(dep, vm)?;
        }

        self.merge_into_root(vm, root_env);
        Ok(())
    }

    /// Update a module's source text, re-parse header, re-evaluate.
    pub fn update_module_source(
        &mut self,
        name: &str,
        new_source: String,
        vm: &mut VM,
        root_env: u32,
    ) -> Result<(), String> {
        // Re-parse the header from the new source
        let dummy_path = PathBuf::from(format!("{}.moof", name));
        let new_desc = parse_header(&new_source, &dummy_path, &mut vm.heap)?;

        if new_desc.name != name {
            return Err(format!("module name changed from '{}' to '{}'", name, new_desc.name));
        }

        // Update the descriptor in the graph
        self.graph.modules.insert(name.to_string(), new_desc);
        self.source_texts.insert(name.to_string(), new_source);

        // Reload module + dependents
        self.reload(name, vm, root_env)
    }

    /// Remove a module. Returns the symbols that were unbound.
    pub fn remove(&mut self, name: &str) -> Result<Vec<String>, String> {
        self.graph.can_remove(name).map_err(|deps| {
            format!("cannot remove '{}': depended on by {}", name, deps.join(", "))
        })?;

        let provides = self.exports.remove(name)
            .unwrap_or_default()
            .into_iter()
            .map(|(sym, _)| sym)
            .collect();

        self.loaded_envs.remove(name);
        self.source_texts.remove(name);

        Ok(provides)
    }

    /// List all loaded modules and their dependency info.
    pub fn list_modules(&self) -> Vec<(&str, &[String], &[String])> {
        let mut result: Vec<(&str, &[String], &[String])> = Vec::new();
        if let Ok(order) = self.graph.topo_sort() {
            for name in &order {
                if let Some(desc) = self.graph.modules.get(name) {
                    result.push((&desc.name, &desc.requires, &desc.provides));
                }
            }
        }
        result
    }

    /// Save the current state to the image directory.
    /// Writes each module's source file + the manifest.
    pub fn save_image(&self) -> Result<String, String> {
        let order = self.graph.topo_sort()?;

        let mut source_hashes = BTreeMap::new();
        let mut provides_counts = BTreeMap::new();

        for name in &order {
            let source = self.source_texts.get(name)
                .ok_or_else(|| format!("no source text for module '{}'", name))?;

            let hash = image::save_module_source(&self.image_dir, name, source)?;
            source_hashes.insert(name.clone(), hash);

            let count = self.exports.get(name).map(|e| e.len()).unwrap_or(0);
            provides_counts.insert(name.clone(), count);
        }

        let manifest = image::build_manifest(&order, &source_hashes, &provides_counts);
        image::save_manifest(&self.image_dir, &manifest)?;

        Ok(manifest.global_hash)
    }

    /// Save just one module's source and update the manifest.
    pub fn save_module(&self, name: &str) -> Result<String, String> {
        // Save the individual module source
        let source = self.source_texts.get(name)
            .ok_or_else(|| format!("no source text for module '{}'", name))?;
        image::save_module_source(&self.image_dir, name, source)?;

        // Rebuild and save the full manifest
        self.save_image()
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

    let source_hash = cache::sha256_hex(source.as_bytes());

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
