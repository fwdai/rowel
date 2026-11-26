//! Thin CLI over the image scanner, for checking the platform backends against
//! a real picture: `cargo run --example scan -- /path/to/photo.png`. Prints
//! the recognized lines, then what a scan of the picture comes to. Not shipped
//! in the app.
//!
//! The holder's photograph is printed as its size only; with a second path it
//! is written there, as the data URI an entry would keep:
//! `cargo run --example scan -- photo.png portrait.txt`.

use std::path::Path;

use rowel_lib::scan::{platform_ocr, read, Image};

fn main() {
    let Some(path) = std::env::args().nth(1) else {
        eprintln!("usage: scan <image path> [portrait output path]");
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

    let Some(found) = read(&lines, Image::File(Path::new(&path))) else {
        println!("--- nothing recognized");
        return;
    };
    println!("--- {}", found.kind);
    for (key, value) in &found.fields {
        if key == "portrait" {
            println!("{key}: {} bytes as a data URI", value.len());
            if let Some(out) = std::env::args().nth(2) {
                std::fs::write(&out, value).expect("the portrait could not be written");
            }
        } else {
            println!("{key}: {value}");
        }
    }
}
