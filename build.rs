fn main() {
    let target = std::env::var("TARGET").unwrap();
    if target.find("linux").is_some() {
        println!("cargo:rustc-link-lib=X11");
    } else if target.find("darwin").is_some() {
        println!("cargo:rustc-link-lib=framework=IOKit");
    }
}
