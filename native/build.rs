#![forbid(unsafe_code)]

use std::{env, fs, path::PathBuf};

fn main() {
    let output = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo deve definir OUT_DIR"));
    for source in [
        "data/utilities.json",
        "data/variants.json",
        "data/compat-theme.json",
        "data/property-order.json",
        "data/completions.json",
        "data/js-regex-unicode.json",
        "vendor/unicode/CaseFolding-17.0.0.txt",
    ] {
        println!("cargo:rerun-if-changed={source}");
        let bytes = fs::read(source).expect("dados incorporados devem existir");
        let bytes = if source.ends_with(".txt") {
            String::from_utf8(bytes)
                .expect("dados Unicode devem ser UTF-8")
                .lines()
                .map(|line| line.split_once('#').map_or(line, |(value, _)| value).trim())
                .filter(|line| !line.is_empty())
                .collect::<Vec<_>>()
                .join("\n")
                .into_bytes()
        } else {
            bytes
        };
        let compressed = miniz_oxide::deflate::compress_to_vec(&bytes, 9);
        let filename = PathBuf::from(source)
            .file_name()
            .expect("dados devem ter nome de arquivo")
            .to_owned();
        fs::write(
            output.join(filename).with_added_extension("deflate"),
            compressed,
        )
        .expect("dados comprimidos devem ser gravados");
    }
}
