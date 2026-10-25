#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn main() {
    std::process::exit(rowel_cli::run(std::env::args().skip(1)));
}

// The library is empty on a phone (see `lib.rs`), but a binary target still
// needs an entry point there for the workspace to build.
#[cfg(any(target_os = "android", target_os = "ios"))]
fn main() {}
