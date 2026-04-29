//! printing — Value → String.
//!
//! two flavors:
//! - `show`: lisp-readable representation. strings get quotes, etc.
//!   used by the CLI's auto-print of the last expression's value.
//! - `display`: human-friendly. strings without quotes. used by
//!   `println` / `print`.
//!
//! later phases route both through `:to-string` and `:inspect` send
//! dispatch (laws/substrate-laws.md L3) so user-defined protos can
//! override.

use crate::value::Value;
use crate::world::World;

/// lisp-readable representation. strings are quoted.
pub fn show(w: &World, v: Value) -> String {
    let mut out = String::new();
    write_value(w, &mut out, v, true);
    out
}

/// human-friendly representation. strings are unquoted.
pub fn display(w: &World, v: Value) -> String {
    let mut out = String::new();
    write_value(w, &mut out, v, false);
    out
}

fn write_value(w: &World, out: &mut String, v: Value, quote_strings: bool) {
    match v {
        Value::Nil => out.push_str("()"),
        Value::Bool(true) => out.push_str("#true"),
        Value::Bool(false) => out.push_str("#false"),
        Value::Int(n) => out.push_str(&n.to_string()),
        Value::Sym(s) => out.push_str(w.syms.name(s)),
        Value::Form(id) => {
            let f = w.heap.get(id);
            // String form?
            if let Some(s) = &f.bytes {
                if quote_strings {
                    out.push('"');
                    for c in s.chars() {
                        match c {
                            '"' => out.push_str("\\\""),
                            '\\' => out.push_str("\\\\"),
                            '\n' => out.push_str("\\n"),
                            '\t' => out.push_str("\\t"),
                            '\r' => out.push_str("\\r"),
                            other => out.push(other),
                        }
                    }
                    out.push('"');
                } else {
                    out.push_str(s);
                }
                return;
            }
            // List form?
            if f.proto == w.list_proto {
                out.push('(');
                let mut first = true;
                let mut cur = Value::Form(id);
                loop {
                    match cur {
                        Value::Nil => break,
                        Value::Form(fid) => {
                            let fc = w.heap.get(fid);
                            if fc.proto != w.list_proto {
                                out.push_str(" . ");
                                write_value(w, out, cur, quote_strings);
                                break;
                            }
                            if !first {
                                out.push(' ');
                            }
                            first = false;
                            write_value(w, out, fc.head, quote_strings);
                            cur = fc.args;
                        }
                        _ => {
                            out.push_str(" . ");
                            write_value(w, out, cur, quote_strings);
                            break;
                        }
                    }
                }
                out.push(')');
                return;
            }
            // send-form `[…]`?
            if f.proto == w.send_form_proto {
                out.push('[');
                write_value(w, out, f.head, quote_strings);
                let mut cur = f.args;
                while let Value::Form(fid) = cur {
                    let fc = w.heap.get(fid);
                    if fc.proto != w.list_proto {
                        break;
                    }
                    out.push(' ');
                    write_value(w, out, fc.head, quote_strings);
                    cur = fc.args;
                }
                out.push(']');
                return;
            }
            // Closure form?
            if f.proto == w.closure_proto {
                out.push_str("#<closure>");
                return;
            }
            // Builtin form?
            if f.proto == w.builtin_proto {
                out.push_str("#<builtin>");
                return;
            }
            // generic Form.
            out.push_str(&format!("#<form {}>", id.0));
        }
    }
}
