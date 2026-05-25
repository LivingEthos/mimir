use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let workspace_root = manifest_dir.parent().unwrap().parent().unwrap();
    let schemas_dir = workspace_root.join("schemas");

    let mut all_schemas = Vec::new();

    for entry in fs::read_dir(&schemas_dir).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) == Some("json") {
            let content = fs::read_to_string(&path).unwrap();
            all_schemas.push(content);
        }
    }

    // For now, write a placeholder generated.rs since typify needs proper
    // schema handling. We'll generate real types once schemas are stable.
    let generated_path = out_dir.join("generated.rs");
    fs::write(
        &generated_path,
        r#"
// Generated from JSON Schemas — placeholder for Phase 0
// Full generation will be wired once schemas are frozen.

use serde::{Deserialize, Serialize};

/// Placeholder schema version type.
pub type SchemaVersion = u32;
"#,
    )
    .unwrap();

    println!("cargo:rerun-if-changed={}", schemas_dir.display());
}
