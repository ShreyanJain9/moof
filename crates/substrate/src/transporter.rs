//! `$transporter` — Self-style file ↔ image bridge.
//!
//! files are *transport*. the canonical home of moof code is the
//! image (the live runtime objects). $transporter ferries source
//! text into the image (`:load:`, `:loadAll:`) and — eventually —
//! ferries in-image objects back out as files (`:dump:toFile:`,
//! reserved for a future session).
//!
//! the cap is a primordial — installed by intrinsics.rs at world
//! creation, bound to `$transporter` in the global env. it is the
//! only path through which moof code reads files. the substrate
//! itself uses it directly (in `new_world()`) to load `lib/main.moof`.
//!
//! see `docs/superpowers/specs/2026-05-02-transporter-and-stdlib-
//! modularization-design.md` for the full design.

use crate::value::Value;
use crate::world::{RaiseError, World};
use std::path::{Path, PathBuf};

/// resolve the lib root, in order:
///   1. `MOOF_LIB` env var (if set and is a directory)
///   2. `<dir of std::env::current_exe()>/../lib` (if a directory)
///   3. `./lib` relative to cwd (if a directory)
///   4. None — caller raises `'tx-no-root`.
pub fn resolve_lib_root() -> Option<PathBuf> {
    if let Ok(env_path) = std::env::var("MOOF_LIB") {
        let p = PathBuf::from(env_path);
        if p.is_dir() {
            return Some(p);
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent().and_then(|p| p.parent()) {
            let candidate = parent.join("lib");
            if candidate.is_dir() {
                return Some(candidate);
            }
        }
    }
    let cwd_lib = PathBuf::from("./lib");
    if cwd_lib.is_dir() {
        return Some(cwd_lib);
    }
    None
}

/// Build the `$transporter` proto-Form. The proto-Form is the cap
/// itself — there is exactly one Transporter, so the proto and the
/// "instance" are identical, like primordial $out / $err.
pub fn install(w: &mut World) {
    use crate::form::Form;

    let proto = w.alloc(Form::with_proto(Value::Form(w.protos.object)));

    // :load: — minimum surface this session.
    w.install_native(proto, "load:", |w, _self, args| {
        let path_val = args.first().copied().unwrap_or(Value::Nil);
        let rel = w
            .string_text(path_val)
            .map(|s| s.to_string())
            .ok_or_else(|| {
                RaiseError::new(w.intern("tx-bad-arg"), ":load: expects a String path")
            })?;
        load_relative(w, &rel)
    });

    // :loadAll: — walks a Cons of Strings, calls :load: on each.
    w.install_native(proto, "loadAll:", |w, _self, args| {
        let list = args.first().copied().unwrap_or(Value::Nil);
        let paths = w.list_to_vec(list).map_err(|_| {
            RaiseError::new(w.intern("tx-bad-arg"), ":loadAll: expects a Cons")
        })?;
        let mut last = Value::Nil;
        for (i, v) in paths.iter().enumerate() {
            let rel = w.string_text(*v).map(|s| s.to_string()).ok_or_else(|| {
                RaiseError::new(
                    w.intern("tx-bad-arg"),
                    format!(":loadAll: element {} is not a String", i + 1),
                )
            })?;
            last = load_relative(w, &rel)?;
        }
        Ok(last)
    });

    // :root — diagnostic; returns the resolved root as a String.
    w.install_native(proto, "root", |w, _self, _args| {
        match &w.transporter_root {
            Some(p) => {
                let s = p.display().to_string();
                Ok(w.make_string(&s))
            }
            None => Err(RaiseError::new(
                w.intern("tx-no-root"),
                "transporter has no root configured",
            )),
        }
    });

    // :dump:toFile: — RESERVED. The Transporter's name promises a
    // round-trip; the second half lands in a future session that
    // walks a Form's :handlers / :slots / :meta and reconstructs
    // source text using the per-method :source slot.
    w.install_native(proto, "dump:toFile:", |w, _self, _args| {
        Err(RaiseError::new(
            w.intern("tx-unimplemented"),
            ":dump:toFile: is reserved — the file→image direction lands in a future session",
        ))
    });

    // bind the proto-Form as the `$transporter` global. that's the
    // cap itself; receiving methods sends to it.
    let global = w.global_env;
    let dollar = w.intern("$transporter");
    w.env_bind(global, dollar, Value::Form(proto));
}

/// shared implementation for `:load:` and `:loadAll:`. resolves rel
/// against the world's transporter_root, reads the file, and
/// `eval_program`'s its contents.
fn load_relative(w: &mut World, rel: &str) -> Result<Value, RaiseError> {
    if Path::new(rel).is_absolute() || rel.contains("..") {
        return Err(RaiseError::new(
            w.intern("tx-bad-path"),
            format!(
                ":load: refuses absolute or `..`-traversing paths: {:?}",
                rel
            ),
        ));
    }
    let root = w.transporter_root.clone().ok_or_else(|| {
        RaiseError::new(
            w.intern("tx-no-root"),
            "transporter has no root configured",
        )
    })?;
    let abs = root.join(rel);
    if !abs.is_file() {
        return Err(RaiseError::new(
            w.intern("tx-not-found"),
            format!("not found: {} (resolved as {})", rel, abs.display()),
        ));
    }
    let source = std::fs::read_to_string(&abs).map_err(|e| {
        RaiseError::new(
            w.intern("tx-read-error"),
            format!("{}: {}", abs.display(), e),
        )
    })?;
    crate::eval_program(w, &source).map_err(|e| {
        // wrap inner errors with the file path for diagnosis. preserve
        // the inner symbol so callers can still pattern-match by kind.
        RaiseError::new(
            e.kind,
            format!("{}: {}", abs.display(), e.message),
        )
    })
}
