//! Regenerates the bundled Hugging Face catalog snapshot
//! (`resources/hf-catalog.json`) from the live HF API.
//!
//! Usage: `cargo run --bin hf_snapshot`

use std::path::Path;

fn main() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build tokio runtime");

    let catalog = runtime
        .block_on(dictosaurus_lib::hf_catalog::fetch_from_hf())
        .expect("fetch HF catalog");

    let out = Path::new(env!("CARGO_MANIFEST_DIR")).join("resources/hf-catalog.json");
    std::fs::create_dir_all(out.parent().unwrap()).expect("create resources dir");
    let mut raw = serde_json::to_string_pretty(&catalog).expect("serialize catalog");
    raw.push('\n');
    std::fs::write(&out, raw).expect("write snapshot");

    println!("wrote {} models to {}", catalog.models.len(), out.display());
}
