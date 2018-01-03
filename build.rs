fn main() {
    let target = std::env::var("TARGET").unwrap();
    if let Some(_) = target.find("linux") {
        println!("cargo:rustc-link-lib=X11");
    }
}
