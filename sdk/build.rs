use std::process::Command;

fn main() {
    // Pin the codec profile to the fullnode source commit (from git, not a
    // hand-maintained constant) so `profile_hash` rotates with any protocol change. Outside a git checkout it falls back to "unknown" — an identity hint only; the schema hash still pins the wire shapes.
    let manifest = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR");
    let repo = format!("{manifest}/..");
    let commit = Command::new("git")
        .args(["-C", &repo, "rev-parse", "HEAD"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .filter(|commit| !commit.is_empty());
    let commit = commit.unwrap_or_else(|| "unknown".to_owned());
    println!("cargo:rustc-env=SDK_FULLNODE_COMMIT={commit}");
    println!("cargo:rerun-if-changed=../Cargo.toml");
}
