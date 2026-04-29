//! moof v4 — phase 2 substrate.
//!
//! see docs/roadmap.md. forcing function (phase 1): `(+ 1 2) → 3`.
//! phase 2 adds: special forms (`def`, `if`, `let`, `fn`, `do`,
//! `quote`), closures via captured envs, comparison + list ops, file
//! loading, and a bootstrap stdlib written in moof.
//!
//! invariants held since phase 1 (laws/substrate-laws.md):
//! - one Form heap kind (L1).
//! - proto-chain dispatch (L2, L3).
//! - source canonical, bytecode derived (L5).

pub mod builtins;
pub mod compiler;
pub mod form;
pub mod heap;
pub mod opcodes;
pub mod print;
pub mod protos;
pub mod reader;
pub mod sym;
pub mod value;
pub mod vm;
pub mod world;

pub use form::FormId;
pub use opcodes::ChunkId;
pub use sym::SymId;
pub use value::Value;
pub use world::World;

/// the bootstrap stdlib, embedded so the substrate ships
/// self-sufficient. loaded in `World::new_with_bootstrap`.
pub const BOOTSTRAP_SOURCE: &str = include_str!("../lib/bootstrap.moof");

/// build a fresh world and load the bootstrap stdlib.
pub fn new_world() -> Result<World, String> {
    let mut world = World::new();
    eval_program(&mut world, BOOTSTRAP_SOURCE)?;
    Ok(world)
}

/// evaluate a multi-expression program in the world's global env.
/// returns the value of the last expression (or `Nil` if empty).
pub fn eval_program(world: &mut World, source: &str) -> Result<Value, String> {
    let forms = reader::read_all(world, source)?;
    let mut last = Value::Nil;
    for form in forms {
        last = eval_form(world, form)?;
    }
    Ok(last)
}

/// evaluate a single expression in the world's global env.
pub fn eval_str(world: &mut World, input: &str) -> Result<Value, String> {
    let form = reader::read(world, input)?;
    eval_form(world, form)
}

/// compile and run a single Form in the world's global env.
pub fn eval_form(world: &mut World, form: Value) -> Result<Value, String> {
    let chunk = compiler::compile(world, form)?;
    let chunk_id = world.add_chunk(chunk);
    let env = world.global_env;
    vm::run_chunk(world, chunk_id, env)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(s: &str) -> Value {
        let mut w = new_world().expect("world boot failed");
        eval_str(&mut w, s).expect("eval failed")
    }

    fn evp(s: &str) -> Value {
        let mut w = new_world().expect("world boot failed");
        eval_program(&mut w, s).expect("eval failed")
    }

    // ── phase 1 baseline: still passes
    #[test]
    fn basic_arithmetic() {
        assert_eq!(ev("(+ 1 2)"), Value::Int(3));
        assert_eq!(ev("(* 3 (+ 4 5))"), Value::Int(27));
        assert_eq!(ev("(- 10 3 2)"), Value::Int(5));
    }

    // ── phase 2 special forms
    #[test]
    fn def_and_lookup() {
        assert_eq!(
            evp("(def x 5) (def y 7) (+ x y)"),
            Value::Int(12)
        );
    }

    #[test]
    fn if_form_branches() {
        let mut w = new_world().unwrap();
        let yes = match eval_str(&mut w, "(if 1 42 99)").unwrap() {
            Value::Int(n) => n,
            _ => panic!(),
        };
        let no = match eval_str(&mut w, "(if () 42 99)").unwrap() {
            Value::Int(n) => n,
            _ => panic!(),
        };
        assert_eq!(yes, 42);
        assert_eq!(no, 99);
    }

    #[test]
    fn let_bindings() {
        assert_eq!(
            ev("(let ((a 1) (b 2)) (+ a b))"),
            Value::Int(3)
        );
        // nested let with shadowing
        assert_eq!(
            ev("(let ((x 10)) (let ((x 1) (y 2)) (+ x y)))"),
            Value::Int(3)
        );
    }

    #[test]
    fn closures() {
        assert_eq!(
            evp("(def square (fn (x) (* x x))) (square 7)"),
            Value::Int(49)
        );
        // capture
        assert_eq!(
            evp("(def make-adder (fn (n) (fn (x) (+ x n))))
                 (def add5 (make-adder 5))
                 (add5 10)"),
            Value::Int(15)
        );
    }

    #[test]
    fn recursion() {
        assert_eq!(
            evp("(def fact
                   (fn (n)
                     (if (= n 0) 1 (* n (fact (- n 1))))))
                 (fact 6)"),
            Value::Int(720)
        );
    }

    #[test]
    fn comparison() {
        assert_eq!(ev("(< 1 2)"), Value::Bool(true));
        assert_eq!(ev("(< 2 1)"), Value::Bool(false));
        assert_eq!(ev("(= 5 5)"), Value::Bool(true));
        assert_eq!(ev("(>= 5 5)"), Value::Bool(true));
        assert_eq!(ev("(< 1 2 3)"), Value::Bool(true));
        assert_eq!(ev("(< 1 3 2)"), Value::Bool(false));
    }

    #[test]
    fn list_ops() {
        assert_eq!(ev("(null? ())"), Value::Bool(true));
        assert_eq!(ev("(null? (cons 1 ()))"), Value::Bool(false));
        assert_eq!(ev("(head (cons 1 (cons 2 ())))"), Value::Int(1));
    }

    // ── stdlib (loaded from lib/bootstrap.moof)
    #[test]
    fn stdlib_length() {
        assert_eq!(ev("(length (list 1 2 3 4 5))"), Value::Int(5));
    }

    #[test]
    fn stdlib_map_reduce() {
        assert_eq!(
            ev("(reduce + 0 (map (fn (x) (* x x)) (list 1 2 3 4)))"),
            Value::Int(30)
        );
    }

    #[test]
    fn stdlib_factorial() {
        assert_eq!(ev("(factorial 6)"), Value::Int(720));
    }
}
