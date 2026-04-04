/// TUI inspector for MOOF — browse the live object graph.
///
/// Uses ratatui + crossterm. Reads the heap directly.
/// Invoked from the REPL via (browse) or (browse obj).

pub mod inspector;
