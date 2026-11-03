//! fitshc build orchestration: `.fitsh` source + `<name>.rs` rust kernels →
//! compiled contract with verified, injected calc containers.
//!
//! Layering (see `vm/doc/fitshc-calc.loc.md`): `vm::fitshc` stays a pure
//! library (tokenizer/parser, `collect_calc_requests`,
//! `compile_with_rv64im`); this crate owns everything that touches the file
//! system or spawns processes (kernel discovery, `// calc:` metadata,
//! wrapper-crate generation, cargo/rustc, packing, verification, caching,
//! manifests). Nodes and consensus never build kernels — only this CLI does.

pub mod meta;
pub mod recipe;
pub mod resolve;
pub mod rustbuild;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::json;
use vm::calc::SpaceCap;
use vm::calc::container::CalcContainer;
use vm::fitshc::compiler::{FitshCompileOutput, compile_with_rv64im};
use vm::fitshc::{CalcRequest, collect_calc_requests};

use crate::meta::KernelMeta;
use crate::resolve::{ElfDefaults, Resolution, resolve_requests};
use crate::rustbuild::KernelBuildRequest;

/// Options for a contract build.
#[derive(Debug, Clone, Default)]
pub struct BuildConfig {
    /// `--calc name=path` overrides.
    pub overrides: BTreeMap<String, PathBuf>,
    /// Skip rust compilation; only use prebuilt `.hcf1`/`.elf` artifacts.
    pub no_build: bool,
    /// Rebuild core/compiler_builtins for the C profiles too.
    pub strict: bool,
    /// IO defaults for bare `.elf` inputs.
    pub elf_defaults: ElfDefaults,
    /// Cache root override (`$FITSHC_CACHE` by default).
    pub cache_root: Option<PathBuf>,
}

/// What was built/fetched for one kernel.
#[derive(Debug, Clone)]
pub struct KernelArtifact {
    pub name: String,
    pub origin: &'static str,
    /// Path of the source of truth (`.rs`, `.hcf1` or `.elf`), when on disk.
    pub source: Option<PathBuf>,
    pub profile_byte: u8,
    pub input_max: u16,
    pub output_max: u16,
    pub mem_kib: u32,
    pub image_len: usize,
    pub code_len: usize,
    pub rodata_len: usize,
    pub init_len: usize,
    pub image_id: String,
    pub cache_hit: bool,
    /// Container bytes as injected into the contract.
    pub container_bytes: Vec<u8>,
    /// Cache/build directory (rust-source kernels only).
    pub build_dir: Option<PathBuf>,
    /// Where the packed `.hcf1` was written next to the fitsh source.
    pub hcf1_path: Option<PathBuf>,
}

/// Compile a `.fitsh` source, resolving every `function rv64im <name>(...)`
/// declaration without an inline payload to container bytes first. The
/// final compile re-runs the full parser, so signature and duplicate checks
/// stay exactly where they are today.
pub fn compile_fitsh_with_calcs(
    source: &str,
    fitsh_path: &Path,
    cfg: &BuildConfig,
    cap: &SpaceCap,
) -> Result<(FitshCompileOutput, Vec<KernelArtifact>, Vec<String>), String> {
    let requests: Vec<CalcRequest> = collect_calc_requests(source).map_err(|e| e.to_string())?;
    let resolutions = resolve_requests(fitsh_path, &requests, &cfg.overrides, cfg.no_build)?;

    let mut pairs: Vec<(String, Vec<u8>)> = Vec::new();
    let mut artifacts = Vec::new();
    let mut warnings = Vec::new();
    for (name, res) in resolutions {
        let origin = res.origin();
        match res {
            Resolution::Rs {
                path,
                meta,
                has_input,
            } => {
                let req = KernelBuildRequest {
                    name: &name,
                    source_path: &path,
                    meta: &meta,
                    has_input,
                    cap,
                    strict: cfg.strict,
                    cache_root: cfg.cache_root.clone(),
                };
                let built = req.build()?;
                warnings.extend(kernel_warnings(
                    &name,
                    &meta,
                    &built.container,
                    built.container_bytes.len(),
                    cap,
                ));
                artifacts.push(KernelArtifact {
                    name: name.clone(),
                    origin,
                    source: Some(path),
                    profile_byte: built.container.profile,
                    input_max: built.container.input_max,
                    output_max: built.container.output_max,
                    mem_kib: built.container.mem_kib,
                    image_len: built.container_bytes.len(),
                    code_len: built.container.code.len(),
                    rodata_len: built.container.rodata.len(),
                    init_len: built.container.init.len(),
                    image_id: hex::encode(built.container.image_id()),
                    cache_hit: built.cache_hit,
                    container_bytes: built.container_bytes.clone(),
                    build_dir: Some(built.build_dir),
                    hcf1_path: None,
                });
                pairs.push((name, built.container_bytes));
            }
            Resolution::Container(path) => {
                let (c, bytes) = crate::resolve::load_container_file(&path, cap)?;
                warnings.extend(kernel_warnings(
                    &name,
                    &KernelMeta::default(),
                    &c,
                    bytes.len(),
                    cap,
                ));
                let artifact = KernelArtifact {
                    name: name.clone(),
                    origin,
                    source: Some(path),
                    profile_byte: c.profile,
                    input_max: c.input_max,
                    output_max: c.output_max,
                    mem_kib: c.mem_kib,
                    image_len: bytes.len(),
                    code_len: c.code.len(),
                    rodata_len: c.rodata.len(),
                    init_len: c.init.len(),
                    image_id: hex::encode(c.image_id()),
                    cache_hit: false,
                    container_bytes: bytes.clone(),
                    build_dir: None,
                    hcf1_path: None,
                };
                artifacts.push(artifact);
                pairs.push((name, bytes));
            }
            Resolution::Elf { path, meta: _ } => {
                let (c, bytes) = crate::resolve::pack_elf_file(
                    &path,
                    &cfg.elf_defaults,
                    crate::meta::MemSpec::Auto,
                    cap,
                )?;
                warnings.extend(kernel_warnings(
                    &name,
                    &KernelMeta::default(),
                    &c,
                    bytes.len(),
                    cap,
                ));
                artifacts.push(KernelArtifact {
                    name: name.clone(),
                    origin,
                    source: Some(path),
                    profile_byte: c.profile,
                    input_max: c.input_max,
                    output_max: c.output_max,
                    mem_kib: c.mem_kib,
                    image_len: bytes.len(),
                    code_len: c.code.len(),
                    rodata_len: c.rodata.len(),
                    init_len: c.init.len(),
                    image_id: hex::encode(c.image_id()),
                    cache_hit: false,
                    container_bytes: bytes.clone(),
                    build_dir: None,
                    hcf1_path: None,
                });
                pairs.push((name, bytes));
            }
        }
    }

    let inject: Vec<(&str, Vec<u8>)> = pairs.iter().map(|(n, b)| (n.as_str(), b.clone())).collect();
    let output = compile_with_rv64im(source, &inject).map_err(|e| e.to_string())?;

    // Side artifacts next to the fitsh source: the packed container (the
    // deployable kernel image) and its build manifest.
    for a in artifacts.iter_mut() {
        let (hcf1, _manifest) = write_kernel_artifacts(fitsh_path, a, cap)?;
        a.hcf1_path = Some(hcf1);
    }
    Ok((output, artifacts, warnings))
}

fn kernel_warnings(
    name: &str,
    _meta: &KernelMeta,
    c: &CalcContainer,
    encoded_len: usize,
    cap: &SpaceCap,
) -> Vec<String> {
    let mut out = Vec::new();
    let lay = c.layout();
    let stack_bytes = lay.stack_top.saturating_sub(lay.stack_bottom);
    if stack_bytes < 8 * 1024 {
        out.push(format!(
            "kernel '{name}': only {} B of stack remain between the guard and the output region; \
             deep recursion or large frames may trap — raise `// calc: mem_kib`",
            stack_bytes
        ));
    }
    if encoded_len * 10 > cap.calc_image_max * 9 {
        out.push(format!(
            "kernel '{name}': encoded image is {encoded_len} B, above 90% of calc_image_max \
             ({} B); code={} rodata={} init={}",
            cap.calc_image_max,
            c.code.len(),
            c.rodata.len(),
            c.init.len()
        ));
    }
    if c.init.len() >= 2048 {
        out.push(format!(
            "kernel '{name}': init image carries {} B of data+bss (zero-initialized data is \
             charged against calc_image_max); prefer a smaller static or the stack gap",
            c.init.len()
        ));
    }
    if c.mem_kib == cap.calc_mem_kib_max {
        out.push(format!(
            "kernel '{name}': mem_kib {} sits at the chain admission cap; a larger kernel needs \
             a sidechain cap",
            c.mem_kib
        ));
    }
    out
}

/// Write the per-kernel deploy artifacts next to the `.fitsh` file:
/// `<name>.hcf1` (the packed container) and `<name>.calc.manifest.json`
/// (build record). Returns the written paths.
pub fn write_kernel_artifacts(
    fitsh_path: &Path,
    artifact: &KernelArtifact,
    cap: &SpaceCap,
) -> Result<(PathBuf, PathBuf), String> {
    let dir = fitsh_path.parent().unwrap_or(Path::new("."));
    let hcf1 = dir.join(format!("{}.hcf1", artifact.name));
    std::fs::write(&hcf1, &artifact.container_bytes)
        .map_err(|e| format!("write {}: {e}", hcf1.display()))?;
    let manifest = dir.join(format!("{}.calc.manifest.json", artifact.name));
    let doc = json!({
        "kernel": artifact.name,
        "origin": artifact.origin,
        "source": artifact.source.as_ref().map(|p| p.display().to_string()),
        "profile_byte": artifact.profile_byte,
        "input_max": artifact.input_max,
        "output_max": artifact.output_max,
        "mem_kib": artifact.mem_kib,
        "image": {
            "code": artifact.code_len,
            "rodata": artifact.rodata_len,
            "init": artifact.init_len,
            "total": artifact.image_len,
            "id": artifact.image_id,
        },
        "cap": {
            "call_data_size": cap.call_data_size,
            "value_size": cap.value_size,
            "calc_mem_kib_max": cap.calc_mem_kib_max,
            "calc_image_max": cap.calc_image_max,
        },
        "cache_hit": artifact.cache_hit,
        "build_dir": artifact.build_dir.as_ref().map(|p| p.display().to_string()),
    });
    std::fs::write(
        &manifest,
        serde_json::to_string_pretty(&doc).unwrap_or_default(),
    )
    .map_err(|e| format!("write {}: {e}", manifest.display()))?;
    Ok((hcf1, manifest))
}

/// Execute a resolved container locally (the `fitshc run` smoke test):
/// interpreter+AOT exactly like a node would.
pub fn run_container(
    container: CalcContainer,
    input: &[u8],
    cycles: u64,
) -> Result<(Vec<u8>, u64, u64), String> {
    let cap = SpaceCap::new(0);
    let obj = vm::calc::CalcFnObj::build(container, &cap, vm::calc::CalcHostConfig::new())
        .map_err(|e| format!("verify/build kernel: {e}"))?;
    let limits = vm::calc::CalcRunLimits {
        cycles,
        resource: i64::MAX,
        deadline: None,
    };
    let exec = obj.run(input, limits);
    match exec.result {
        Ok(out) => Ok((out, exec.cycles_used, exec.resource.max(0) as u64)),
        Err(e) => Err(format!("kernel run failed: {e:?}")),
    }
}

#[cfg(test)]
mod lib_tests {
    use super::*;

    #[test]
    fn inline_only_contract_builds_without_any_files() {
        // No requests → identical to plain compile_with_rv64im.
        let src = "pragma fitsh 1.0.0\ncontract D {\n    function external f(x: bytes) -> bytes { return x }\n}\n";
        let cfg = BuildConfig::default();
        let (out, artifacts, _) =
            compile_fitsh_with_calcs(src, Path::new("/tmp/none.fitsh"), &cfg, &SpaceCap::new(0))
                .unwrap();
        assert!(artifacts.is_empty());
        assert_eq!(out.3, "D");
    }

    #[test]
    fn missing_kernel_error_lists_paths() {
        let src =
            "pragma fitsh 1.0.0\ncontract D {\n    function rv64im hash(data: bytes) -> bytes\n}\n";
        let cfg = BuildConfig::default();
        let err = match compile_fitsh_with_calcs(
            src,
            Path::new("/tmp/definitely-missing-dir/c.fitsh"),
            &cfg,
            &SpaceCap::new(0),
        ) {
            Ok(_) => panic!("expected resolution failure"),
            Err(e) => e,
        };
        assert!(err.contains("hash.rs"), "{err}");
    }
}
