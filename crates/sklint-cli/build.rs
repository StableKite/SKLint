use std::env;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-env-changed=SKLINT_SOURCE_TREE_SHA256");
    println!("cargo:rerun-if-env-changed=SKLINT_BUILD_REVISION");
    println!("cargo:rerun-if-env-changed=SKLINT_SOURCE_COMMIT");
    println!("cargo:rerun-if-env-changed=SKLINT_SOURCE_DIRTY");

    let rustc = env::var("RUSTC").unwrap_or_else(|_| "rustc".to_string());
    let rustc_version = Command::new(&rustc)
        .arg("--version")
        .output()
        .ok()
        .filter(|out| out.status.success())
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
        .unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=SKLINT_RUSTC_VERSION={rustc_version}");

    let target = env::var("TARGET").unwrap_or_else(|_| "unknown".to_string());
    println!("cargo:rustc-env=SKLINT_BUILD_TARGET={target}");

    for (key, default) in [
        ("SKLINT_SOURCE_TREE_SHA256", "unknown"),
        ("SKLINT_BUILD_REVISION", "development"),
        ("SKLINT_SOURCE_COMMIT", "unknown"),
        ("SKLINT_SOURCE_DIRTY", "unknown"),
    ] {
        let value = env::var(key).unwrap_or_else(|_| default.to_string());
        println!("cargo:rustc-env={key}={value}");
    }
}
