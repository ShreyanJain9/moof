/// The MOOF System Browser — Smalltalk-style live object explorer.
///
/// Three-pane layout:
///   Left:   named objects in the environment
///   Middle: slots + handlers of selected object
///   Right:  source code / value detail
///   Bottom: eval bar

use eframe::egui;
use std::sync::{Arc, Mutex};
use crate::runtime::value::{Value, HeapObject};
use crate::runtime::heap::Heap;
use crate::vm::exec::VM;

/// Shared state between the GUI and the VM.
pub struct BrowserState {
    pub vm: VM,
    pub root_env: u32,
}

struct BrowserApp {
    state: Arc<Mutex<BrowserState>>,
    // UI state
    selected_name: Option<String>,
    selected_handler: Option<String>,
    eval_input: String,
    eval_output: String,
    pending_eval: Option<String>,
    filter: String,
}

pub fn run_browser(vm: VM, root_env: u32) {
    let state = Arc::new(Mutex::new(BrowserState { vm, root_env }));

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1200.0, 800.0])
            .with_title("MOOF — System Browser"),
        ..Default::default()
    };

    let state_clone = state.clone();
    let _ = eframe::run_native(
        "MOOF Browser",
        options,
        Box::new(move |_cc| {
            Ok(Box::new(BrowserApp {
                state: state_clone,
                selected_name: None,
                selected_handler: None,
                eval_input: String::new(),
                eval_output: String::new(),
                pending_eval: None,
                filter: String::new(),
            }))
        }),
    );
}

impl eframe::App for BrowserApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let mut state = self.state.lock().unwrap();

        // Process pending eval (before any immutable borrows)
        if let Some(input) = self.pending_eval.take() {
            let root = state.root_env;
            match crate::eval_source(&mut state.vm, root, &input, "<browser>") {
                Ok(val) => self.eval_output = state.vm.format_value(val),
                Err(e) => self.eval_output = format!("!! {}", e),
            }
        }

        // Top bar
        egui::TopBottomPanel::top("top_bar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading("🐄 MOOF System Browser");
                ui.separator();
                ui.label("Filter:");
                ui.text_edit_singleline(&mut self.filter);
            });
        });

        // Bottom eval bar
        egui::TopBottomPanel::bottom("eval_bar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label("moof>");
                let response = ui.text_edit_singleline(&mut self.eval_input);
                if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    if !self.eval_input.is_empty() {
                        self.pending_eval = Some(self.eval_input.clone());
                        self.eval_input.clear();
                    }
                }
                if !self.eval_output.is_empty() {
                    ui.separator();
                    ui.label(format!("=> {}", &self.eval_output));
                }
            });
        });

        // Left panel: named objects
        egui::SidePanel::left("objects_panel")
            .default_width(250.0)
            .show(ctx, |ui| {
                ui.heading("Objects");
                ui.separator();

                let bindings = get_env_bindings(&state.vm.heap, state.root_env);
                egui::ScrollArea::vertical().show(ui, |ui| {
                    for (name, val) in &bindings {
                        if !self.filter.is_empty()
                            && !name.to_lowercase().contains(&self.filter.to_lowercase())
                        {
                            continue;
                        }

                        let type_tag = value_type_tag(&state.vm.heap, *val);
                        let label = format!("{} {}", type_tag, name);
                        let is_selected = self.selected_name.as_deref() == Some(name);

                        if ui.selectable_label(is_selected, &label).clicked() {
                            self.selected_name = Some(name.clone());
                            self.selected_handler = None;
                        }
                    }
                });
            });

        // Right panel: source / detail
        egui::SidePanel::right("source_panel")
            .default_width(400.0)
            .show(ctx, |ui| {
                if let Some(handler_name) = &self.selected_handler {
                    ui.heading(format!("Source: {}", handler_name));
                    ui.separator();

                    if let Some(ref obj_name) = self.selected_name {
                        let val = lookup_binding(&state.vm.heap, state.root_env, obj_name);
                        if let Some(Value::Object(id)) = val {
                            let source = get_handler_source(
                                &state.vm.heap, id, handler_name,
                            );
                            egui::ScrollArea::vertical().show(ui, |ui| {
                                ui.monospace(&source);
                            });
                        }
                    }
                } else {
                    ui.heading("Detail");
                    ui.separator();

                    if let Some(ref name) = self.selected_name {
                        let val = lookup_binding(&state.vm.heap, state.root_env, name);
                        if let Some(val) = val {
                            let detail = format_value_detail(&state.vm, val);
                            egui::ScrollArea::vertical().show(ui, |ui| {
                                ui.monospace(&detail);
                            });
                        }
                    }
                }
            });

        // Center panel: slots + handlers
        egui::CentralPanel::default().show(ctx, |ui| {
            if let Some(ref name) = self.selected_name.clone() {
                let val = lookup_binding(&state.vm.heap, state.root_env, name);

                if let Some(Value::Object(id)) = val {
                    match state.vm.heap.get(id) {
                        HeapObject::GeneralObject {
                            parent,
                            slots,
                            handlers,
                        } => {
                            let parent = *parent;
                            let slots: Vec<_> = slots.clone();
                            let handlers: Vec<_> = handlers.clone();

                            ui.heading(name);
                            ui.label(format!(
                                "parent: {}",
                                state.vm.format_value(parent)
                            ));
                            ui.separator();

                            // Slots
                            if !slots.is_empty() {
                                ui.heading("Slots");
                                egui::Grid::new("slots_grid").striped(true).show(
                                    ui,
                                    |ui| {
                                        for (sym, val) in &slots {
                                            ui.label(
                                                state.vm.heap.symbol_name(*sym),
                                            );
                                            ui.label(
                                                state.vm.format_value(*val),
                                            );
                                            ui.end_row();
                                        }
                                    },
                                );
                                ui.separator();
                            }

                            // Handlers
                            if !handlers.is_empty() {
                                ui.heading("Handlers");
                                egui::ScrollArea::vertical().show(ui, |ui| {
                                    for (sym, handler_val) in &handlers {
                                        let sel_name = state
                                            .vm
                                            .heap
                                            .symbol_name(*sym)
                                            .to_string();
                                        let handler_type =
                                            value_type_tag(&state.vm.heap, *handler_val);
                                        let label =
                                            format!("{} {}", handler_type, sel_name);
                                        let is_selected = self
                                            .selected_handler
                                            .as_deref()
                                            == Some(&sel_name);
                                        if ui
                                            .selectable_label(is_selected, &label)
                                            .clicked()
                                        {
                                            self.selected_handler =
                                                Some(sel_name);
                                        }
                                    }
                                });
                            }
                        }
                        _ => {
                            // Non-object value
                            ui.heading(name);
                            if let Some(val) = val {
                                ui.monospace(format_value_detail(&state.vm, val));
                            }
                        }
                    }
                } else if let Some(val) = val {
                    ui.heading(name);
                    ui.monospace(format_value_detail(&state.vm, val));
                }
            } else {
                ui.centered_and_justified(|ui| {
                    ui.heading("select an object from the left panel");
                });
            }
        });
    }
}

// ── helpers ────────────────────────────────────────────────

fn get_env_bindings(heap: &Heap, env_id: u32) -> Vec<(String, Value)> {
    let mut bindings = Vec::new();
    match heap.get(env_id) {
        HeapObject::Environment(env) => {
            for (&sym, &val) in &env.bindings {
                let name = heap.symbol_name(sym).to_string();
                bindings.push((name, val));
            }
        }
        _ => {}
    }
    bindings.sort_by(|a, b| a.0.cmp(&b.0));
    bindings
}

fn lookup_binding(heap: &Heap, env_id: u32, name: &str) -> Option<Value> {
    match heap.get(env_id) {
        HeapObject::Environment(env) => {
            for (&sym, &val) in &env.bindings {
                if heap.symbol_name(sym) == name {
                    return Some(val);
                }
            }
            None
        }
        _ => None,
    }
}

fn value_type_tag(heap: &Heap, val: Value) -> &'static str {
    match val {
        Value::Nil => "nil",
        Value::True | Value::False => "bool",
        Value::Integer(_) => "int",
        Value::Float(_) => "float",
        Value::Symbol(_) => "sym",
        Value::Object(id) => match heap.get(id) {
            HeapObject::Cons { .. } => "list",
            HeapObject::MoofString(_) => "str",
            HeapObject::GeneralObject { .. } => "obj",
            HeapObject::BytecodeChunk(_) => "code",
            HeapObject::Lambda { .. } => "fn",
            HeapObject::Operative { .. } => "vau",
            HeapObject::Environment(_) => "env",
            HeapObject::NativeFunction { .. } => "native",
        },
    }
}

fn format_value_detail(vm: &VM, val: Value) -> String {
    let mut s = String::new();
    s.push_str(&format!("Type: {:?}\n", val));
    s.push_str(&format!("Display: {}\n", vm.format_value(val)));

    match val {
        Value::Object(id) => match vm.heap.get(id) {
            HeapObject::Lambda { params, source, .. } => {
                s.push_str(&format!("\nParams: {}\n", vm.format_value(*params)));
                s.push_str(&format!("Source: {}\n", vm.format_value(*source)));
            }
            HeapObject::Operative { params, env_param, source, .. } => {
                s.push_str(&format!("\nParams: {}\n", vm.format_value(*params)));
                s.push_str(&format!(
                    "Env param: {}\n",
                    vm.heap.symbol_name(*env_param)
                ));
                s.push_str(&format!("Source: {}\n", vm.format_value(*source)));
            }
            HeapObject::MoofString(str_val) => {
                s.push_str(&format!("\nLength: {}\n", str_val.len()));
                s.push_str(&format!("Content: {}\n", str_val));
            }
            HeapObject::NativeFunction { name } => {
                s.push_str(&format!("\nNative: {}\n", name));
            }
            _ => {}
        },
        _ => {}
    }
    s
}

fn get_handler_source(heap: &Heap, obj_id: u32, handler_name: &str) -> String {
    match heap.get(obj_id) {
        HeapObject::GeneralObject { handlers, .. } => {
            for (sym, handler_val) in handlers {
                if heap.symbol_name(*sym) == handler_name {
                    return format_handler_detail(heap, *handler_val);
                }
            }
            "Handler not found".to_string()
        }
        _ => "Not an object".to_string(),
    }
}

fn format_handler_detail(heap: &Heap, val: Value) -> String {
    match val {
        Value::Object(id) => match heap.get(id) {
            HeapObject::Lambda { params, source, .. } => {
                let params_str = format_cons_for_display(heap, *params);
                let source_str = format_cons_for_display(heap, *source);
                format!("Parameters: {}\n\nSource:\n{}", params_str, source_str)
            }
            HeapObject::NativeFunction { name } => {
                format!("<native {}>\n\nImplemented in Rust.", name)
            }
            _ => format!("{:?}", val),
        },
        _ => format!("{:?}", val),
    }
}

fn format_cons_for_display(heap: &Heap, val: Value) -> String {
    match val {
        Value::Nil => "nil".to_string(),
        Value::True => "true".to_string(),
        Value::False => "false".to_string(),
        Value::Integer(n) => n.to_string(),
        Value::Float(f) => format!("{}", f),
        Value::Symbol(id) => format!("'{}", heap.symbol_name(id)),
        Value::Object(id) => match heap.get(id) {
            HeapObject::Cons { .. } => {
                let mut parts = Vec::new();
                let mut current = val;
                loop {
                    match current {
                        Value::Nil => break,
                        Value::Object(cid) => match heap.get(cid) {
                            HeapObject::Cons { car, cdr } => {
                                parts.push(format_cons_for_display(heap, *car));
                                current = *cdr;
                            }
                            _ => {
                                parts.push(format!(
                                    ". {}",
                                    format_cons_for_display(heap, current)
                                ));
                                break;
                            }
                        },
                        other => {
                            parts.push(format!(
                                ". {}",
                                format_cons_for_display(heap, other)
                            ));
                            break;
                        }
                    }
                }
                format!("({})", parts.join(" "))
            }
            HeapObject::MoofString(s) => format!("\"{}\"", s),
            HeapObject::GeneralObject { .. } => format!("<object #{}>", id),
            HeapObject::Lambda { .. } => "<lambda>".to_string(),
            HeapObject::Operative { .. } => "<operative>".to_string(),
            HeapObject::NativeFunction { name } => format!("<native {}>", name),
            _ => format!("<#{}>", id),
        },
    }
}
