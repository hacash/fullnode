//! P2-D03 error-message regression tests: process-level CLI assertions on the
//! error channel (stderr vs stdout), message text and exit codes.
//!
//! Each test documents the pre-change behavior in a comment (the doc 66 §5
//! experiment it corresponds to) and pins the post-change contract.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_fitshc"))
}

fn tmp_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("d03-{}-{}", tag, std::process::id()));
    fs::create_dir_all(&d).unwrap();
    d
}

fn write(dir: &PathBuf, name: &str, body: &str) -> PathBuf {
    let p = dir.join(name);
    fs::write(&p, body).unwrap();
    p
}

/// E02/T3: a compile error must be reported on **stderr** (was: stdout via
/// `println!("Error: {e}")`, invisible under `2>/dev/null`), carry the
/// `fitshc: ` prefix, and exit 1.
#[test]
fn compile_error_goes_to_stderr_with_prefix_and_exit_1() {
    let dir = tmp_dir("stderr");
    // Missing closing brace: contract body hits EOF (E02 shape).
    let src = "pragma fitsh 1.0.0\ncontract D {\n    function external f(x: bytes) -> bytes { return x }\n";
    let f = write(&dir, "broken.fitsh", src);
    let out = bin()
        .arg("build")
        .arg(&f)
        .output()
        .expect("spawn fitshc");
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert_ne!(out.status.code(), Some(0), "must exit nonzero");
    assert!(
        stderr.starts_with("fitshc: "),
        "stderr must carry the fitshc: prefix, got: {stderr:?}"
    );
    assert!(!stderr.contains("Error: "), "old prefix must be gone: {stderr:?}");
    assert!(
        !stdout.contains("fitshc: "),
        "error text must not leak to stdout: {stdout:?}"
    );
    fs::remove_dir_all(&dir).ok();
}

/// T3: the success surface is unchanged — `Compile success!` stays on stdout.
#[test]
fn success_path_still_prints_to_stdout() {
    let dir = tmp_dir("okpath");
    let src = "pragma fitsh 1.0.0\ncontract D {\n    function external f(x: bytes) -> bytes { return x }\n}\n";
    let f = write(&dir, "ok.fitsh", src);
    let out = bin()
        .arg("build")
        .arg(&f)
        .arg("--no-artifacts")
        .arg("--out-dir")
        .arg(&dir)
        .output()
        .expect("spawn fitshc");
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert_eq!(out.status.code(), Some(0), "stderr: {stderr}");
    assert!(stdout.contains("Compile success!"), "{stdout}");
    assert!(!stderr.contains("fitshc: "), "{stderr}");
    fs::remove_dir_all(&dir).ok();
}

// More T-item tests are appended per commit (T4, T5, T10) as each lands.

/// T8/E19 companion: a bad kernel extension names the extension plainly
/// (was `unsupported kernel type Some("ktxt"): ...`).
#[test]
fn unsupported_kernel_type_is_not_debug() {
    let dir = tmp_dir("kerneltype");
    let f = write(&dir, "kernel.ktxt", "not a kernel");
    let out = bin().arg("run").arg(&f).output().expect("spawn fitshc");
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert_ne!(out.status.code(), Some(0));
    assert!(
        stderr.contains("unsupported kernel type '.ktxt'"),
        "{stderr:?}"
    );
    assert!(!stderr.contains("Some("), "Debug leak: {stderr:?}");
    fs::remove_dir_all(&dir).ok();
}

/// T8: unknown flags name the flag and point at --help (was
/// `unknown flag "--turbo"` with Debug quoting, no hint).
#[test]
fn unknown_flag_names_flag_and_help() {
    let dir = tmp_dir("flag");
    let f = write(&dir, "ok.fitsh", "pragma fitsh 1.0.0\ncontract D {\n    function external f(x: bytes) -> bytes { return x }\n}\n");
    let out = bin()
        .arg("build")
        .arg(&f)
        .arg("--turbo")
        .output()
        .expect("spawn fitshc");
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert_ne!(out.status.code(), Some(0));
    assert!(
        stderr.contains("unknown flag '--turbo'") && stderr.contains("--help"),
        "{stderr:?}"
    );
    fs::remove_dir_all(&dir).ok();
}

/// T4/E20: an arithmetic-fault kernel (`0u64 - 1u64` → exit code 90) must
/// decode the exit code in the message (was: `Error: kernel run failed: Trap`).
#[test]
fn run_reports_arithmetic_fault_exit_code_90() {
    let dir = tmp_dir("exit90");
    let f = write(
        &dir,
        "ovf.fitsh",
        "pragma fitsh 1.0.0\ncontract D {\n    function rv64im k() -> bytes {\n        var a = 0u64 - 1u64\n        return a as bytes\n    }\n}\n",
    );
    let out = bin().arg("run").arg(&f).output().expect("spawn fitshc");
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert_ne!(out.status.code(), Some(0));
    assert!(
        stderr.contains("arithmetic fault") && stderr.contains("90"),
        "exit code 90 must be decoded, got: {stderr:?}"
    );
    assert!(
        !stderr.contains(": Trap") && !stderr.contains("Trap\n") && !stderr.ends_with("Trap"),
        "bare Debug `Trap` must be gone: {stderr:?}"
    );
    fs::remove_dir_all(&dir).ok();
}

/// T4: a cast-failure kernel (exit code 71) decodes distinctly from 90.
/// `size(data)` is runtime (1 with the `--input aa`), so `+ 300u64 as u8`
/// overflows the u8 cast at runtime → EXIT_CAST.
#[test]
fn run_reports_cast_failure_exit_code_71() {
    let dir = tmp_dir("exit71");
    let f = write(
        &dir,
        "cast.fitsh",
        "pragma fitsh 1.0.0\ncontract D {\n    function rv64im k(data: bytes) -> bytes {\n        var a = (size(data) + 300u64) as u8\n        return a as bytes\n    }\n}\n",
    );
    let out = bin()
        .arg("run")
        .arg(&f)
        .arg("--input")
        .arg("aa")
        .output()
        .expect("spawn fitshc");
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert_ne!(out.status.code(), Some(0));
    assert!(
        stderr.contains("cast failure") && stderr.contains("71"),
        "exit code 71 must be decoded, got: {stderr:?}"
    );
    fs::remove_dir_all(&dir).ok();
}

/// T10/E10: `--cycles abc` explains the expectation (was: bare `--cycles N`).
#[test]
fn cycles_flag_reports_expected_format() {
    let dir = tmp_dir("cycles");
    let f = write(
        &dir,
        "k.fitsh",
        "pragma fitsh 1.0.0\ncontract D {\n    function rv64im k() -> bytes { return 0x00 }\n}\n",
    );
    let out = bin()
        .arg("run")
        .arg(&f)
        .arg("--cycles")
        .arg("abc")
        .output()
        .expect("spawn fitshc");
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert_ne!(out.status.code(), Some(0));
    assert!(
        stderr.contains("--cycles") && stderr.contains("integer") && stderr.contains("'abc'"),
        "must name the flag, the expectation and the offending value: {stderr:?}"
    );
    fs::remove_dir_all(&dir).ok();
}

/// T10/E16: `--input zz` names the hex expectation (was: bare hex-crate error).
#[test]
fn input_flag_reports_hex_expectation() {
    let dir = tmp_dir("input");
    let f = write(
        &dir,
        "k.fitsh",
        "pragma fitsh 1.0.0\ncontract D {\n    function rv64im k(data: bytes) -> bytes { return data }\n}\n",
    );
    let out = bin()
        .arg("run")
        .arg(&f)
        .arg("--input")
        .arg("zz")
        .output()
        .expect("spawn fitshc");
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert_ne!(out.status.code(), Some(0));
    assert!(
        stderr.contains("--input") && stderr.contains("hex") && stderr.contains("'zz'"),
        "must name flag, expectation and value: {stderr:?}"
    );
    fs::remove_dir_all(&dir).ok();
}

/// T10: missing positional file args point at --help on every subcommand.
#[test]
fn missing_file_arguments_point_at_help() {
    for sub in ["pack", "run"] {
        let out = bin().arg(sub).output().expect("spawn fitshc");
        let stderr = String::from_utf8_lossy(&out.stderr).to_string();
        assert_ne!(out.status.code(), Some(0));
        assert!(
            stderr.contains("requires a") && stderr.contains("(try --help)"),
            "{sub}: {stderr:?}"
        );
    }
}
