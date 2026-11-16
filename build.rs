fn main() {
    println!("cargo:rerun-if-changed=native/Preview.swift");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }
    let output = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    let arch = match std::env::var("CARGO_CFG_TARGET_ARCH").unwrap().as_str() {
        "aarch64" => "arm64",
        "x86_64" => "x86_64",
        other => panic!("Unsupported macOS architecture: {other}"),
    };
    let status = std::process::Command::new("xcrun")
        .args([
            "swiftc",
            "-O",
            "-target",
            &format!("{arch}-apple-macosx14.4"),
        ])
        .arg("-module-cache-path")
        .arg(output.join("swift-cache"))
        .arg("native/Preview.swift")
        .arg("-o")
        .arg(output.join("switchboard-preview"))
        .status()
        .expect("Swift compiler is required for macOS document previews");
    assert!(
        status.success(),
        "Could not build native document preview helper"
    );
}
