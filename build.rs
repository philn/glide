use std::error::Error;
use vergen::EmitBuilder;

fn main() -> Result<(), Box<dyn Error>> {
    let target = std::env::var("TARGET")?;
    if target.contains("linux") {
        println!("cargo:rustc-link-lib=X11");
    } else if target.contains("darwin") {
        println!("cargo:rustc-link-lib=framework=IOKit");
    }

    Ok(EmitBuilder::builder().all_build().all_git().cargo_features().emit()?)
}
