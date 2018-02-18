fn main() {
    let target = std::env::var("TARGET").unwrap();
    if let Some(_) = target.find("linux") {
        println!("cargo:rustc-link-lib=X11");
    } else if let Some(_) = target.find("darwin") {
        println!("cargo:rustc-link-lib=framework=IOKit");
    }
}
