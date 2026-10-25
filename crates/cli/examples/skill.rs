//! Writes the octopus skill from the command tree.

fn main() -> std::io::Result<()> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join(octopus_cli::skill::PATH);
    std::fs::create_dir_all(path.parent().expect("the skill lives in a directory"))?;
    std::fs::write(&path, octopus_cli::skill::render(&octopus_cli::commands::tree()))?;
    println!("wrote {}", path.display());
    Ok(())
}
