//! `cargo run --example check_original -- <file>...`
//!
//! Loads each file with the port's own loader and prints a redacted summary: which keys were read, which keys
//! of the file were not read (key names only), and which values the loader refuses (key name only). Values are
//! never printed. `*translations.properties` files are checked against the translation keys, every other file
//! against the config keys.
use mod_voicechat::horizonvc_core::config::{Config, ORIGINAL_KEYS};
use mod_voicechat::horizonvc_core::properties;
use std::path::Path;

const TRANSLATION_KEYS: [&str; 4] = [
    "force_voicechat_kick_message",
    "rate_limit_kick_message",
    "voicechat_not_compatible_message",
    "voicechat_needed_for_command_message",
];

fn main() {
    let files: Vec<String> = std::env::args().skip(1).collect();
    if files.is_empty() {
        eprintln!("usage: check_original <file>...");
        std::process::exit(2);
    }
    let mut failed = false;
    for file in files {
        let name = Path::new(&file)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        println!("== {name}");
        let text = match std::fs::read_to_string(&file) {
            Ok(t) => t,
            Err(_) => {
                println!("  cannot read the file");
                failed = true;
                continue;
            }
        };
        let translations = name.ends_with("translations.properties");
        let known: &[&str] = if translations { &TRANSLATION_KEYS } else { &ORIGINAL_KEYS };
        let pairs = properties::parse(&text);
        let mut read = Vec::new();
        let mut not_read = Vec::new();
        let mut refused = Vec::new();
        for (key, value) in &pairs {
            if !known.contains(&key.as_str()) {
                not_read.push(key.clone());
                continue;
            }
            if !translations {
                // Check the value alone, with the port's own loader; the message names the key, never the value.
                if let Err(e) = Config::parse(&format!("{key}={value}")) {
                    refused.push(e);
                    continue;
                }
            }
            read.push(key.clone());
        }
        let missing: Vec<&str> = known
            .iter()
            .copied()
            .filter(|k| !pairs.iter().any(|(p, _)| p == k))
            .collect();
        println!("  read ({}): {}", read.len(), read.join(", "));
        println!("  not read ({}): {}", not_read.len(), not_read.join(", "));
        println!("  not in file, port default used ({}): {}", missing.len(), missing.join(", "));
        for e in &refused {
            println!("  refused: {e}");
            failed = true;
        }
    }
    if failed {
        std::process::exit(1);
    }
}
