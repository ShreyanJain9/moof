/// The MOOF System Browser — Smalltalk-style live object explorer.
///
/// Four-pane layout + workspace:
///   Left:    named objects in the environment (filterable)
///   Center:  delegation chain, slots, handlers of selected object
///   Right:   source editor for selected handler
///   Bottom:  workspace (multi-line eval with history)

use eframe::egui;
use std::sync::{Arc, Mutex};
use crate::runtime::value::{Value, HeapObject};
use crate::runtime::heap::Heap;
use crate::vm::exec::VM;
// persistence is now directory-based — gui save is a no-op for now

// ── types ──────────────────────────────────────────────────

#[derive(Clone, Debug)]
enum NavTarget { Named(String), HeapId(u32) }

enum PendingAction {
    Eval(String),
    SetSlot { obj_id: u32, sym: u32, expr: String },
    SaveHandler { obj_name: String, selector: String, code: String },
    SaveImage,
}

struct WsEntry { input: String, output: String, is_err: bool }

pub struct BrowserState { pub vm: VM, pub root_env: u32 }

// ── app ────────────────────────────────────────────────────

struct App {
    state: Arc<Mutex<BrowserState>>,
    current: Option<NavTarget>,
    nav_history: Vec<NavTarget>,
    selected_handler: Option<String>,
    editing_slot: Option<(u32, String)>,
    handler_src: String,
    handler_dirty: bool,
    ws_input: String,
    ws_history: Vec<WsEntry>,
    filter: String,
    pending: Vec<PendingAction>,
    status: Option<(String, std::time::Instant)>,
    new_obj_open: bool,
    new_obj_code: String,
    // snapshot caches (rebuilt each frame from heap)
    cached_bindings: Vec<(String, Value)>,
    cached_slots: Vec<(u32, Value, String, String)>, // sym, val, name, display
    cached_handlers: Vec<(u32, Value, String, String)>, // sym, val, name, tag
    cached_chain: Vec<(String, u32)>,
    cached_parent: Value,
    cached_obj_id: Option<u32>,
}

pub fn run_browser(vm: VM, root_env: u32) {
    let state = Arc::new(Mutex::new(BrowserState { vm, root_env }));
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1400.0, 900.0])
            .with_title("MOOF — System Browser"),
        ..Default::default()
    };
    let sc = state.clone();
    let _ = eframe::run_native("MOOF Browser", options, Box::new(move |_| {
        Ok(Box::new(App {
            state: sc, current: None, nav_history: Vec::new(),
            selected_handler: None, editing_slot: None,
            handler_src: String::new(), handler_dirty: false,
            ws_input: String::new(), ws_history: Vec::new(),
            filter: String::new(), pending: Vec::new(),
            status: None, new_obj_open: false,
            new_obj_code: "(def my-obj { Object })".into(),
            cached_bindings: Vec::new(), cached_slots: Vec::new(),
            cached_handlers: Vec::new(), cached_chain: Vec::new(),
            cached_parent: Value::Nil, cached_obj_id: None,
        }))
    }));
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // ── phase 1: process pending mutations ───────────
        {
            let actions = std::mem::take(&mut self.pending);
            let mut st = self.state.lock().unwrap();
            for a in actions {
                match a {
                    PendingAction::Eval(input) => {
                        let r = st.root_env;
                        match crate::eval_source(&mut st.vm, r, &input, "<ws>") {
                            Ok(v) => self.ws_history.push(WsEntry {
                                input, output: st.vm.format_value(v), is_err: false }),
                            Err(e) => self.ws_history.push(WsEntry {
                                input, output: e, is_err: true }),
                        }
                    }
                    PendingAction::SetSlot { obj_id, sym, expr } => {
                        let r = st.root_env;
                        match crate::eval_source(&mut st.vm, r, &expr, "<slot>") {
                            Ok(v) => { st.vm.heap.set_slot(obj_id, sym, v); self.editing_slot = None; }
                            Err(e) => self.status = Some((format!("!! {}", e), std::time::Instant::now())),
                        }
                    }
                    PendingAction::SaveHandler { obj_name, selector, code } => {
                        let expr = format!("(handle! {} [\"{}\" toSymbol] {})", obj_name, selector, code);
                        let r = st.root_env;
                        match crate::eval_source(&mut st.vm, r, &expr, "<handler>") {
                            Ok(_) => {
                                self.handler_dirty = false;
                                self.status = Some(("Handler saved!".into(), std::time::Instant::now()));
                            }
                            Err(e) => self.status = Some((format!("!! {}", e), std::time::Instant::now())),
                        }
                    }
                    PendingAction::SaveImage => {
                        do_save(&st.vm);
                        self.status = Some(("Image saved!".into(), std::time::Instant::now()));
                    }
                }
            }
        }

        // ── phase 2: snapshot heap data into cached fields ──
        {
            let st = self.state.lock().unwrap();
            self.cached_bindings = get_bindings(&st.vm.heap, st.root_env);
            if let Some(id) = resolve_nav(&st.vm.heap, st.root_env, &self.current) {
                self.cached_obj_id = Some(id);
                if let HeapObject::GeneralObject { parent, slots, handlers } = st.vm.heap.get(id) {
                    self.cached_parent = *parent;
                    self.cached_slots = slots.iter().map(|&(s, v)| {
                        let n = st.vm.heap.symbol_name(s).to_string();
                        let d = st.vm.format_value(v);
                        (s, v, n, d)
                    }).collect();
                    self.cached_handlers = handlers.iter().map(|&(s, v)| {
                        let n = st.vm.heap.symbol_name(s).to_string();
                        let t = type_tag(&st.vm.heap, v).to_string();
                        (s, v, n, t)
                    }).collect();
                    self.cached_chain = get_chain(&st.vm.heap, id, st.root_env);
                } else {
                    self.cached_slots.clear();
                    self.cached_handlers.clear();
                    self.cached_chain.clear();
                }
            } else {
                self.cached_obj_id = None;
                self.cached_slots.clear();
                self.cached_handlers.clear();
                self.cached_chain.clear();
            }

            // Initialize handler source when first selected
            if let Some(ref sel) = self.selected_handler {
              if !self.handler_dirty {
                if self.handler_src.is_empty() {
                    if let Some(id) = self.cached_obj_id {
                        self.handler_src = get_handler_src(&st.vm.heap, id, sel);
                    }
                }
              }
            }
        }

        // ── phase 3: render UI (no heap borrows) ────────────

        // Top bar
        egui::TopBottomPanel::top("top").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading(egui::RichText::new("MOOF").strong());
                ui.separator();
                if ui.add_enabled(!self.nav_history.is_empty(), egui::Button::new("< Back")).clicked() {
                    if let Some(p) = self.nav_history.pop() {
                        self.current = Some(p);
                        self.selected_handler = None;
                        self.editing_slot = None;
                        self.handler_src.clear();
                    }
                }
                ui.separator();
                ui.label("Filter:");
                ui.add(egui::TextEdit::singleline(&mut self.filter).desired_width(120.0));
                ui.separator();
                if ui.button("New Object").clicked() { self.new_obj_open = !self.new_obj_open; }
                if ui.button("Save Image").clicked() { self.pending.push(PendingAction::SaveImage); }
                if let Some((ref msg, when)) = self.status {
                    if when.elapsed().as_secs() < 3 {
                        ui.separator();
                        ui.label(egui::RichText::new(msg).italics());
                    }
                }
            });
            if self.new_obj_open {
                ui.horizontal(|ui| {
                    ui.text_edit_singleline(&mut self.new_obj_code);
                    if ui.button("Create").clicked() {
                        self.pending.push(PendingAction::Eval(self.new_obj_code.clone()));
                        self.new_obj_open = false;
                    }
                });
            }
        });

        // Workspace
        egui::TopBottomPanel::bottom("ws").default_height(180.0).resizable(true).show(ctx, |ui| {
            ui.heading(egui::RichText::new("Workspace").size(13.0));
            egui::ScrollArea::vertical().max_height(ui.available_height() - 28.0).stick_to_bottom(true).show(ui, |ui| {
                for e in &self.ws_history {
                    ui.label(egui::RichText::new(format!("moof> {}", &e.input)).monospace().color(egui::Color32::GRAY));
                    let c = if e.is_err { egui::Color32::from_rgb(220,60,60) } else { egui::Color32::from_rgb(80,180,80) };
                    ui.label(egui::RichText::new(format!("=> {}", &e.output)).monospace().color(c));
                }
            });
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("moof>").monospace());
                let r = ui.add(egui::TextEdit::singleline(&mut self.ws_input).font(egui::TextStyle::Monospace).desired_width(ui.available_width() - 50.0));
                if (r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))) || ui.button("Run").clicked() {
                    if !self.ws_input.is_empty() {
                        self.pending.push(PendingAction::Eval(self.ws_input.clone()));
                        self.ws_input.clear();
                    }
                }
            });
        });

        // Left panel: objects
        egui::SidePanel::left("objs").default_width(200.0).resizable(true).show(ctx, |ui| {
            ui.heading(egui::RichText::new("Objects").size(13.0));
            ui.separator();
            egui::ScrollArea::vertical().show(ui, |ui| {
                for (name, val) in &self.cached_bindings {
                    if !self.filter.is_empty() && !name.to_lowercase().contains(&self.filter.to_lowercase()) { continue; }
                    let tag = type_tag_val(*val);
                    let is_sel = matches!(&self.current, Some(NavTarget::Named(n)) if n == name);
                    let label = egui::RichText::new(format!("{:<6} {}", tag, name)).monospace().color(tag_color(tag));
                    if ui.selectable_label(is_sel, label).clicked() {
                        if let Some(old) = self.current.take() { self.nav_history.push(old); }
                        self.current = Some(NavTarget::Named(name.clone()));
                        self.selected_handler = None;
                        self.editing_slot = None;
                        self.handler_src.clear();
                        self.handler_dirty = false;
                    }
                }
            });
        });

        // Right panel: source editor
        egui::SidePanel::right("src").default_width(380.0).resizable(true).show(ctx, |ui| {
            if let Some(ref sel) = self.selected_handler.clone() {
                ui.heading(egui::RichText::new(format!("Handler: {}", sel)).size(13.0));
                ui.separator();
                egui::ScrollArea::vertical().show(ui, |ui| {
                    let r = ui.add(egui::TextEdit::multiline(&mut self.handler_src).font(egui::TextStyle::Monospace).desired_width(f32::INFINITY).desired_rows(20));
                    if r.changed() { self.handler_dirty = true; }
                });
                if self.handler_dirty {
                    ui.horizontal(|ui| {
                        if ui.button("Save Handler").clicked() {
                            if let Some(NavTarget::Named(ref n)) = self.current {
                                self.pending.push(PendingAction::SaveHandler {
                                    obj_name: n.clone(), selector: sel.clone(), code: self.handler_src.clone() });
                            }
                        }
                        if ui.button("Revert").clicked() { self.handler_src.clear(); self.handler_dirty = false; }
                    });
                }
            } else {
                ui.heading(egui::RichText::new("Detail").size(13.0));
                ui.separator();
                if self.current.is_some() {
                    ui.label(egui::RichText::new("Select a handler to view source").italics());
                }
            }
        });

        // Center panel: object detail
        egui::CentralPanel::default().show(ctx, |ui| {
            if self.current.is_none() {
                ui.centered_and_justified(|ui| { ui.heading("Select an object from the left panel"); });
                return;
            }

            let title = match &self.current {
                Some(NavTarget::Named(n)) => n.clone(),
                Some(NavTarget::HeapId(id)) => format!("object #{}", id),
                None => return,
            };

            ui.heading(egui::RichText::new(&title).size(18.0).strong());

            // Delegation chain
            if !self.cached_chain.is_empty() {
                ui.horizontal(|ui| {
                    for (i, (name, cid)) in self.cached_chain.iter().enumerate() {
                        if i > 0 { ui.label("->"); }
                        if Some(*cid) == self.cached_obj_id {
                            ui.label(egui::RichText::new(name).strong());
                        } else if ui.link(name).clicked() {
                            if let Some(old) = self.current.take() { self.nav_history.push(old); }
                            self.current = Some(NavTarget::Named(name.clone()));
                            self.selected_handler = None;
                            self.handler_src.clear();
                        }
                    }
                });
            }
            ui.separator();

            // Slots
            if !self.cached_slots.is_empty() {
                ui.heading(egui::RichText::new("Slots").size(14.0));
                let slots = self.cached_slots.clone();
                egui::Grid::new("slots").striped(true).show(ui, |ui| {
                    for (sym, val, name, display) in &slots {
                        ui.label(egui::RichText::new(name).strong().monospace());
                        if self.editing_slot.as_ref().map(|(s,_)| *s) == Some(*sym) {
                            let text = &mut self.editing_slot.as_mut().unwrap().1;
                            let r = ui.text_edit_singleline(text);
                            if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                                if let Some(oid) = self.cached_obj_id {
                                    self.pending.push(PendingAction::SetSlot { obj_id: oid, sym: *sym, expr: text.clone() });
                                }
                            }
                            if ui.small_button("x").clicked() { self.editing_slot = None; }
                        } else {
                            if val.as_object().is_some() {
                                if ui.link(egui::RichText::new(display).monospace()).clicked() {
                                    if let Some(old) = self.current.take() { self.nav_history.push(old); }
                                    self.current = Some(NavTarget::HeapId(val.as_object().unwrap()));
                                    self.selected_handler = None;
                                    self.handler_src.clear();
                                }
                            } else {
                                ui.label(egui::RichText::new(display).monospace());
                            }
                            if ui.small_button("edit").clicked() {
                                self.editing_slot = Some((*sym, display.clone()));
                            }
                        }
                        ui.end_row();
                    }
                });
                ui.separator();
            }

            // Handlers
            if !self.cached_handlers.is_empty() {
                ui.heading(egui::RichText::new("Handlers").size(14.0));
                egui::ScrollArea::vertical().show(ui, |ui| {
                    let handlers = self.cached_handlers.clone();
                    for (_sym, _val, name, tag) in &handlers {
                        let color = tag_color(tag);
                        let label = egui::RichText::new(format!("{:<6} {}", tag, name)).monospace().color(color);
                        let is_sel = self.selected_handler.as_deref() == Some(name.as_str());
                        if ui.selectable_label(is_sel, label).clicked() {
                            self.selected_handler = Some(name.clone());
                            self.handler_src.clear();
                            self.handler_dirty = false;
                        }
                    }
                });
            }
        });
    }
}

// ── helpers (pure, no self borrows) ────────────────────────

fn do_save(_vm: &VM) {
    // Image saving is now directory-based — handled by ModuleLoader
}

fn get_bindings(heap: &Heap, env_id: u32) -> Vec<(String, Value)> {
    let mut b = Vec::new();
    if let HeapObject::Environment(env) = heap.get(env_id) {
        for (&s, &v) in &env.bindings {
            let n = heap.symbol_name(s).to_string();
            if !n.starts_with('*') && !n.starts_with('%') && !n.starts_with('$') { b.push((n, v)); }
        }
    }
    b.sort_by(|a, b| a.0.cmp(&b.0));
    b
}

fn resolve_nav(heap: &Heap, env_id: u32, nav: &Option<NavTarget>) -> Option<u32> {
    match nav {
        Some(NavTarget::Named(name)) => {
            if let HeapObject::Environment(env) = heap.get(env_id) {
                for (&s, &v) in &env.bindings { if heap.symbol_name(s) == name { return v.as_object(); } }
            }
            None
        }
        Some(NavTarget::HeapId(id)) => Some(*id),
        None => None,
    }
}

fn get_chain(heap: &Heap, obj_id: u32, env_id: u32) -> Vec<(String, u32)> {
    let mut chain = Vec::new();
    let mut cur = Some(obj_id);
    let mut seen = std::collections::HashSet::new();
    while let Some(id) = cur {
        if !seen.insert(id) { break; }
        let name = rev_lookup(heap, env_id, id);
        chain.push((name, id));
        cur = match heap.get(id) {
            HeapObject::GeneralObject { parent, .. } => parent.as_object(),
            _ => None,
        };
    }
    chain.reverse();
    chain
}

fn rev_lookup(heap: &Heap, env_id: u32, obj_id: u32) -> String {
    if let HeapObject::Environment(env) = heap.get(env_id) {
        for (&s, &v) in &env.bindings {
            if v == Value::Object(obj_id) {
                let n = heap.symbol_name(s).to_string();
                if !n.starts_with('*') && !n.starts_with('%') { return n; }
            }
        }
    }
    format!("#{}", obj_id)
}

fn get_handler_src(heap: &Heap, obj_id: u32, sel: &str) -> String {
    if let HeapObject::GeneralObject { handlers, .. } = heap.get(obj_id) {
        for (s, v) in handlers {
            if heap.symbol_name(*s) == sel {
                return match *v {
                    Value::Object(hid) => match heap.get(hid) {
                        HeapObject::Lambda { source, .. } => fmt_sexp(heap, *source, 0),
                        HeapObject::NativeFunction { name } => format!("; <native {}>\n; Implemented in Rust", name),
                        _ => "; unknown handler type".into(),
                    },
                    _ => "; non-object handler".into(),
                };
            }
        }
    }
    "; handler not found".into()
}

fn type_tag(heap: &Heap, val: Value) -> &'static str {
    match val {
        Value::Nil => "nil", Value::True | Value::False => "bool",
        Value::Integer(_) => "int", Value::Float(_) => "float", Value::Symbol(_) => "sym",
        Value::Object(id) => match heap.get(id) {
            HeapObject::Cons { .. } => "list", HeapObject::MoofString(_) => "str",
            HeapObject::GeneralObject { .. } => "obj", HeapObject::BytecodeChunk(_) => "code",
            HeapObject::Lambda { .. } => "fn", HeapObject::Operative { .. } => "vau",
            HeapObject::Environment(_) => "env", HeapObject::NativeFunction { .. } => "native",
        },
    }
}

fn type_tag_val(val: Value) -> &'static str {
    match val {
        Value::Nil => "nil", Value::True | Value::False => "bool",
        Value::Integer(_) => "int", Value::Float(_) => "float",
        Value::Symbol(_) => "sym", Value::Object(_) => "obj",
    }
}

fn tag_color(tag: &str) -> egui::Color32 {
    match tag {
        "obj" => egui::Color32::from_rgb(100,149,237),
        "fn" => egui::Color32::from_rgb(80,180,80),
        "vau" => egui::Color32::from_rgb(180,80,180),
        "native" => egui::Color32::from_rgb(220,160,50),
        "str" => egui::Color32::from_rgb(180,130,80),
        "int" | "float" => egui::Color32::from_rgb(160,160,160),
        "sym" => egui::Color32::from_rgb(160,100,200),
        "list" => egui::Color32::from_rgb(120,160,120),
        _ => egui::Color32::GRAY,
    }
}

fn fmt_sexp(heap: &Heap, val: Value, ind: usize) -> String {
    match val {
        Value::Nil => "nil".into(), Value::True => "true".into(), Value::False => "false".into(),
        Value::Integer(n) => n.to_string(), Value::Float(f) => format!("{}", f),
        Value::Symbol(id) => heap.symbol_name(id).to_string(),
        Value::Object(id) => match heap.get(id) {
            HeapObject::Cons { .. } => {
                let elems = heap.list_to_vec(val);
                if elems.is_empty() { return "()".into(); }
                let short = elems.len() <= 5 && elems.iter().all(|e| sexp_len(heap, *e) < 20);
                if short {
                    let parts: Vec<String> = elems.iter().map(|e| fmt_sexp(heap, *e, ind+2)).collect();
                    format!("({})", parts.join(" "))
                } else {
                    let pad = " ".repeat(ind + 1);
                    let mut parts = Vec::new();
                    for (i, e) in elems.iter().enumerate() {
                        let s = fmt_sexp(heap, *e, ind + 1);
                        if i == 0 { parts.push(s); } else { parts.push(format!("{}{}", pad, s)); }
                    }
                    format!("({})", parts.join("\n"))
                }
            }
            HeapObject::MoofString(s) => format!("\"{}\"", s),
            _ => format!("<#{}>", id),
        },
    }
}

fn sexp_len(heap: &Heap, val: Value) -> usize {
    match val {
        Value::Object(id) => match heap.get(id) {
            HeapObject::Cons { .. } => heap.list_to_vec(val).iter().map(|e| sexp_len(heap, *e) + 1).sum(),
            HeapObject::MoofString(s) => s.len() + 2,
            _ => 10,
        },
        Value::Symbol(id) => heap.symbol_name(id).len(),
        _ => 5,
    }
}
