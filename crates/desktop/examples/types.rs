//! Writes the UI's generated TypeScript from the Rust types.

fn main() -> std::io::Result<()> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for (path, rendered) in [
        (octopus_shell::types::PATH, octopus_shell::types::render()),
        (octopus_desktop::types::PATH, octopus_desktop::types::render()),
    ] {
        let path = root.join(path);
        std::fs::write(&path, rendered)?;
        println!("wrote {}", path.display());
    }
    Ok(())
}
