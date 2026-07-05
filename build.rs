use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR")?);
    let framework_dir = manifest_dir.join("vendor");
    let framework = framework_dir.join("Sparkle.framework");
    if !framework.exists() {
        return Err(format!("Sparkle.framework not found at {}", framework.display()).into());
    }

    // Expose the framework search path to every target in the crate. The
    // actual `-framework Sparkle` link directive lives in sparkle.rs so it
    // only applies to the main tray binary — linking Sparkle into the setuid
    // led-helper triggers macOS library-validation and blocks its launch.
    println!(
        "cargo:rustc-link-search=framework={}",
        framework_dir.display()
    );

    // At runtime, resolve @rpath to the app's embedded Frameworks directory.
    // bundle.sh copies Sparkle.framework into LED.app/Contents/Frameworks/.
    println!("cargo:rustc-link-arg=-Wl,-rpath,@executable_path/../Frameworks");
    if std::env::var("PROFILE").as_deref() == Ok("debug") {
        println!(
            "cargo:rustc-link-arg=-Wl,-rpath,{}",
            framework_dir.display()
        );
    }

    println!("cargo:rerun-if-changed=vendor/Sparkle.framework");
    println!("cargo:rerun-if-changed=build.rs");
    Ok(())
}
