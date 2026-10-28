//! Writes the UI's generated TypeScript from the Rust types.

fn main() -> std::io::Result<()> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join(octopus_desktop::types::PATH);
    std::fs::write(&path, octopus_desktop::types::render())?;
    println!("wrote {}", path.display());
    Ok(())
}
