use std::path::PathBuf;
fn main() {
    let mut args = std::env::args().skip(1);
    let root = match args.next().as_deref() {
        None => PathBuf::from("."),
        Some("--root") => match args.next() {
            Some(path) => PathBuf::from(path),
            None => {
                eprintln!("--root requires a path");
                std::process::exit(2);
            }
        },
        Some(_) => {
            eprintln!("usage: slopgate-architecture [--root <repository>]");
            std::process::exit(2);
        }
    };
    if args.next().is_some() {
        eprintln!("unexpected argument");
        std::process::exit(2);
    }
    match slopgate_architecture::inspect(&root) {
        Ok(errors) if errors.is_empty() => println!("Architecture contract passed: dependency boundaries, source ownership, specification traceability."),
        Ok(errors) => { for error in errors { eprintln!("{error}"); } std::process::exit(1); }
        Err(error) => { eprintln!("Architecture review incomplete: {error}"); std::process::exit(2); }
    }
}
