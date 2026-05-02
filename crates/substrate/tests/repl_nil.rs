//! REPL must display `nil` when the user evaluates `nil`. The previous
//! "lisp convention" of suppressing nil from REPL output was an
//! ergonomic bug — moof has its own conventions and `(defmethod nil
//! (inspect) "nil")` is canonical.

use std::process::Command;

#[test]
fn repl_displays_nil_on_nil_input() {
    // one-shot mode is a sufficient proxy for the REPL print path —
    // both share `print_via_out_inspect` (after Task 2) and the same
    // `is_nil()` gate. (full pty-driving would need expectrl; one-shot
    // is enough for this regression.)
    let out = Command::new(env!("CARGO_BIN_EXE_moof"))
        .arg("nil")
        .output()
        .expect("failed to spawn moof");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(
        stdout, "nil\n",
        "expected `nil\\n` on stdout for `moof nil`, got: {:?}",
        stdout
    );
}
