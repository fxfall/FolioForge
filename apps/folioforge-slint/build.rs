fn main() {
    let translations =
        std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("lang");
    let configuration = slint_build::CompilerConfiguration::new()
        .with_style("native".into())
        .with_bundled_translations(translations.to_string_lossy().into_owned());
    slint_build::compile_with_config("ui/main.slint", configuration)
        .expect("Slint UI compilation failed");
}
