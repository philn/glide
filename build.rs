use std::error::Error;
use vergen_gitcl::{BuildBuilder, CargoBuilder, Emitter, GitclBuilder};

fn main() -> Result<(), Box<dyn Error>> {
    let target = std::env::var("TARGET")?;
    if target.contains("linux") {
        println!("cargo:rustc-link-lib=X11");
    } else if target.contains("darwin") {
        println!("cargo:rustc-link-lib=framework=IOKit");
    }

    Ok(Emitter::default()
        .add_instructions(&BuildBuilder::all_build()?)?
        .add_instructions(&CargoBuilder::all_cargo()?)?
        .add_instructions(&GitclBuilder::all_git()?)?
        .emit()?)
}
