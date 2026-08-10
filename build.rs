use std::error::Error;
use vergen_gitcl::{Build, Cargo, Emitter, Gitcl};

fn main() -> Result<(), Box<dyn Error>> {
    let target = std::env::var("TARGET")?;
    if target.contains("linux") {
        println!("cargo:rustc-link-lib=X11");
    } else if target.contains("darwin") {
        println!("cargo:rustc-link-lib=framework=IOKit");
    }

    Ok(Emitter::default()
        .add_instructions(&Build::all_build())?
        .add_instructions(&Cargo::all_cargo())?
        .add_instructions(&Gitcl::all_git())?
        .emit()?)
}
