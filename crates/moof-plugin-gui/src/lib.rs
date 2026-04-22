// moof-plugin-gui — eframe-backed notebook view, exposed to moof
// as a `Gui` prototype bound in the root env.
//
// Usage from moof:
//
//   (def w (Workspace "scratch"))
//   ... build up blocks ...
//   [Gui show: w]                 ; blocks until window closes
//
// Same-vat semantics: `show:` runs on the caller's heap. This lets
// the snapshot walk protos + handlers without cross-vat copying.
// A future capability-vat version would require the workspace to
// pre-flatten itself to a plain snapshot dict before sending.

use eframe::egui;
use moof_core::heap::Heap;
use moof_core::value::Value;
use moof_core::{Plugin, native};

// ═══════════════════════════════════════════════════════════
// Snapshot: moof → plain Rust data
// ═══════════════════════════════════════════════════════════

enum BlockSnapshot {
    Heading { text: String, level: i64 },
    Paragraph { text: String },
    Code { code: String, result: Option<String>, evaled: bool },
    ObjectRef { label: String, target_desc: String },
    Unknown { type_name: String, raw: String },
}

struct NotebookSnapshot {
    title: String,
    blocks: Vec<BlockSnapshot>,
}

fn snapshot_workspace(heap: &Heap, workspace: Value) -> Result<NotebookSnapshot, String> {
    let id = workspace.as_any_object().ok_or("show: not an object")?;
    let title_sym = heap.find_symbol("title").ok_or("show: no title symbol")?;
    let blocks_sym = heap.find_symbol("blocks").ok_or("show: no blocks symbol")?;

    let title = heap.get(id).slot_get(title_sym)
        .and_then(|v| string_of(heap, v))
        .unwrap_or_else(|| "untitled".into());

    let blocks_val = heap.get(id).slot_get(blocks_sym).unwrap_or(Value::NIL);
    let block_values = heap.list_to_vec(blocks_val);
    let blocks = block_values.iter()
        .map(|b| snapshot_block(heap, *b))
        .collect();

    Ok(NotebookSnapshot { title, blocks })
}

fn snapshot_block(heap: &Heap, block: Value) -> BlockSnapshot {
    let id = match block.as_any_object() {
        Some(id) => id,
        None => return BlockSnapshot::Unknown {
            type_name: "non-object".into(),
            raw: heap.format_value(block),
        },
    };

    let type_name = type_name_of(heap, block);
    match type_name.as_str() {
        "Heading" => BlockSnapshot::Heading {
            text: slot_string(heap, id, "text").unwrap_or_default(),
            level: slot_integer(heap, id, "level").unwrap_or(1),
        },
        "Paragraph" => BlockSnapshot::Paragraph {
            text: slot_string(heap, id, "text").unwrap_or_default(),
        },
        "CodeBlock" => {
            let code = heap.find_symbol("code")
                .and_then(|s| heap.get(id).slot_get(s))
                .map(|v| heap.format_value(v))
                .unwrap_or_default();
            let result_val = heap.find_symbol("result")
                .and_then(|s| heap.get(id).slot_get(s));
            let evaled = result_val.map(|v| !v.is_nil()).unwrap_or(false);
            let result = if evaled {
                result_val.map(|v| heap.format_value(v))
            } else { None };
            BlockSnapshot::Code { code, result, evaled }
        }
        "ObjectRef" => BlockSnapshot::ObjectRef {
            label: slot_string(heap, id, "label").unwrap_or_default(),
            target_desc: heap.find_symbol("target")
                .and_then(|s| heap.get(id).slot_get(s))
                .map(|v| heap.format_value(v))
                .unwrap_or_default(),
        },
        _ => BlockSnapshot::Unknown {
            type_name,
            raw: heap.format_value(block),
        },
    }
}

fn type_name_of(heap: &Heap, val: Value) -> String {
    // Walk the proto chain looking for a stored `__name` slot on the
    // proto object (set by `(def Name { ... })`). Only falls back to
    // variant names if no proto has one.
    if let Some(id) = val.as_any_object() {
        if heap.is_pair(val) { return "Cons".to_string(); }
        if heap.is_text(val) { return "String".to_string(); }
        if heap.is_bytes(val) { return "Bytes".to_string(); }
        if heap.is_table(val) { return "Table".to_string(); }
        if let Some(name_sym) = heap.find_symbol("__name") {
            let mut cur = heap.get(id).proto();
            for _ in 0..32 {
                let Some(pid) = cur.as_any_object() else { break };
                if let Some(n) = heap.get(pid).slot_get(name_sym) {
                    if let Some(s) = n.as_symbol() {
                        return heap.symbol_name(s).to_string();
                    }
                }
                cur = heap.get(pid).proto();
                if cur.is_nil() { break; }
            }
        }
        "Object".to_string()
    } else {
        "primitive".to_string()
    }
}

fn slot_string(heap: &Heap, id: u32, name: &str) -> Option<String> {
    let sym = heap.find_symbol(name)?;
    string_of(heap, heap.get(id).slot_get(sym)?)
}

fn slot_integer(heap: &Heap, id: u32, name: &str) -> Option<i64> {
    let sym = heap.find_symbol(name)?;
    heap.get(id).slot_get(sym)?.as_integer()
}

fn string_of(heap: &Heap, val: Value) -> Option<String> {
    heap.get_string(val.as_any_object()?).map(|s| s.to_string())
}

// ═══════════════════════════════════════════════════════════
// eframe app
// ═══════════════════════════════════════════════════════════

struct NotebookApp { snapshot: NotebookSnapshot }

impl eframe::App for NotebookApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
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
            let size = match *level { 1 => 28.0, 2 => 22.0, 3 => 18.0, _ => 16.0 };
            ui.label(egui::RichText::new(text).size(size).strong());
        }
        BlockSnapshot::Paragraph { text } => { ui.label(text); }
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
            let s = if label.is_empty() { format!("→ {target_desc}") }
                    else { format!("→ {label}: {target_desc}") };
            ui.label(egui::RichText::new(s).italics());
        }
        BlockSnapshot::Unknown { type_name, raw } => {
            ui.label(egui::RichText::new(format!("[{type_name}] {raw}")).weak().monospace());
        }
    }
}

fn run_notebook(snapshot: NotebookSnapshot) -> Result<(), String> {
    let title = format!("moof · {}", snapshot.title);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([700.0, 600.0]),
        ..Default::default()
    };
    eframe::run_native(
        &title,
        options,
        Box::new(|_cc| Ok(Box::new(NotebookApp { snapshot }))),
    ).map_err(|e| format!("gui: {e}"))
}

// ═══════════════════════════════════════════════════════════
// Plugin wiring
// ═══════════════════════════════════════════════════════════

pub struct GuiPlugin;

impl Plugin for GuiPlugin {
    fn name(&self) -> &str { "gui" }

    fn register(&self, heap: &mut Heap) {
        // Create a Gui prototype under Object and bind it as `Gui`.
        let object_proto = heap.type_protos[moof_core::heap::PROTO_OBJ];
        let proto = heap.make_object(object_proto);
        let proto_id = proto.as_any_object().expect("Gui proto");

        let gui_sym = heap.intern("Gui");
        heap.env_def(gui_sym, proto);

        // [Gui show: workspace] — blocks on the eframe event loop.
        native(heap, proto_id, "show:", |heap, _recv, args| {
            let ws = args.first().copied().unwrap_or(Value::NIL);
            let snap = snapshot_workspace(heap, ws)?;
            run_notebook(snap)?;
            Ok(Value::NIL)
        });

        native(heap, proto_id, "describe", |heap, _recv, _args| {
            Ok(heap.alloc_string("<Gui>"))
        });

        let type_sym = heap.intern("Gui");
        native(heap, proto_id, "typeName",
            move |_, _, _| Ok(Value::symbol(type_sym)));
    }
}

#[unsafe(no_mangle)]
pub fn moof_create_type_plugin() -> Box<dyn Plugin> {
    Box::new(GuiPlugin)
}
