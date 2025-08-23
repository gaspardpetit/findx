fn main() {
    let default_version = env!("CARGO_PKG_VERSION");
    let version = std::env::var("FINDX_VERSION").unwrap_or_else(|_| default_version.to_string());
    println!("cargo:rustc-env=FINDX_VERSION={}", version);
}
