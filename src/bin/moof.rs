use std::path::PathBuf;

use clap::Parser;

#[derive(Parser)]
#[command(name = "moof", about = "Moof Open Objectspace Fabric")]
struct Cli {
    /// Path to the image directory
    #[arg(long, default_value = ".moof")]
    image: PathBuf,

    /// Run the MCP server instead of the REPL
    #[arg(long)]
    mcp: bool,

    /// Run the TUI inspector
    #[arg(long)]
    browse: bool,
}

fn main() {
    let cli = Cli::parse();

    if cli.mcp {
        eprintln!("MCP server not yet implemented in v2");
        std::process::exit(1);
    }

    if cli.browse {
        eprintln!("TUI inspector not yet implemented in v2");
        std::process::exit(1);
    }

    if let Err(e) = moof_shell::repl::run(&cli.image) {
        eprintln!("fatal: {e}");
        std::process::exit(1);
    }
}
