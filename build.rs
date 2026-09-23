use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::var_os("CARGO_FEATURE_DIRECT").is_some()
        && std::env::var_os("CARGO_FEATURE_FIELD_TEST").is_none()
    {
        let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR")?);
        let framework = manifest_dir.join("vendor/Sparkle.framework");
        if !framework.exists() {
            return Err(format!("Sparkle.framework not found at {}", framework.display()).into());
        }
        // Sparkle is loaded explicitly via NSBundle at startup. Keep an rpath
        // for any libraries referenced by its embedded executable components.
        println!("cargo:rustc-link-arg=-Wl,-rpath,@executable_path/../Frameworks");
    }

    println!("cargo:rerun-if-changed=vendor/Sparkle.framework");
    println!("cargo:rerun-if-changed=build.rs");
    Ok(())
}
