use std::path::Path;

fn main() {
    release_notes();
    tauri_build::build()
}

/// Bundles every version's release notes (`docs/releases/<version>.md`)
/// as `RELEASES`, so the app can offer players a "What's new" post from
/// their "For players" sections (see `src/whatsnew.rs`).
fn release_notes() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/releases");
    println!("cargo:rerun-if-changed={}", dir.display());
    let mut entries = Vec::new();
    for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
        let path = entry.path();
        let Some(version) = path.file_name().and_then(|n| n.to_str()).and_then(|n| n.strip_suffix(".md")) else {
            continue;
        };
        if version.split('.').count() != 3 || !version.split('.').all(|p| p.parse::<u32>().is_ok()) {
            continue;
        }
        println!("cargo:rerun-if-changed={}", path.display());
        let path = path.canonicalize().unwrap();
        entries.push(format!("({version:?}, include_str!({:?})),\n", path.display().to_string()));
    }
    entries.sort();
    let code = format!("pub const RELEASES: &[(&str, &str)] = &[\n{}];\n", entries.concat());
    let out = Path::new(&std::env::var("OUT_DIR").unwrap()).join("releases.rs");
    std::fs::write(out, code).unwrap();
}
