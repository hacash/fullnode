//! `fitshc image` admission. These cases fail in metadata validation, before
//! rustc, and must not create the output file.

use std::path::PathBuf;
use std::process::Command;

fn project(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fitshc-image-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn run(dir: &std::path::Path, src: &str) -> (bool, String, PathBuf) {
    let rs = dir.join("kern.rs");
    std::fs::write(&rs, src).unwrap();
    let out = dir.join("kern.hcf1");
    let exe = env!("CARGO_BIN_EXE_fitshc");
    let got = Command::new(exe)
        .arg("image")
        .arg(&rs)
        .arg("-o")
        .arg(&out)
        .arg("--no-cache")
        .output()
        .unwrap();
    let text = format!(
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&got.stdout),
        String::from_utf8_lossy(&got.stderr)
    );
    (got.status.success(), text, out)
}

#[test]
fn image_rejects_hash_and_writes_nothing() {
    let dir = project("hash");
    let (ok, text, out) = run(
        &dir,
        "// calc: hash = true\n\
         pub fn kern(input: &[u8], out: &mut [u8]) -> usize { 0 }\n",
    );
    assert!(!ok, "{text}");
    assert!(text.contains("hash"), "{text}");
    assert!(
        !out.exists(),
        "rejected image must not create {}",
        out.display()
    );
}

#[test]
fn image_refuses_to_overwrite_the_source() {
    let dir = project("clobber");
    let rs = dir.join("kern.rs");
    let body = "pub fn kern(input: &[u8], out: &mut [u8]) -> usize { 0 }\n";
    std::fs::write(&rs, body).unwrap();
    let exe = env!("CARGO_BIN_EXE_fitshc");
    let got = Command::new(exe)
        .current_dir(&dir)
        .arg("image")
        .arg("kern.rs")
        .arg("-o")
        .arg("./kern.rs")
        .output()
        .unwrap();
    assert!(!got.status.success());
    assert_eq!(std::fs::read_to_string(&rs).unwrap(), body);
}

#[test]
fn image_rejects_output_max_over_value_size() {
    let dir = project("outmax");
    let (ok, text, out) = run(
        &dir,
        "// calc: output_max = 2000\n\
         pub fn kern(input: &[u8], out: &mut [u8]) -> usize { 0 }\n",
    );
    assert!(!ok, "{text}");
    assert!(
        text.contains("output_max") || text.contains("value_size"),
        "{text}"
    );
    assert!(
        !out.exists(),
        "rejected image must not create {}",
        out.display()
    );
}
