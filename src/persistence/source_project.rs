/// Source Projector: heap -> .moof source files
///
/// On every save, generate a `source/` directory alongside image.bin
/// where each named user definition gets its own .moof file.
/// These are READ-ONLY for git diffing — loading always uses image.bin.

use std::path::Path;
use std::fs;
use std::collections::BTreeMap;

use crate::runtime::value::{Value, HeapObject};
use crate::runtime::heap::Heap;

/// Project the root environment's bindings into individual .moof source files.
///
/// Creates `dir/source/` and writes one file per named definition.
pub fn project_source(heap: &Heap, root_env: u32, dir: &Path) -> Result<(), String> {
    let source_dir = dir.join("source");
    fs::create_dir_all(&source_dir)
        .map_err(|e| format!("Cannot create {}: {}", source_dir.display(), e))?;

    // Collect bindings from the root environment
    let bindings = match heap.get(root_env) {
        HeapObject::Environment(env) => &env.bindings,
        _ => return Err("root_env is not an Environment".into()),
    };

    // Collect all definitions, sorted by name for deterministic output
    let mut defs: BTreeMap<String, String> = BTreeMap::new();

    for (&sym_id, &val) in bindings.iter() {
        let name = heap.symbol_name(sym_id).to_string();

        // Skip internal names (*, %, $)
        if name.starts_with('*') || name.starts_with('%') || name.starts_with('$') {
            continue;
        }

        // Skip native functions
        if let Value::Object(id) = val {
            if matches!(heap.get(id), HeapObject::NativeFunction { .. }) {
                continue;
            }
        }

        // Generate the source representation
        if let Some(source) = format_definition(heap, &name, val) {
            defs.insert(name, source);
        }
    }

    // Remove stale .moof files from source/ that are no longer defined
    if let Ok(entries) = fs::read_dir(&source_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().map_or(false, |e| e == "moof") {
                if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                    if !defs.contains_key(stem) {
                        let _ = fs::remove_file(&path);
                    }
                }
            }
        }
    }

    // Write each definition
    for (name, source) in &defs {
        let file_name = sanitize_filename(name);
        let path = source_dir.join(format!("{}.moof", file_name));
        fs::write(&path, source)
            .map_err(|e| format!("Cannot write {}: {}", path.display(), e))?;
    }

    Ok(())
}

/// Generate a moof source string for a single definition.
fn format_definition(heap: &Heap, name: &str, val: Value) -> Option<String> {
    match val {
        Value::Nil => Some(format!("(def {} nil)\n", name)),
        Value::True => Some(format!("(def {} true)\n", name)),
        Value::False => Some(format!("(def {} false)\n", name)),
        Value::Integer(n) => Some(format!("(def {} {})\n", name, n)),
        Value::Float(f) => Some(format!("(def {} {})\n", name, f)),
        Value::Symbol(id) => Some(format!("(def {} '{})\n", name, heap.symbol_name(id))),
        Value::Object(id) => format_heap_definition(heap, name, id),
    }
}

/// Format a heap-allocated value as a definition.
fn format_heap_definition(heap: &Heap, name: &str, id: u32) -> Option<String> {
    match heap.get(id) {
        HeapObject::MoofString(s) => {
            Some(format!("(def {} \"{}\")\n", name, escape_string(s)))
        }

        HeapObject::Lambda { source, .. } => {
            if source.is_nil() {
                Some(format!("; {} — <lambda, no source>\n", name))
            } else {
                let src = format_value_as_source(heap, *source);
                Some(format!("(def {} {})\n", name, src))
            }
        }

        HeapObject::Operative { source, .. } => {
            if source.is_nil() {
                Some(format!("; {} — <operative, no source>\n", name))
            } else {
                let src = format_value_as_source(heap, *source);
                Some(format!("(def {} {})\n", name, src))
            }
        }

        HeapObject::GeneralObject { parent, slots, handlers } => {
            Some(format_object_definition(heap, name, *parent, slots, handlers))
        }

        HeapObject::Cons { .. } => {
            // A list value — format as quoted literal
            let src = format_value_as_source(heap, val_from_id(id));
            Some(format!("(def {} '{})\n", name, src))
        }

        HeapObject::Environment(_) => {
            // Skip environments (they're structural, not user data)
            None
        }

        HeapObject::BytecodeChunk(_) => {
            // Raw bytecode chunks aren't user-facing definitions
            None
        }

        HeapObject::NativeFunction { .. } => {
            // Already filtered out upstream, but just in case
            None
        }
    }
}

fn val_from_id(id: u32) -> Value {
    Value::Object(id)
}

/// Format an object definition, including slots and handlers.
fn format_object_definition(
    heap: &Heap,
    name: &str,
    parent: Value,
    slots: &[(u32, Value)],
    handlers: &[(u32, Value)],
) -> String {
    let mut lines = Vec::new();

    // Opening: (def Name { Parent
    let parent_name = match parent {
        Value::Nil => "Object".to_string(),
        Value::Object(pid) => {
            // Try to find the parent's name by looking it up
            find_object_name(heap, pid).unwrap_or_else(|| format!("<object #{}>", pid))
        }
        _ => "Object".to_string(),
    };

    // Collect slot definitions
    let mut slot_parts = Vec::new();
    for &(sym_id, val) in slots {
        let sname = heap.symbol_name(sym_id);
        let vstr = format_value_as_source(heap, val);
        slot_parts.push(format!("  {}: {}", sname, vstr));
    }

    // Collect single-keyword handlers that can go in the {} literal
    let mut inline_handlers = Vec::new();
    let mut multi_handlers = Vec::new();

    for &(sym_id, handler_val) in handlers {
        let sel_name = heap.symbol_name(sym_id).to_string();

        // Get handler source
        let handler_source = match handler_val {
            Value::Object(hid) => match heap.get(hid) {
                HeapObject::Lambda { source, .. } if !source.is_nil() => {
                    Some(format_value_as_source(heap, *source))
                }
                HeapObject::Operative { source, .. } if !source.is_nil() => {
                    Some(format_value_as_source(heap, *source))
                }
                _ => None,
            },
            _ => None,
        };

        // Multi-keyword selectors (contain :) need defmethod / handle!
        if sel_name.matches(':').count() > 1 || sel_name.ends_with(':') {
            if let Some(src) = handler_source {
                multi_handlers.push((sel_name, src));
            }
        } else if let Some(src) = handler_source {
            // Extract params and body from the lambda/fn source
            inline_handlers.push((sel_name, src));
        }
    }

    // Build the { } literal
    let mut obj_parts = Vec::new();
    obj_parts.push(format!("(def {} {{ {}", name, parent_name));

    for sp in &slot_parts {
        obj_parts.push(sp.clone());
    }

    // For inline handlers, try to format them nicely
    for (sel, src) in &inline_handlers {
        // The source is the full (fn ...) or (lambda ...) form
        // Extract params and body for compact {} notation
        if let Some((params, body)) = extract_fn_params_body(heap, src) {
            obj_parts.push(format!("  {}: {}", sel, params));
            for line in body {
                obj_parts.push(format!("    {}", line));
            }
        } else {
            // Fallback: just show the source
            obj_parts.push(format!("  ; handler {}", sel));
        }
    }

    obj_parts.push("})".to_string());
    lines.push(obj_parts.join("\n"));

    // Multi-keyword handlers go after as handle! or defmethod
    for (sel, src) in &multi_handlers {
        lines.push(format!(
            "\n(handle! {} [\"{}\" toSymbol] {})",
            name, sel, src
        ));
    }

    lines.push(String::new()); // trailing newline
    lines.join("\n")
}

/// Try to extract (params) and body lines from a fn/lambda source string.
/// Returns None if it can't parse the structure.
fn extract_fn_params_body(_heap: &Heap, src: &str) -> Option<(String, Vec<String>)> {
    // Simple heuristic: if it starts with (fn (params) body...) or (lambda (params) body...)
    let trimmed = src.trim();
    if !(trimmed.starts_with("(fn ") || trimmed.starts_with("(lambda ")) {
        return None;
    }

    // Find the params list
    let after_keyword = if trimmed.starts_with("(fn ") {
        &trimmed[4..]
    } else {
        &trimmed[8..]
    };

    // Find matching paren for params
    if !after_keyword.starts_with('(') {
        // Might be variadic like `args`
        // Find end of the symbol
        let end = after_keyword.find(|c: char| c.is_whitespace()).unwrap_or(after_keyword.len());
        let params = &after_keyword[..end];
        let rest = after_keyword[end..].trim();
        // Remove trailing )
        let rest = if rest.ends_with(')') { &rest[..rest.len()-1] } else { rest };
        let body_lines: Vec<String> = rest.lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect();
        return Some((params.to_string(), body_lines));
    }

    let mut depth = 0;
    let mut param_end = 0;
    for (i, ch) in after_keyword.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    param_end = i + 1;
                    break;
                }
            }
            _ => {}
        }
    }

    if param_end == 0 { return None; }

    let params = &after_keyword[..param_end];
    let rest = after_keyword[param_end..].trim();
    // Remove the trailing ) from the (fn ...) form
    let rest = if rest.ends_with(')') { &rest[..rest.len()-1] } else { rest };
    let body_lines: Vec<String> = rest.lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect();

    Some((params.to_string(), body_lines))
}

/// Find a name for an object by scanning the root env.
/// This is a heuristic — we walk env 0's bindings.
fn find_object_name(heap: &Heap, target_id: u32) -> Option<String> {
    // Walk env 0 (typically root)
    if let HeapObject::Environment(env) = heap.get(0) {
        for (&sym_id, &val) in env.bindings.iter() {
            if val == Value::Object(target_id) {
                return Some(heap.symbol_name(sym_id).to_string());
            }
        }
    }
    None
}

/// Format a Value as moof source (s-expression).
fn format_value_as_source(heap: &Heap, val: Value) -> String {
    match val {
        Value::Nil => "nil".to_string(),
        Value::True => "true".to_string(),
        Value::False => "false".to_string(),
        Value::Integer(n) => n.to_string(),
        Value::Float(f) => format!("{}", f),
        Value::Symbol(id) => heap.symbol_name(id).to_string(),
        Value::Object(id) => format_heap_value_as_source(heap, id),
    }
}

/// Format a heap object as source.
fn format_heap_value_as_source(heap: &Heap, id: u32) -> String {
    match heap.get(id) {
        HeapObject::MoofString(s) => format!("\"{}\"", escape_string(s)),

        HeapObject::Cons { .. } => format_list_as_source(heap, Value::Object(id)),

        HeapObject::Lambda { source, .. } if !source.is_nil() => {
            format_value_as_source(heap, *source)
        }

        HeapObject::Operative { source, .. } if !source.is_nil() => {
            format_value_as_source(heap, *source)
        }

        HeapObject::GeneralObject { .. } => {
            // Try to find a name
            find_object_name(heap, id)
                .unwrap_or_else(|| format!("<object #{}>", id))
        }

        HeapObject::NativeFunction { name } => format!("<native {}>", name),
        HeapObject::BytecodeChunk(_) => "<bytecode>".to_string(),
        HeapObject::Lambda { .. } => "<lambda>".to_string(),
        HeapObject::Operative { .. } => "<operative>".to_string(),
        HeapObject::Environment(_) => "<environment>".to_string(),
    }
}

/// Format a cons list as a source s-expression.
fn format_list_as_source(heap: &Heap, val: Value) -> String {
    let mut parts = Vec::new();
    let mut current = val;
    loop {
        match current {
            Value::Nil => break,
            Value::Object(id) => match heap.get(id) {
                HeapObject::Cons { car, cdr } => {
                    parts.push(format_value_as_source(heap, *car));
                    current = *cdr;
                }
                _ => {
                    parts.push(format!(". {}", format_value_as_source(heap, current)));
                    break;
                }
            },
            other => {
                parts.push(format!(". {}", format_value_as_source(heap, other)));
                break;
            }
        }
    }
    format!("({})", parts.join(" "))
}

/// Escape a string for moof source output.
fn escape_string(s: &str) -> String {
    s.replace('\\', "\\\\")
     .replace('"', "\\\"")
     .replace('\n', "\\n")
     .replace('\t', "\\t")
     .replace('\r', "\\r")
}

/// Sanitize a name for use as a filename.
fn sanitize_filename(name: &str) -> String {
    name.chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            _ => c,
        })
        .collect()
}
