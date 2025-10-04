use std::path::PathBuf;

fn main() {
    println!("cargo:rustc-link-lib=flutter_engine");

    let engine_dir = PathBuf::from("./engine")
        .canonicalize()
        .expect("unable to get the absolute path of engine");

    println!("cargo:rustc-link-search={}", engine_dir.display());

    let bindings = bindgen::builder()
        .header("engine/embedder.h")
        .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()))
        .generate()
        .expect("unable to generate bindings");

    let out_path = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    bindings
        .write_to_file(out_path.join("embedder_bindings.rs"))
        .expect("failed to write embedder_bindings.rs");
}
