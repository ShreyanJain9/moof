//! the substrate's primordial native methods + global bindings.
//!
//! installed during `World::new()`, before any moof source loads.
//! covers exactly what's needed for the phase-A forcing function:
//!
//! - `:call` on `Method` (covers Closure, all method-Forms).
//! - arithmetic + comparison on `Integer` (`:+`, `:-`, `:*`, `:/`,
//!   `:<`, `:>`, `:<=`, `:>=`, `:=`, `:!=`).
//! - structural ops: `:head`, `:tail`, `:cons:`, `:null?` on `List`.
//! - identity / equality on `Object`, `Symbol`, `Bool`, `Nil`.
//! - reflection on `Object`: `:proto`, `:slots`, `:handlers`,
//!   `:meta`, `:source`, `:identity`, `:=`, `:is`, `:to-string`,
//!   `:inspect`, `:new`, `:does-not-understand:with:`.
//! - global callables that forward to receiver methods: `+`, `-`,
//!   `*`, `/`, `<`, `>`, `<=`, `>=`, `=`, `!=`, `head`, `tail`,
//!   `cons`, `null?`, `list?`, `not`.
//!
//! everything else — `length`, `map`, `filter`, the protocol
//! framework — lives in moof code at phase A.10.

use crate::form::Form;
use crate::sym::SymId;
use crate::value::Value;
use crate::world::{NativeFn, RaiseError, World};

/// install all phase-A intrinsics. idempotent: safe to call once
/// at world init.
pub fn install(w: &mut World) {
    install_call_on_method(w);
    install_integer_methods(w);
    install_symbol_methods(w);
    install_bool_methods(w);
    install_nil_methods(w);
    install_object_reflection(w);
    install_list_methods(w);
    install_method_methods(w);
    install_console_proto_and_caps(w);
    install_globals(w);
    install_proto_globals(w);
}

/// :to-string on Method (covers Closure too). renders the source
/// if available, else `<closure>` / `<method>`.
fn install_method_methods(w: &mut World) {
    w.install_native(w.protos.method, "to-string", |w, self_, _| {
        let id = match self_.as_form_id() {
            Some(id) => id,
            None => {
                return Ok(Value::Sym(w.intern("<method>")));
            }
        };
        let source = w.heap.get(id).meta_at(w.source_sym);
        if source.is_nil() {
            return Ok(Value::Sym(w.intern("<closure>")));
        }
        // try rendering source. if it's a list, recursive print;
        // if a sym (a method name placeholder), render directly.
        match source {
            Value::Sym(s) => {
                let text = format!("<method:{}>", w.resolve(s));
                Ok(Value::Sym(w.intern(&text)))
            }
            Value::Form(_) => {
                // a parsed code-form (a list).
                let inner = render_list_to_string(w, source)?;
                let text = format!("<closure source: {}>", inner);
                Ok(Value::Sym(w.intern(&text)))
            }
            _ => Ok(Value::Sym(w.intern("<closure>"))),
        }
    });
}

/// expose the canonical protos as moof globals (`Object`, `List`,
/// `Integer`, …). user code can refer to them by name to install
/// handlers, allocate instances, and inspect the proto chain.
fn install_proto_globals(w: &mut World) {
    let bindings = [
        ("Object", w.protos.object),
        ("Nil-proto", w.protos.nil),
        ("Bool", w.protos.bool_),
        ("Integer", w.protos.integer),
        ("Symbol", w.protos.symbol),
        ("List", w.protos.list),
        ("Method", w.protos.method),
        ("Chunk", w.protos.chunk),
        ("Closure", w.protos.closure),
        ("Env", w.protos.env),
        ("ForeignHandle", w.protos.foreign),
    ];
    let global = w.global_env;
    for (name, id) in bindings {
        let s = w.intern(name);
        w.env_bind(global, s, Value::Form(id));
    }
}

// ─────────────────────────────────────────────────────────────────
// :call on Method (so Closures + plain method-Forms are callable)
// ─────────────────────────────────────────────────────────────────

fn install_call_on_method(w: &mut World) {
    // [m call: arg…] ≡ world.invoke(m, captured-self-or-nil, args).
    // for a closure created inside a method body, the captured self
    // is in the closure's :captured-self slot (set by PushClosure).
    // for a standalone method-Form, no captured self exists; pass nil.
    w.install_native(w.protos.method, "call", |world, self_, args| {
        let id = self_.as_form_id().ok_or_else(|| {
            RaiseError::new(world.intern("dispatch"), "receiver of :call is not a Form")
        })?;
        let captured_sym = world.intern("captured-self");
        let captured = world.heap.get(id).slot(captured_sym);
        world.invoke(id, captured, args)
    });
}

// ─────────────────────────────────────────────────────────────────
// Integer methods
// ─────────────────────────────────────────────────────────────────

fn install_integer_methods(w: &mut World) {
    w.install_native(w.protos.integer, "+", |w, self_, args| {
        let a = int_arg(w, self_, "+")?;
        let b = int_arg(w, args[0], "+")?;
        Ok(Value::Int(a.wrapping_add(b)))
    });
    w.install_native(w.protos.integer, "-", |w, self_, args| {
        let a = int_arg(w, self_, "-")?;
        let b = int_arg(w, args[0], "-")?;
        Ok(Value::Int(a.wrapping_sub(b)))
    });
    w.install_native(w.protos.integer, "*", |w, self_, args| {
        let a = int_arg(w, self_, "*")?;
        let b = int_arg(w, args[0], "*")?;
        Ok(Value::Int(a.wrapping_mul(b)))
    });
    w.install_native(w.protos.integer, "/", |w, self_, args| {
        let a = int_arg(w, self_, "/")?;
        let b = int_arg(w, args[0], "/")?;
        if b == 0 {
            return Err(RaiseError::new(
                w.intern("division-by-zero"),
                "integer division by zero",
            ));
        }
        // moof Integer division returns the integer quotient at
        // phase A; later phases may promote to Rational
        // (`docs/concepts/numbers.md`) but the seed keeps it tight.
        Ok(Value::Int(a.wrapping_div(b)))
    });
    w.install_native(w.protos.integer, "=", |w, self_, args| {
        let a = int_arg(w, self_, "=")?;
        match args[0] {
            Value::Int(b) => Ok(Value::Bool(a == b)),
            _ => Ok(Value::Bool(false)),
        }
    });
    w.install_native(w.protos.integer, "!=", |w, self_, args| {
        let a = int_arg(w, self_, "!=")?;
        match args[0] {
            Value::Int(b) => Ok(Value::Bool(a != b)),
            _ => Ok(Value::Bool(true)),
        }
    });
    w.install_native(w.protos.integer, "<", |w, self_, args| {
        let a = int_arg(w, self_, "<")?;
        let b = int_arg(w, args[0], "<")?;
        Ok(Value::Bool(a < b))
    });
    w.install_native(w.protos.integer, ">", |w, self_, args| {
        let a = int_arg(w, self_, ">")?;
        let b = int_arg(w, args[0], ">")?;
        Ok(Value::Bool(a > b))
    });
    w.install_native(w.protos.integer, "<=", |w, self_, args| {
        let a = int_arg(w, self_, "<=")?;
        let b = int_arg(w, args[0], "<=")?;
        Ok(Value::Bool(a <= b))
    });
    w.install_native(w.protos.integer, ">=", |w, self_, args| {
        let a = int_arg(w, self_, ">=")?;
        let b = int_arg(w, args[0], ">=")?;
        Ok(Value::Bool(a >= b))
    });
    w.install_native(w.protos.integer, "to-string", |w, self_, _args| {
        let a = int_arg(w, self_, "to-string")?;
        let s = w.intern(&a.to_string());
        Ok(Value::Sym(s))
    });
}

fn int_arg(w: &mut World, v: Value, op: &str) -> Result<i64, RaiseError> {
    v.as_int().ok_or_else(|| {
        RaiseError::new(
            w.intern("type-error"),
            format!("{} expected an Integer", op),
        )
    })
}

// ─────────────────────────────────────────────────────────────────
// Symbol / Bool / Nil — minimum equality story
// ─────────────────────────────────────────────────────────────────

fn install_symbol_methods(w: &mut World) {
    w.install_native(w.protos.symbol, "=", |_, self_, args| {
        Ok(Value::Bool(self_ == args[0]))
    });
    w.install_native(w.protos.symbol, "!=", |_, self_, args| {
        Ok(Value::Bool(self_ != args[0]))
    });
    w.install_native(w.protos.symbol, "to-string", |w, self_, _| {
        let s = self_.as_sym().ok_or_else(|| {
            RaiseError::new(w.intern("type-error"), "to-string on non-Symbol")
        })?;
        // Symbol's :to-string is identity — the symbol *is* its
        // textual rendering at phase A (no String type yet).
        let _ = w;
        Ok(Value::Sym(s))
    });
}

fn install_bool_methods(w: &mut World) {
    w.install_native(w.protos.bool_, "=", |_, self_, args| {
        Ok(Value::Bool(self_ == args[0]))
    });
    w.install_native(w.protos.bool_, "!=", |_, self_, args| {
        Ok(Value::Bool(self_ != args[0]))
    });
    w.install_native(w.protos.bool_, "not", |_, self_, _args| match self_ {
        Value::Bool(b) => Ok(Value::Bool(!b)),
        _ => Ok(Value::Bool(false)), // shouldn't happen if dispatch is right
    });
    w.install_native(w.protos.bool_, "to-string", |w, self_, _| match self_ {
        Value::Bool(true) => Ok(Value::Sym(w.intern("#true"))),
        Value::Bool(false) => Ok(Value::Sym(w.intern("#false"))),
        _ => Err(RaiseError::new(
            w.intern("type-error"),
            "to-string on non-Bool",
        )),
    });
}

fn install_nil_methods(w: &mut World) {
    w.install_native(w.protos.nil, "=", |_, self_, args| {
        Ok(Value::Bool(self_ == args[0]))
    });
    w.install_native(w.protos.nil, "!=", |_, self_, args| {
        Ok(Value::Bool(self_ != args[0]))
    });
    w.install_native(w.protos.nil, "to-string", |w, _, _| {
        Ok(Value::Sym(w.intern("nil")))
    });
    w.install_native(w.protos.nil, "head", |w, _, _| {
        // (head nil) → nil. lispy convention; users beware.
        let _ = w;
        Ok(Value::Nil)
    });
    w.install_native(w.protos.nil, "tail", |w, _, _| {
        let _ = w;
        Ok(Value::Nil)
    });
    w.install_native(w.protos.nil, "null?", |_, _, _| Ok(Value::Bool(true)));
    // (cons h ()) — nil is the empty list, so consing onto it
    // builds a one-element list. without this, `(map …)` and
    // friends fall over at the recursion base case.
    w.install_native(w.protos.nil, "cons:", |w, self_, args| {
        let head_sym = w.head_sym;
        let tail_sym = w.tail_sym;
        let mut cell = Form::with_proto(Value::Form(w.protos.list));
        cell.slots.insert(head_sym, args[0]);
        cell.slots.insert(tail_sym, self_);
        let id = w.alloc(cell);
        Ok(Value::Form(id))
    });
}

// ─────────────────────────────────────────────────────────────────
// List (cons-cell) methods
// ─────────────────────────────────────────────────────────────────

fn install_list_methods(w: &mut World) {
    // we read the head/tail SymIds from the world *inside* each
    // native (they're already cached on `World`, not allocated per
    // call). this lets the closures be `fn` pointers (no capture).
    w.install_native(w.protos.list, "head", |w, self_, _| {
        let id = self_.as_form_id().ok_or_else(|| {
            RaiseError::new(w.intern("type-error"), "head on non-List")
        })?;
        let head_sym = w.head_sym;
        Ok(w.heap.get(id).slot(head_sym))
    });
    w.install_native(w.protos.list, "tail", |w, self_, _| {
        let id = self_.as_form_id().ok_or_else(|| {
            RaiseError::new(w.intern("type-error"), "tail on non-List")
        })?;
        let tail_sym = w.tail_sym;
        Ok(w.heap.get(id).slot(tail_sym))
    });
    w.install_native(w.protos.list, "null?", |_, _, _| Ok(Value::Bool(false)));
    w.install_native(w.protos.list, "cons:", |w, self_, args| {
        let head_sym = w.head_sym;
        let tail_sym = w.tail_sym;
        let mut cell = Form::with_proto(Value::Form(w.protos.list));
        cell.slots.insert(head_sym, args[0]);
        cell.slots.insert(tail_sym, self_);
        let id = w.alloc(cell);
        Ok(Value::Form(id))
    });
    // List :to-string — recursive `(elem1 elem2 ...)` rendering.
    w.install_native(w.protos.list, "to-string", |w, self_, _| {
        let s = render_list_to_string(w, self_)?;
        Ok(Value::Sym(w.intern(&s)))
    });
}

/// recursive list-to-string. each element's :to-string is sent;
/// joins with spaces, wraps in parens.
fn render_list_to_string(w: &mut World, list: Value) -> Result<String, RaiseError> {
    let mut out = String::from("(");
    let mut cur = list;
    let mut first = true;
    let to_string = w.intern("to-string");
    let head_sym = w.head_sym;
    let tail_sym = w.tail_sym;
    loop {
        match cur {
            Value::Nil => break,
            Value::Form(id) => {
                if !first {
                    out.push(' ');
                }
                first = false;
                let head = w.heap.get(id).slot(head_sym);
                let tail = w.heap.get(id).slot(tail_sym);
                let head_str_v = w.send(head, to_string, &[])?;
                let head_str_sym = head_str_v.as_sym().ok_or_else(|| {
                    RaiseError::new(
                        w.intern("type-error"),
                        ":to-string returned non-symbol",
                    )
                })?;
                out.push_str(w.resolve(head_str_sym));
                cur = tail;
            }
            _ => {
                // improper list — should be rare in moof; show as
                // `(... . tail)`.
                if !first {
                    out.push(' ');
                }
                out.push_str(". ");
                let tail_str_v = w.send(cur, to_string, &[])?;
                let tail_str_sym = tail_str_v.as_sym().ok_or_else(|| {
                    RaiseError::new(
                        w.intern("type-error"),
                        ":to-string returned non-symbol",
                    )
                })?;
                out.push_str(w.resolve(tail_str_sym));
                break;
            }
        }
    }
    out.push(')');
    Ok(out)
}

// ─────────────────────────────────────────────────────────────────
// Object reflection — the load-bearing moldable promise (L6)
// ─────────────────────────────────────────────────────────────────

fn install_object_reflection(w: &mut World) {
    w.install_native(w.protos.object, "proto", |w, self_, _| {
        Ok(w.proto_of(self_))
    });

    w.install_native(w.protos.object, "slots", |w, self_, _| {
        // returns a moof list of (sym . value) pairs. for tagged
        // immediates with no slots, returns nil.
        match self_ {
            Value::Form(id) => {
                // collect (head, tail) cons cells from the slots
                // table, in insertion order.
                let f = w.heap.get(id);
                let pairs: Vec<(SymId, Value)> = f
                    .slots
                    .iter()
                    .map(|(k, v)| (*k, *v))
                    .collect();
                let mut entries = Vec::with_capacity(pairs.len());
                for (k, v) in pairs {
                    let pair = w.make_list(&[Value::Sym(k), v]);
                    entries.push(pair);
                }
                Ok(w.make_list(&entries))
            }
            _ => Ok(Value::Nil),
        }
    });

    w.install_native(w.protos.object, "handlers", |w, self_, _| {
        // returns a moof list of (selector . method-Form) pairs from
        // *this proto* (not the inherited chain). reading inherited
        // handlers is the user's job (walk via :proto).
        match self_ {
            Value::Form(id) => {
                let pairs: Vec<(SymId, Value)> = w
                    .heap
                    .get(id)
                    .handlers
                    .iter()
                    .map(|(k, v)| (*k, *v))
                    .collect();
                let mut entries = Vec::with_capacity(pairs.len());
                for (k, v) in pairs {
                    let pair = w.make_list(&[Value::Sym(k), v]);
                    entries.push(pair);
                }
                Ok(w.make_list(&entries))
            }
            _ => Ok(Value::Nil),
        }
    });

    w.install_native(w.protos.object, "meta", |w, self_, _| {
        match self_ {
            Value::Form(id) => {
                let pairs: Vec<(SymId, Value)> = w
                    .heap
                    .get(id)
                    .meta
                    .iter()
                    .map(|(k, v)| (*k, *v))
                    .collect();
                let mut entries = Vec::with_capacity(pairs.len());
                for (k, v) in pairs {
                    let pair = w.make_list(&[Value::Sym(k), v]);
                    entries.push(pair);
                }
                Ok(w.make_list(&entries))
            }
            _ => Ok(Value::Nil),
        }
    });

    w.install_native(w.protos.object, "source", |w, self_, _| match self_ {
        Value::Form(id) => Ok(w.heap.get(id).meta_at(w.source_sym)),
        _ => Ok(Value::Nil),
    });

    w.install_native(w.protos.object, "identity", |_, self_, _| match self_ {
        Value::Form(id) => Ok(Value::Int(id.0 as i64)),
        // tagged-immediates report identity = 0 (no heap slot).
        _ => Ok(Value::Int(0)),
    });

    w.install_native(w.protos.object, "is", |_, self_, args| {
        // identity equality (same heap-id or same tagged-immediate).
        Ok(Value::Bool(self_ == args[0]))
    });

    // Object's `:=` is identity equality by default. specific protos
    // (Integer, Symbol, etc.) override with structural equality.
    w.install_native(w.protos.object, "=", |_, self_, args| {
        Ok(Value::Bool(self_ == args[0]))
    });

    w.install_native(w.protos.object, "!=", |_, self_, args| {
        Ok(Value::Bool(self_ != args[0]))
    });

    w.install_native(w.protos.object, "to-string", |w, self_, _| {
        // default rendering: `<Form#N>` for heap forms; tagged
        // immediates get their own to-string overrides on Integer/
        // Symbol/Bool/Nil already.
        match self_ {
            Value::Form(id) => Ok(Value::Sym(w.intern(&format!("<Form#{}>", id.0)))),
            Value::Foreign(id) => Ok(Value::Sym(w.intern(&format!("<Foreign#{}>", id.0)))),
            Value::Nil => Ok(Value::Sym(w.intern("nil"))),
            // these shouldn't be reached because each tagged kind
            // overrides :to-string. defensive fallback:
            Value::Bool(b) => Ok(Value::Sym(w.intern(if b { "#true" } else { "#false" }))),
            Value::Int(n) => Ok(Value::Sym(w.intern(&n.to_string()))),
            Value::Sym(s) => Ok(Value::Sym(s)), // identity for symbols
        }
    });

    w.install_native(w.protos.object, "inspect", |w, self_, _| {
        // phase A: same as :to-string. phase C swaps in a richer
        // moof-side Inspector view.
        let to_string = w.intern("to-string");
        w.send(self_, to_string, &[])
    });

    w.install_native(w.protos.object, "new", |w, self_, _| {
        // (Proto :new) → fresh instance.
        let proto_id = self_.as_form_id().ok_or_else(|| {
            RaiseError::new(w.intern("type-error"), ":new on non-Form proto")
        })?;
        let f = Form::with_proto(Value::Form(proto_id));
        let id = w.alloc(f);
        Ok(Value::Form(id))
    });

    // default does-not-understand:with: raises. user code can
    // override on any proto.
    w.install_native(
        w.protos.object,
        "does-not-understand:with:",
        |w, self_, args| {
            let sel = args[0].as_sym().unwrap_or(SymId::NONE);
            let kind = w.intern("does-not-understand");
            Err(RaiseError::new(
                kind,
                format!(
                    "{} does not understand `{}`",
                    fmt_short(w, self_),
                    if sel.is_none() { "<unknown>" } else { w.resolve(sel) }
                ),
            ))
        },
    );
}

fn fmt_short(w: &World, v: Value) -> String {
    match v {
        Value::Nil => "nil".into(),
        Value::Bool(true) => "#true".into(),
        Value::Bool(false) => "#false".into(),
        Value::Int(n) => n.to_string(),
        Value::Sym(s) => format!("'{}", w.resolve(s)),
        Value::Form(id) => format!("<Form#{}>", id.0),
        Value::Foreign(id) => format!("<Foreign#{}>", id.0),
    }
}

// ─────────────────────────────────────────────────────────────────
// Console proto + $out / $err caps
//
// per `process/docs-driven.md`'s capability rule, there is *no*
// path to stdout from moof code that isn't through a cap. the
// supervisor (in phase A: the substrate seed itself) constructs
// the primordial $out and $err caps at boot and binds them in the
// global env.
//
// phase A's Console is bare rust stdout/stderr — a placeholder for
// the proper `os/console.mco` that lands in phase B alongside the
// mco loader. the moof interface (`[$out emit: bytes]`,
// `[$out say: x]`) is the same at both phases.
// ─────────────────────────────────────────────────────────────────

fn install_console_proto_and_caps(w: &mut World) {
    // allocate a Console proto inheriting from Object.
    let console_proto = w.alloc(Form::with_proto(Value::Form(w.protos.object)));

    // primitive methods (rust):
    //   :emit:  — write bytes to fd. arg[0] must be a Sym whose
    //             text is the bytes to emit.
    //   :close  — no-op for stdout/stderr; future fds destruct via
    //             ForeignHandle.
    //   :next, :done? — these are write-only; raise.
    w.install_native(console_proto, "emit:", |w, self_, args| {
        use std::io::Write;
        let label = self_.as_form_id().and_then(|id| {
            let label_sym = w.intern("label");
            w.heap.get(id).slot(label_sym).as_sym()
        });
        let label_text = label.map(|s| w.resolve(s).to_string());
        let bytes_sym = match args.first().copied() {
            Some(Value::Sym(s)) => s,
            _ => {
                return Err(RaiseError::new(
                    w.intern("type-error"),
                    "emit: expects a symbol-payload (phase-A placeholder for String)",
                ));
            }
        };
        let text = w.resolve(bytes_sym).to_string();
        let result = match label_text.as_deref() {
            Some("stdout") => std::io::stdout().write_all(text.as_bytes()),
            Some("stderr") => std::io::stderr().write_all(text.as_bytes()),
            other => {
                return Err(RaiseError::new(
                    w.intern("dispatch-error"),
                    format!("Console with unknown label `{:?}`", other),
                ));
            }
        };
        result.map_err(|e| RaiseError::new(w.intern("io-error"), e.to_string()))?;
        Ok(Value::Nil)
    });

    // :say: x  — derived: emit (to-string x) then a newline.
    // phase A.10's protocol-derived stdlib will replace this with
    // a moof-side definition. for now: native, so we don't need
    // protocol machinery at boot.
    w.install_native(console_proto, "say:", |w, self_, args| {
        let to_string = w.intern("to-string");
        let text = w.send(args[0], to_string, &[])?;
        let emit = w.intern("emit:");
        w.send(self_, emit, &[text])?;
        let newline = Value::Sym(w.intern("\n"));
        w.send(self_, emit, &[newline])?;
        Ok(Value::Nil)
    });

    // :show: x — emit without newline. lets users compose multi-
    // value lines.
    w.install_native(console_proto, "show:", |w, self_, args| {
        let to_string = w.intern("to-string");
        let text = w.send(args[0], to_string, &[])?;
        let emit = w.intern("emit:");
        w.send(self_, emit, &[text])?;
        Ok(Value::Nil)
    });

    // :close — phase A: no-op. phase B's mco wires up real fd cleanup.
    w.install_native(console_proto, "close", |_, _, _| Ok(Value::Nil));

    // :next / :done? — Console is sink-only.
    w.install_native(console_proto, "next", |w, _, _| {
        Err(RaiseError::new(
            w.intern("not-supported"),
            ":next on a Console (write-only)",
        ))
    });
    w.install_native(console_proto, "done?", |_, _, _| Ok(Value::Bool(false)));

    // primordial $out, $err.
    let stdout_label = w.intern("stdout");
    let stderr_label = w.intern("stderr");
    let label_sym = w.intern("label");

    let mut out_form = Form::with_proto(Value::Form(console_proto));
    out_form.slots.insert(label_sym, Value::Sym(stdout_label));
    let out_id = w.alloc(out_form);

    let mut err_form = Form::with_proto(Value::Form(console_proto));
    err_form.slots.insert(label_sym, Value::Sym(stderr_label));
    let err_id = w.alloc(err_form);

    let global = w.global_env;
    let dollar_out = w.intern("$out");
    let dollar_err = w.intern("$err");
    w.env_bind(global, dollar_out, Value::Form(out_id));
    w.env_bind(global, dollar_err, Value::Form(err_id));

    // also expose the proto by name so user code can later subclass it.
    let console_name = w.intern("Console");
    w.env_bind(global, console_name, Value::Form(console_proto));
}

// ─────────────────────────────────────────────────────────────────
// global callables
// ─────────────────────────────────────────────────────────────────

/// expand `global_dispatcher!("+")` into a `NativeFn` that, given
/// args `[a, b, …]`, sends `:+` to `a` with `[b, …]` as args.
///
/// the macro yields a *bare* closure with no captures, which Rust
/// coerces to `fn(_) -> _`. this is what makes it a `NativeFn`.
macro_rules! global_dispatcher {
    ($sel:literal) => {{
        let f: NativeFn = |world, _self_, args| {
            if args.is_empty() {
                let kind = world.intern("arity");
                return Err(RaiseError::new(
                    kind,
                    concat!("global `", $sel, "` needs at least 1 arg"),
                ));
            }
            let sel = world.intern($sel);
            world.send(args[0], sel, &args[1..])
        };
        f
    }};
}

fn install_globals(w: &mut World) {
    // arithmetic + comparison forwarders.
    install_global(w, "+", global_dispatcher!("+"));
    install_global(w, "-", global_dispatcher!("-"));
    install_global(w, "*", global_dispatcher!("*"));
    install_global(w, "/", global_dispatcher!("/"));
    install_global(w, "<", global_dispatcher!("<"));
    install_global(w, ">", global_dispatcher!(">"));
    install_global(w, "<=", global_dispatcher!("<="));
    install_global(w, ">=", global_dispatcher!(">="));
    install_global(w, "=", global_dispatcher!("="));
    install_global(w, "!=", global_dispatcher!("!="));

    // structural ops.
    install_global(w, "head", global_dispatcher!("head"));
    install_global(w, "tail", global_dispatcher!("tail"));
    install_global(w, "null?", global_dispatcher!("null?"));
    install_global(w, "to-string", global_dispatcher!("to-string"));
    install_global(w, "not", global_dispatcher!("not"));

    // (cons head tail) ≡ [tail cons: head] — tail is the receiver.
    install_global(w, "cons", |world, _, args| {
        if args.len() != 2 {
            return Err(RaiseError::new(world.intern("arity"), "cons takes 2 args"));
        }
        let cons_sel = world.intern("cons:");
        world.send(args[1], cons_sel, &[args[0]])
    });

    // (list a b c) → '(a b c). builds a fresh list.
    install_global(w, "list", |world, _, args| Ok(world.make_list(args)));

    // (proto v) ≡ [v proto]. mostly for repl convenience.
    install_global(w, "proto", global_dispatcher!("proto"));
    install_global(w, "type-of", global_dispatcher!("proto"));
    install_global(w, "identity", global_dispatcher!("identity"));
    install_global(w, "inspect", global_dispatcher!("inspect"));

    // (length xs) — sends :length to the receiver.
    install_global(w, "length", global_dispatcher!("length"));
    install_global(w, "empty?", global_dispatcher!("empty?"));
    install_global(w, "reverse", global_dispatcher!("reverse"));
    install_global(w, "zero?", global_dispatcher!("zero?"));
    install_global(w, "positive?", global_dispatcher!("positive?"));
    install_global(w, "negative?", global_dispatcher!("negative?"));
    install_global(w, "abs", global_dispatcher!("abs"));
    install_global(w, "square", global_dispatcher!("square"));
    // (slot v 'name) — read slot directly. useful before
    // get-slot-method on every proto.
    install_global(w, "slot", |w, _, args| {
        if args.len() != 2 {
            return Err(RaiseError::new(w.intern("arity"), "(slot v 'name)"));
        }
        let id = args[0].as_form_id().ok_or_else(|| {
            RaiseError::new(w.intern("type-error"), "slot on tagged-immediate")
        })?;
        let name = args[1].as_sym().ok_or_else(|| {
            RaiseError::new(w.intern("type-error"), "slot name must be a symbol")
        })?;
        Ok(w.heap.get(id).slot(name))
    });
    // (slot-set! v 'name value) — write slot.
    install_global(w, "slot-set!", |w, _, args| {
        if args.len() != 3 {
            return Err(RaiseError::new(
                w.intern("arity"),
                "(slot-set! v 'name value)",
            ));
        }
        let id = args[0].as_form_id().ok_or_else(|| {
            RaiseError::new(w.intern("type-error"), "slot-set! on tagged-immediate")
        })?;
        let name = args[1].as_sym().ok_or_else(|| {
            RaiseError::new(w.intern("type-error"), "slot name must be a symbol")
        })?;
        w.heap.get_mut(id).slots.insert(name, args[2]);
        Ok(args[2])
    });
    // (set-handler! Proto 'selector method-fn) — install a method
    // on a proto's handler table. method-fn is typically a closure;
    // it must answer `:call` (so any Method-shaped Form works).
    //
    // this is the moldable-substrate's moof-side install primitive.
    // phase A.10's stdlib uses it to install protocol-derived
    // methods on List, Integer, etc. without needing a defproto
    // operative yet.
    install_global(w, "set-handler!", |w, _, args| {
        if args.len() != 3 {
            return Err(RaiseError::new(
                w.intern("arity"),
                "(set-handler! Proto 'selector method-fn)",
            ));
        }
        let proto_id = args[0].as_form_id().ok_or_else(|| {
            RaiseError::new(w.intern("type-error"), "set-handler! Proto must be a Form")
        })?;
        let sel = args[1].as_sym().ok_or_else(|| {
            RaiseError::new(w.intern("type-error"), "set-handler! selector must be a symbol")
        })?;
        w.heap
            .get_mut(proto_id)
            .handlers
            .insert(sel, args[2]);
        Ok(args[2])
    });
}

/// allocate a global-dispatcher Form (proto: Method, native fn
/// recorded in side table) and bind it under `name` in the global
/// env.
fn install_global(w: &mut World, name: &str, native: NativeFn) {
    let f = Form::with_proto(Value::Form(w.protos.method));
    let id = w.alloc(f);
    let name_sym = w.intern(name);
    // tag :source with the symbol so `[+ source] → '+`.
    w.heap
        .get_mut(id)
        .meta
        .insert(w.source_sym, Value::Sym(name_sym));
    w.native_fns.insert(id, native);
    let global = w.global_env;
    w.env_bind(global, name_sym, Value::Form(id));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(w: &mut World, src: &str) -> Result<Value, RaiseError> {
        let form = w.read(src).map_err(|e| RaiseError::from_reader(&mut w.syms, e))?;
        let chunk = crate::compiler::compile(w, form)?;
        w.run_top(chunk)
    }

    fn fresh() -> World {
        let mut w = World::new();
        install(&mut w);
        w
    }

    #[test]
    fn arithmetic_works() {
        let mut w = fresh();
        assert_eq!(ev(&mut w, "(+ 1 2)").unwrap(), Value::Int(3));
        assert_eq!(ev(&mut w, "(- 10 3)").unwrap(), Value::Int(7));
        assert_eq!(ev(&mut w, "(* 4 5)").unwrap(), Value::Int(20));
        assert_eq!(ev(&mut w, "(/ 20 4)").unwrap(), Value::Int(5));
    }

    #[test]
    fn nested_arithmetic() {
        let mut w = fresh();
        assert_eq!(ev(&mut w, "(* 3 (+ 4 5))").unwrap(), Value::Int(27));
    }

    #[test]
    fn comparison_works() {
        let mut w = fresh();
        assert_eq!(ev(&mut w, "(< 1 2)").unwrap(), Value::Bool(true));
        assert_eq!(ev(&mut w, "(< 2 1)").unwrap(), Value::Bool(false));
        assert_eq!(ev(&mut w, "(= 5 5)").unwrap(), Value::Bool(true));
        assert_eq!(ev(&mut w, "(>= 5 5)").unwrap(), Value::Bool(true));
        assert_eq!(ev(&mut w, "(!= 5 6)").unwrap(), Value::Bool(true));
    }

    #[test]
    fn integer_send_directly() {
        // bypass the global; send to the integer directly via :+
        // on the Integer proto.
        let mut w = fresh();
        let plus = w.intern("+");
        assert_eq!(
            w.send(Value::Int(5), plus, &[Value::Int(7)]).unwrap(),
            Value::Int(12)
        );
    }

    #[test]
    fn proto_returns_proto_form() {
        let mut w = fresh();
        let r = ev(&mut w, "(proto 5)").unwrap();
        assert_eq!(r, Value::Form(w.protos.integer));
    }

    #[test]
    fn identity_returns_form_id() {
        let mut w = fresh();
        // tagged immediates have identity 0
        assert_eq!(ev(&mut w, "(identity 5)").unwrap(), Value::Int(0));
        // a fresh list has a real id
        let v = ev(&mut w, "(list 1 2 3)").unwrap();
        let id = v.as_form_id().unwrap();
        let identity_sym = w.intern("identity");
        let r = w.send(v, identity_sym, &[]).unwrap();
        assert_eq!(r, Value::Int(id.0 as i64));
    }

    #[test]
    fn list_head_tail_cons() {
        let mut w = fresh();
        let head_sym = w.intern("head");
        // build (1 2 3) and inspect
        let v = ev(&mut w, "(list 1 2 3)").unwrap();
        assert_eq!(w.send(v, head_sym, &[]).unwrap(), Value::Int(1));
        let tail_sym = w.intern("tail");
        let tail = w.send(v, tail_sym, &[]).unwrap();
        assert_eq!(w.send(tail, head_sym, &[]).unwrap(), Value::Int(2));
        // (cons 0 (list 1 2 3)) → list with first element 0
        let consed = ev(&mut w, "(cons 0 (list 1 2 3))").unwrap();
        assert_eq!(w.send(consed, head_sym, &[]).unwrap(), Value::Int(0));
    }

    #[test]
    fn null_check_works() {
        let mut w = fresh();
        assert_eq!(ev(&mut w, "(null? nil)").unwrap(), Value::Bool(true));
        assert_eq!(ev(&mut w, "(null? (list 1))").unwrap(), Value::Bool(false));
        // (null? 5) — Integer doesn't have :null?, so dnu raises.
        let err = ev(&mut w, "(null? 5)").unwrap_err();
        assert!(err.message.contains("does not understand"));
    }

    #[test]
    fn integer_to_string() {
        let mut w = fresh();
        let r = ev(&mut w, "(to-string 42)").unwrap();
        assert_eq!(w.resolve(r.as_sym().unwrap()), "42");
    }

    #[test]
    fn def_then_use() {
        let mut w = fresh();
        ev(&mut w, "(def x 10)").unwrap();
        ev(&mut w, "(def y 20)").unwrap();
        assert_eq!(ev(&mut w, "(+ x y)").unwrap(), Value::Int(30));
    }

    #[test]
    fn factorial_works_end_to_end() {
        // the reflection of phase-A's forcing function: a real
        // recursive definition compiles and runs and produces a
        // correct answer.
        let mut w = fresh();
        ev(
            &mut w,
            "(def fact (fn (n)
                (if (= n 0)
                    1
                    (* n (fact (- n 1))))))",
        )
        .unwrap();
        assert_eq!(ev(&mut w, "(fact 0)").unwrap(), Value::Int(1));
        assert_eq!(ev(&mut w, "(fact 1)").unwrap(), Value::Int(1));
        assert_eq!(ev(&mut w, "(fact 5)").unwrap(), Value::Int(120));
        assert_eq!(ev(&mut w, "(fact 10)").unwrap(), Value::Int(3628800));
    }

    #[test]
    fn closures_capture_correctly() {
        let mut w = fresh();
        ev(
            &mut w,
            "(def make-adder (fn (n) (fn (x) (+ x n))))",
        )
        .unwrap();
        assert_eq!(ev(&mut w, "((make-adder 5) 7)").unwrap(), Value::Int(12));
        assert_eq!(ev(&mut w, "((make-adder 10) 20)").unwrap(), Value::Int(30));
    }

    #[test]
    fn let_with_arithmetic() {
        let mut w = fresh();
        assert_eq!(
            ev(&mut w, "(let ((a 3) (b 4)) (+ a b))").unwrap(),
            Value::Int(7)
        );
    }

    #[test]
    fn does_not_understand_default_raises() {
        let mut w = fresh();
        let mystery = w.intern("flibbertigibbet");
        let err = w.send(Value::Int(5), mystery, &[]).unwrap_err();
        assert_eq!(w.resolve(err.kind), "does-not-understand");
    }

    #[test]
    fn reflection_proto_via_send() {
        let mut w = fresh();
        let proto_sym = w.intern("proto");
        assert_eq!(
            w.send(Value::Int(7), proto_sym, &[]).unwrap(),
            Value::Form(w.protos.integer)
        );
        assert_eq!(
            w.send(Value::Bool(true), proto_sym, &[]).unwrap(),
            Value::Form(w.protos.bool_)
        );
    }

    #[test]
    fn reflection_source_returns_source_for_chunks() {
        // a chunk's :source meta carries the original Form.
        let mut w = fresh();
        let f = w.read("(+ 1 2)").unwrap();
        let chunk = crate::compiler::compile(&mut w, f).unwrap();
        let source_sel = w.intern("source");
        let r = w.send(Value::Form(chunk), source_sel, &[]).unwrap();
        // r should be the original parsed form (a list).
        assert_eq!(r, f);
    }

    #[test]
    fn reflection_slots_returns_slot_pairs() {
        // build an object with known slots; reflect.
        let mut w = fresh();
        let mut f = Form::with_proto(Value::Form(w.protos.object));
        let a = w.intern("a");
        let b = w.intern("b");
        f.slots.insert(a, Value::Int(1));
        f.slots.insert(b, Value::Int(2));
        let id = w.alloc(f);
        let slots_sel = w.intern("slots");
        let r = w.send(Value::Form(id), slots_sel, &[]).unwrap();
        // r is a list of (sym . value) pairs, in insertion order.
        let entries = w.list_to_vec(r).unwrap();
        assert_eq!(entries.len(), 2);
        let pair0 = w.list_to_vec(entries[0]).unwrap();
        assert_eq!(pair0[0], Value::Sym(a));
        assert_eq!(pair0[1], Value::Int(1));
        let pair1 = w.list_to_vec(entries[1]).unwrap();
        assert_eq!(pair1[0], Value::Sym(b));
        assert_eq!(pair1[1], Value::Int(2));
    }

    #[test]
    fn integer_inspect_falls_through_to_to_string() {
        let mut w = fresh();
        let r = ev(&mut w, "(inspect 42)").unwrap();
        assert_eq!(w.resolve(r.as_sym().unwrap()), "42");
    }

    #[test]
    fn out_cap_is_bound_in_global_env() {
        let mut w = fresh();
        let dollar_out = w.intern("$out");
        let v = w.env_lookup(w.global_env, dollar_out).unwrap();
        // it's a Form (a Console instance).
        let id = v.as_form_id().unwrap();
        // its proto is Console.
        let proto = w.heap.get(id).proto;
        // Console isn't on `Protos` (it's a user-visible intrinsic
        // proto living in the global env). check via name lookup.
        let console_sym = w.intern("Console");
        let console_proto = w.env_lookup(w.global_env, console_sym).unwrap();
        assert_eq!(proto, console_proto);
    }

    #[test]
    fn out_cap_responds_to_emit() {
        // we can't easily capture stdout from a unit test; verify
        // that :emit: dispatches without panicking on a valid call.
        let mut w = fresh();
        let dollar_out = w.intern("$out");
        let out = w.env_lookup(w.global_env, dollar_out).unwrap();
        let emit = w.intern("emit:");
        // we deliberately use stderr for the test so test runner's
        // captured stdout isn't disrupted. switch out's label.
        let label_sym = w.intern("label");
        let stderr_sym = w.intern("stderr");
        let id = out.as_form_id().unwrap();
        w.heap.get_mut(id).slots.insert(label_sym, Value::Sym(stderr_sym));
        let payload = Value::Sym(w.intern(""));
        let r = w.send(out, emit, &[payload]).unwrap();
        assert_eq!(r, Value::Nil);
    }

    #[test]
    fn out_cap_say_dispatches_through_to_string() {
        // :say: 42 → emit "42"; emit "\n". exercises the dispatch
        // chain without actually writing.
        let mut w = fresh();
        // route to stderr so test runner stays happy.
        let dollar_out = w.intern("$out");
        let out = w.env_lookup(w.global_env, dollar_out).unwrap();
        let label_sym = w.intern("label");
        let stderr_sym = w.intern("stderr");
        let id = out.as_form_id().unwrap();
        w.heap.get_mut(id).slots.insert(label_sym, Value::Sym(stderr_sym));
        // the actual call:
        let say = w.intern("say:");
        let r = w.send(out, say, &[Value::Int(42)]).unwrap();
        assert_eq!(r, Value::Nil);
    }

    #[test]
    fn no_free_function_print_in_world() {
        // the substrate symbol-table check from
        // `process/docs-driven.md`'s capability rule. there must be
        // no `print`, `println`, `puts` global binding.
        let mut w = fresh();
        for forbidden in ["print", "println", "puts", "simulated_println"] {
            let s = w.intern(forbidden);
            let v = w.env_lookup(w.global_env, s);
            assert!(
                v.is_none(),
                "forbidden global `{}` is bound (must not be)",
                forbidden
            );
        }
    }

    #[test]
    fn user_code_cannot_synthesize_a_cap() {
        // there is no constructor that produces a Console out of
        // thin air. (Console proto's :new would, but that's the
        // moldable extension hook for the future Transcript proto;
        // phase A's discipline is "don't invoke :new on Console
        // unless the supervisor authorizes it." we don't enforce
        // *yet* — phase B's cap-attenuation primitive does. for
        // now: document the gap honestly.)
        //
        // what *is* enforced: the supervisor binds the primordial
        // caps; they're the only ones in scope.
        let mut w = fresh();
        let dollar_out = w.intern("$out");
        let dollar_err = w.intern("$err");
        let dollar_x = w.intern("$x");
        assert!(w.env_lookup(w.global_env, dollar_out).is_some());
        assert!(w.env_lookup(w.global_env, dollar_err).is_some());
        assert!(w.env_lookup(w.global_env, dollar_x).is_none());
    }
}
