fn main() {
    // Phase 0 placeholder. Subcommand dispatch arrives in Phase 2.
    let arg = std::env::args().nth(1);
    match arg.as_deref() {
        Some("--version") => println!("slopgate-rs {}", env!("CARGO_PKG_VERSION")),
        _ => {
            eprintln!("slopgate-rs: no subcommands yet (Phase 0 scaffold)");
            std::process::exit(2);
        }
    }
}
