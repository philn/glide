fn main() {
    let target = std::env::var("TARGET").unwrap();
    if target.contains("linux") {
        println!("cargo:rustc-link-lib=X11");
    } else if target.contains("darwin") {
        println!("cargo:rustc-link-lib=framework=IOKit");
    }
}
