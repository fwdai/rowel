#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn main() {
    let mut args = std::env::args();
    let program = args.next().unwrap_or_else(|| "rowel".to_string());
    std::process::exit(rowel_cli::run(&program, args));
}

// The library is empty on a phone (see `lib.rs`), but a binary target still
// needs an entry point there for the workspace to build.
#[cfg(any(target_os = "android", target_os = "ios"))]
fn main() {}
