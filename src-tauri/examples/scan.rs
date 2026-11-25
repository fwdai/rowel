//! Thin CLI over the image scanner, for checking the platform OCR backend
//! against a real picture: `cargo run --example scan -- /path/to/photo.png`.
//! Prints the recognized lines, then whatever the parsers made of them. Not
//! shipped in the app.
//!
//! With a second path, the holder's photograph is looked for as well — on any
//! picture, recognized or not — and written there as the data URI an entry
//! would keep: `cargo run --example scan -- photo.png portrait.txt`.

use std::path::Path;

use rowel_lib::scan::{platform_ocr, platform_portrait, scan_lines, Image};

fn main() {
    let Some(path) = std::env::args().nth(1) else {
        eprintln!("usage: scan <image path>");
        std::process::exit(1);
    };
    let Some(ocr) = platform_ocr() else {
        eprintln!("no text recognizer on this platform");
        std::process::exit(1);
    };

    let lines = ocr.recognize(Path::new(&path)).unwrap_or_else(|e| {
        eprintln!("{e}");
        std::process::exit(1);
    });
    println!("--- {} lines recognized", lines.len());
    for line in &lines {
        println!("{line}");
    }

    match scan_lines(&lines) {
        Some(found) => {
            println!("--- {}", found.kind);
            for (key, value) in &found.fields {
                println!("{key}: {value}");
            }
        }
        None => println!("--- nothing recognized"),
    }

    if let Some(out) = std::env::args().nth(2) {
        match platform_portrait(Image::File(Path::new(&path))) {
            Some(portrait) => {
                std::fs::write(&out, &portrait).expect("the portrait could not be written");
                println!(
                    "--- portrait: {} bytes as a data URI, in {out}",
                    portrait.len()
                );
            }
            None => println!("--- no portrait"),
        }
    }
}
