// Minimal GUI notebook — opens an eframe window showing a
// Workspace's blocks. one-shot: the call blocks until the user
// closes the window. no interactivity yet (read-only view).
//
// Pattern: snapshot the workspace to Rust data, then hand the
// snapshot to eframe. the moof heap is NOT accessed from the
// GUI thread — we copy everything needed up front.

use moof::heap::Heap;
use moof::object::HeapObject;
use moof::value::Value;
use eframe::egui;

/// A block extracted from the moof heap for GUI rendering.
pub enum BlockSnapshot {
    Heading { text: String, level: i64 },
    Paragraph { text: String },
    Code { code: String, result: Option<String>, evaled: bool },
    ObjectRef { label: String, target_desc: String },
    Unknown { type_name: String, raw: String },
}

pub struct NotebookSnapshot {
    pub title: String,
    pub blocks: Vec<BlockSnapshot>,
}

impl NotebookSnapshot {
    /// Pull a Workspace value out of the heap and snapshot it.
    /// The moof value must be a Workspace (has title/blocks slots).
    pub fn from_workspace(heap: &Heap, workspace: Value) -> Result<Self, String> {
        let id = workspace.as_any_object().ok_or("not an object")?;
        let title_sym = heap.find_symbol("title").ok_or("no title symbol")?;
        let blocks_sym = heap.find_symbol("blocks").ok_or("no blocks symbol")?;

        let title = match heap.get(id).slot_get(title_sym) {
            Some(t) => string_of(heap, t).unwrap_or_else(|| "untitled".into()),
            None => "untitled".into(),
        };

        let blocks_val = heap.get(id).slot_get(blocks_sym).unwrap_or(Value::NIL);
        let block_values = heap.list_to_vec(blocks_val);

        let blocks = block_values.iter()
            .map(|b| snapshot_block(heap, *b))
            .collect();

        Ok(NotebookSnapshot { title, blocks })
    }
}

fn snapshot_block(heap: &Heap, block: Value) -> BlockSnapshot {
    let id = match block.as_any_object() {
        Some(id) => id,
        None => return BlockSnapshot::Unknown {
            type_name: "non-object".into(),
            raw: heap.format_value(block),
        },
    };

    // type name lookup — walk the parent chain looking for a
    // typeName slot (matches the moof [obj typeName] convention).
    let type_name = type_name_of(heap, block);

    match type_name.as_str() {
        "Heading" => {
            let text = slot_string(heap, id, "text").unwrap_or_default();
            let level = slot_integer(heap, id, "level").unwrap_or(1);
            BlockSnapshot::Heading { text, level }
        }
        "Paragraph" => {
            BlockSnapshot::Paragraph {
                text: slot_string(heap, id, "text").unwrap_or_default(),
            }
        }
        "CodeBlock" => {
            let code = match heap.find_symbol("code").and_then(|s| heap.get(id).slot_get(s)) {
                Some(v) => heap.format_value(v),
                None => String::new(),
            };
            let evaled = heap.find_symbol("evaled")
                .and_then(|s| heap.get(id).slot_get(s))
                .map(|v| v.is_true())
                .unwrap_or(false);
            let result = if evaled {
                heap.find_symbol("result")
                    .and_then(|s| heap.get(id).slot_get(s))
                    .map(|v| heap.format_value(v))
            } else {
                None
            };
            BlockSnapshot::Code { code, result, evaled }
        }
        "ObjectRef" => {
            let label = slot_string(heap, id, "label").unwrap_or_default();
            let target = heap.find_symbol("target")
                .and_then(|s| heap.get(id).slot_get(s))
                .map(|v| heap.format_value(v))
                .unwrap_or_default();
            BlockSnapshot::ObjectRef { label, target_desc: target }
        }
        _ => BlockSnapshot::Unknown {
            type_name,
            raw: heap.format_value(block),
        },
    }
}

fn type_name_of(heap: &Heap, val: Value) -> String {
    // walk parent chain looking for a typeName slot (set by moof's
    // [typeName] convention). falls back to HeapObject variant name.
    if let Some(type_name_sym) = heap.find_symbol("typeName") {
        let mut cur = val;
        for _ in 0..32 {
            if cur.is_nil() { break; }
            if let Some(id) = cur.as_any_object() {
                // check for a typeName HANDLER on this object (not just slot)
                if let Some(handler) = heap.get(id).handler_get(type_name_sym) {
                    // the handler body returns the type name symbol.
                    // without invoking bytecode here we fall back below,
                    // but we can check: if handler is a closure whose
                    // chunk returns a quoted symbol literal, extract it.
                    let _ = handler;
                }
                cur = heap.get(id).parent();
            } else {
                break;
            }
        }
    }
    // Fallback: variant-based type name
    if let Some(id) = val.as_any_object() {
        match heap.get(id) {
            HeapObject::General { parent, .. } => {
                // try parent's typeName slot
                if let Some(pid) = parent.as_any_object() {
                    if let Some(name_sym) = heap.find_symbol("__name") {
                        if let Some(n) = heap.get(pid).slot_get(name_sym) {
                            if let Some(s) = n.as_symbol() {
                                return heap.symbol_name(s).to_string();
                            }
                        }
                    }
                }
                "Object".to_string()
            }
            HeapObject::Pair(_, _) => "Cons".to_string(),
            HeapObject::Text(_) => "String".to_string(),
            HeapObject::Buffer(_) => "Bytes".to_string(),
            HeapObject::Table { .. } => "Table".to_string(),
            HeapObject::Environment { .. } => "Environment".to_string(),
        }
    } else {
        "primitive".to_string()
    }
}

fn slot_string(heap: &Heap, id: u32, name: &str) -> Option<String> {
    let sym = heap.find_symbol(name)?;
    let val = heap.get(id).slot_get(sym)?;
    string_of(heap, val)
}

fn slot_integer(heap: &Heap, id: u32, name: &str) -> Option<i64> {
    let sym = heap.find_symbol(name)?;
    heap.get(id).slot_get(sym)?.as_integer()
}

fn string_of(heap: &Heap, val: Value) -> Option<String> {
    let id = val.as_any_object()?;
    match heap.get(id) {
        HeapObject::Text(s) => Some(s.clone()),
        _ => None,
    }
}

// ═══════════════════════════════════════════════════════════
// GUI
// ═══════════════════════════════════════════════════════════

struct NotebookApp {
    snapshot: NotebookSnapshot,
}

impl eframe::App for NotebookApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                // title bar
                ui.heading(format!("═ {} ═", self.snapshot.title));
                ui.add_space(8.0);

                for block in &self.snapshot.blocks {
                    render_block(ui, block);
                    ui.add_space(8.0);
                }
            });
        });
    }
}

fn render_block(ui: &mut egui::Ui, block: &BlockSnapshot) {
    match block {
        BlockSnapshot::Heading { text, level } => {
            let size = match *level {
                1 => 28.0,
                2 => 22.0,
                3 => 18.0,
                _ => 16.0,
            };
            ui.label(
                egui::RichText::new(text)
                    .size(size)
                    .strong()
            );
        }
        BlockSnapshot::Paragraph { text } => {
            ui.label(text);
        }
        BlockSnapshot::Code { code, result, evaled } => {
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.label(egui::RichText::new(code).monospace());
                if *evaled {
                    if let Some(r) = result {
                        ui.separator();
                        ui.label(egui::RichText::new(format!("⇒ {r}")).monospace().weak());
                    }
                }
            });
        }
        BlockSnapshot::ObjectRef { label, target_desc } => {
            let s = if label.is_empty() {
                format!("→ {target_desc}")
            } else {
                format!("→ {label}: {target_desc}")
            };
            ui.label(egui::RichText::new(s).italics());
        }
        BlockSnapshot::Unknown { type_name, raw } => {
            ui.label(
                egui::RichText::new(format!("[{type_name}] {raw}"))
                    .weak()
                    .monospace()
            );
        }
    }
}

/// Open a window showing the given workspace. Blocks until the
/// user closes it.
pub fn show_workspace(snapshot: NotebookSnapshot) -> Result<(), String> {
    let title = format!("moof · {}", snapshot.title);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([700.0, 600.0]),
        ..Default::default()
    };
    eframe::run_native(
        &title,
        options,
        Box::new(|_cc| Ok(Box::new(NotebookApp { snapshot }))),
    ).map_err(|e| format!("notebook: {e}"))
}
