use std::{env, fs, path::PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=rp2350.x");
    println!("cargo:rerun-if-changed=build.rs");
    let target = env::var("TARGET").unwrap();
    if target == "thumbv8m.main-none-eabihf" {
        let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
        fs::write(out.join("memory.x"), include_bytes!("rp2350.x")).unwrap();
        println!("cargo:rustc-link-search={}", out.display());
    }
}
