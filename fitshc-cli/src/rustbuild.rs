//! Kernel build engine: generated wrapper crate → cargo/rustc → ELF →
//! packed, verified container, with a content-addressed cache.
//!
//! Layout of a build directory (the cache entry lives at
//! `<cache_root>/k-<key16>/`, keyed by a hash over every build input, so a
//! hit is always a match for the exact request):
//!
//! ```text
//! <cache_root>/k-<key>/
//!   Cargo.toml            wrapper crate manifest (fixed template)
//!   src/main.rs           generated wrapper (`#[path]` → user's file)
//!   container.ld          fixed linker script
//!   <target>.json         custom target spec (build-std recipes)
//!   target/               cargo target dir
//!   out/kernel.elf        linked ELF (kept for inspection / `pack`)
//!   out/kernel.hcf1       packed container (the cache payload)
//!   out/manifest.json     toolchain + image record
//! ```
//!
//! Cache key: kernel source bytes + parsed metadata + recipe/metadata-schema
//! versions + recipe inputs + flags + rustc/cargo versions + the calc-relevant
//! `SpaceCap` fields. A hit skips cargo entirely; the stored container is
//! re-decoded and re-verified (cheap) before reuse, so a corrupted cache entry
//! fails loudly instead of injecting garbage.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::json;

use vm::calc::SpaceCap;
use vm::calc::container::CalcContainer;
use vm::calc::verify::verify_container_with_cap;
use vm::calc::{pack_elf, pack_elf_min_mem, sha256_bytes};

use crate::meta::{KernelMeta, MemSpec};
use crate::recipe::{
    RECIPE_VERSION, Recipe, WRAPPER_CARGO_TOML, base_rustflags, cargo_extra_args, wrapper_source,
};

/// Default cache root: `$FITSHC_CACHE`, else `$XDG_CACHE_HOME|~/.cache` +
/// `fitshc`, else the system temp dir.
pub fn default_cache_root() -> PathBuf {
    if let Some(p) = std::env::var_os("FITSHC_CACHE") {
        return PathBuf::from(p);
    }
    if let Some(home) = std::env::var_os("XDG_CACHE_HOME") {
        if !home.is_empty() {
            return PathBuf::from(home).join("fitshc");
        }
    }
    if let Some(home) = std::env::var_os("HOME") {
        if !home.is_empty() {
            return PathBuf::from(home).join(".cache").join("fitshc");
        }
    }
    std::env::temp_dir().join("fitshc-cache")
}

/// Tool binary with env override; the value may carry arguments, e.g.
/// `FITSHC_CARGO='cargo +nightly'`.
fn tool(bin_env: &str, default: &str) -> Vec<String> {
    match std::env::var(bin_env) {
        Ok(v) if !v.trim().is_empty() => v.split_whitespace().map(String::from).collect(),
        _ => vec![default.to_string()],
    }
}

fn run_tool(cmd: &mut Command, what: &str) -> Result<String, String> {
    let out = cmd
        .output()
        .map_err(|e| format!("failed to run {what}: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "{what} failed (exit {}):\n{}",
            out.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&out.stderr).trim_end()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn rustc_version() -> Result<String, String> {
    let mut parts = tool("FITSHC_RUSTC", "rustc");
    let mut cmd = Command::new(parts.remove(0));
    cmd.args(parts).arg("--version");
    run_tool(&mut cmd, "rustc --version")
}

fn cargo_version() -> Result<String, String> {
    let mut parts = tool("FITSHC_CARGO", "cargo");
    let mut cmd = Command::new(parts.remove(0));
    cmd.args(parts).arg("--version");
    run_tool(&mut cmd, "cargo --version")
}

/// Everything the builder needs for one kernel.
pub struct KernelBuildRequest<'a> {
    /// fitsh function name (also the Rust function name users must define).
    pub name: &'a str,
    /// Absolute path of the user's Rust source (`<name>.rs`).
    pub source_path: &'a Path,
    /// Parsed `// calc:` metadata (already validated against `cap`).
    pub meta: &'a KernelMeta,
    /// Whether the fitsh declaration passes call data (`kernel(data: bytes)`).
    pub has_input: bool,
    pub cap: &'a SpaceCap,
    /// Rebuild core/compiler_builtins even for the C profiles (drops the
    /// prebuilt-`+A` core caveat of the builtin target).
    pub strict: bool,
    pub cache_root: Option<PathBuf>,
}

/// Result of a kernel build.
pub struct KernelBuildResult {
    pub container: CalcContainer,
    pub container_bytes: Vec<u8>,
    pub elf: Vec<u8>,
    pub elf_sha256: String,
    pub mem_kib: u32,
    pub cache_key: String,
    pub cache_hit: bool,
    pub build_dir: PathBuf,
}

impl KernelBuildRequest<'_> {
    fn expected_signature(&self) -> String {
        if self.has_input {
            format!(
                "pub fn {}(input: &[u8], out: &mut [u8]) -> usize",
                self.name
            )
        } else {
            format!("pub fn {}(out: &mut [u8]) -> usize", self.name)
        }
    }

    fn cache_key(&self, recipe: &Recipe, rustflags: &str, versions: &(String, String)) -> String {
        let source = std::fs::read(self.source_path).unwrap_or_default();
        let mut key = Vec::new();
        key.extend_from_slice(b"fitshc-kernel-v1\0");
        key.extend_from_slice(
            format!(
                "recipe={RECIPE_VERSION} meta_schema={} strict={} name={} has_input={}\0",
                crate::meta::META_SCHEMA_VERSION,
                self.strict,
                self.name,
                self.has_input
            )
            .as_bytes(),
        );
        key.extend_from_slice(self.meta.cache_key_text().as_bytes());
        key.push(0);
        key.extend_from_slice(&sha256_bytes(&source));
        key.push(0);
        key.extend_from_slice(recipe.id.as_bytes());
        key.push(0);
        key.extend_from_slice(rustflags.as_bytes());
        key.push(0);
        key.extend_from_slice(crate::recipe::CONTAINER_LD.as_bytes());
        key.push(0);
        if let Some((name, spec)) = recipe.target_json.as_ref() {
            key.extend_from_slice(name.as_bytes());
            key.extend_from_slice(spec.as_bytes());
        }
        key.push(0);
        key.extend_from_slice(crate::recipe::WRAPPER_CARGO_TOML.as_bytes());
        key.push(0);
        key.extend_from_slice(versions.0.as_bytes());
        key.push(b'/');
        key.extend_from_slice(versions.1.as_bytes());
        key.push(0);
        key.extend_from_slice(
            format!(
                "mem_kib_max={} image_max={} call_data={} value={}\0",
                self.cap.calc_mem_kib_max,
                self.cap.calc_image_max,
                self.cap.call_data_size,
                self.cap.value_size
            )
            .as_bytes(),
        );
        hex::encode(&sha256_bytes(&key)[..8])
    }

    /// Build (or fetch from cache) the kernel container.
    pub fn build(&self) -> Result<KernelBuildResult, String> {
        self.build_at(
            &self
                .source_path
                .canonicalize()
                .map_err(|e| format!("resolve kernel path {}: {e}", self.source_path.display()))?,
        )
    }

    /// [`Self::build`] with an already-canonical source path.
    fn build_at(&self, source_path: &Path) -> Result<KernelBuildResult, String> {
        self.meta.validate(self.cap).map_err(|e| {
            format!(
                "kernel '{}' metadata in {} is not admissible: {e}",
                self.name,
                source_path.display()
            )
        })?;
        let recipe = self.meta.profile.recipe(self.strict);
        let versions = (rustc_version()?, cargo_version()?);
        let cache_root = self.cache_root.clone().unwrap_or_else(default_cache_root);

        // rustflags depend on the build dir only through the `-T` script
        // path, which never affects image bytes — key on the placeholder
        // form, run with the absolute one.
        let mut flags = base_rustflags(Path::new("container.ld"), source_path);
        if let Some(tf) = recipe.target_features {
            flags.push(format!("-C target-feature={tf}"));
        }
        let flags = flags.join(" ");
        let key = self.cache_key(&recipe, &flags, &versions);
        let build_dir = cache_root.join(format!("k-{key}"));
        let out_dir = build_dir.join("out");
        let hcf1_path = out_dir.join("kernel.hcf1");
        let key_file = out_dir.join("cache.key");
        let ld_path = build_dir.join("container.ld");
        let rustflags = flags.replace(
            "-C link-arg=-Tcontainer.ld",
            &format!("-C link-arg=-T{}", ld_path.display()),
        );

        if hcf1_path.is_file() && key_file.is_file() {
            let stored = std::fs::read_to_string(&key_file).unwrap_or_default();
            if stored == key {
                if let Ok(bytes) = std::fs::read(&hcf1_path) {
                    if let Ok((c, used)) = CalcContainer::decode(&bytes) {
                        if used == bytes.len() && verify_container_with_cap(&c, self.cap).is_ok() {
                            let elf = std::fs::read(out_dir.join("kernel.elf")).unwrap_or_default();
                            let mem_kib = c.mem_kib;
                            return Ok(KernelBuildResult {
                                container_bytes: bytes,
                                container: c,
                                elf_sha256: hex::encode(sha256_bytes(&elf)),
                                elf,
                                mem_kib,
                                cache_key: key,
                                cache_hit: true,
                                build_dir,
                            });
                        }
                    }
                }
                // Corrupt/stale entry: rebuild in place.
            }
        }

        // Materialize the wrapper crate. The user's file is referenced by
        // absolute `#[path]`, never copied, so `mod helpers;` and rustc
        // diagnostics keep pointing at the original file.
        let src_dir = build_dir.join("src");
        std::fs::create_dir_all(&src_dir)
            .map_err(|e| format!("create {}: {e}", src_dir.display()))?;
        let wrapper = wrapper_source(
            &source_path.display().to_string(),
            self.name,
            self.has_input,
        );
        write_file(&src_dir.join("main.rs"), &wrapper)?;
        write_file(&build_dir.join("Cargo.toml"), WRAPPER_CARGO_TOML)?;
        write_file(&ld_path, crate::recipe::CONTAINER_LD)?;
        let target_arg = if let Some((name, spec)) = recipe.target_json.as_ref() {
            write_file(&build_dir.join(name), spec)?;
            build_dir.join(name).display().to_string()
        } else {
            recipe.target.clone()
        };

        let mut argv = tool("FITSHC_CARGO", "cargo");
        let mut cmd = Command::new(argv.remove(0));
        cmd.args(&argv)
            .arg("build")
            .arg("--release")
            .arg("--target")
            .arg(&target_arg)
            .args(cargo_extra_args(&recipe))
            .env("RUSTFLAGS", &rustflags)
            .env("CARGO_TARGET_DIR", build_dir.join("target"))
            .current_dir(&build_dir);
        if recipe.build_std {
            cmd.env("RUSTC_BOOTSTRAP", "1");
        }
        let out = cmd
            .output()
            .map_err(|e| format!("failed to run cargo: {e}"))?;
        if !out.status.success() {
            return Err(format!(
                "rustc failed building kernel '{}' from {} — it must define `{}`:\n{}",
                self.name,
                self.source_path.display(),
                self.expected_signature(),
                String::from_utf8_lossy(&out.stderr).trim_end()
            ));
        }

        // cargo names the output dir after the target triple / JSON stem.
        let triple_dir = target_arg
            .rsplit('/')
            .next()
            .unwrap_or(&recipe.target)
            .trim_end_matches(".json");
        let elf_path = build_dir
            .join("target")
            .join(triple_dir)
            .join("release")
            .join("calc_kernel");
        let elf = std::fs::read(&elf_path).map_err(|e| {
            format!(
                "read built ELF {}: {e} (cargo target dir was {triple_dir})",
                elf_path.display()
            )
        })?;

        // Pack + verify under the target chain cap — the same checks deploy
        // runs, so a local pass means the chain accepts the container.
        let container = match self.meta.mem_kib {
            MemSpec::Auto => pack_elf_min_mem(
                self.cap,
                &elf,
                self.meta.profile_byte(),
                self.meta.input_max,
                self.meta.output_max,
            ),
            MemSpec::KiB(kib) => pack_elf(
                self.cap,
                &elf,
                self.meta.profile_byte(),
                kib,
                self.meta.input_max,
                self.meta.output_max,
            ),
        }
        .map_err(|e| self.translate_pack_error(e.to_string()))?;
        verify_container_with_cap(&container, self.cap)
            .map_err(|e| self.translate_pack_error(e.to_string()))?;
        let container_bytes = container
            .encode()
            .map_err(|e| format!("encode container: {e}"))?;
        let container_mem = container.mem_kib;

        std::fs::create_dir_all(&out_dir)
            .map_err(|e| format!("create {}: {e}", out_dir.display()))?;
        write_file(&hcf1_path, &container_bytes)?;
        write_file(&out_dir.join("kernel.elf"), &elf)?;
        write_file(&key_file, &key)?;
        let manifest = json!({
            "kernel": self.name,
            "wrapper_abi": 1,
            "recipe": recipe.id,
            "toolchain": { "rustc": versions.0, "cargo": versions.1 },
            "meta": manifest_meta(self.meta),
            "profile_byte": self.meta.profile_byte(),
            "source_sha256": hex::encode(sha256_bytes(&std::fs::read(source_path).unwrap_or_default())),
            "elf_sha256": hex::encode(sha256_bytes(&elf)),
            "image": {
                "mem_kib": container.mem_kib,
                "code": container.code.len(),
                "rodata": container.rodata.len(),
                "init": container.init.len(),
                "total": container_bytes.len(),
                "id": hex::encode(container.image_id()),
            },
        });
        write_file(
            &out_dir.join("manifest.json"),
            &serde_json::to_string_pretty(&manifest).unwrap_or_default(),
        )?;

        Ok(KernelBuildResult {
            container_bytes,
            container,
            elf_sha256: hex::encode(sha256_bytes(&elf)),
            elf,
            mem_kib: container_mem,
            cache_key: key,
            cache_hit: false,
            build_dir,
        })
    }

    /// Wrap pack/verify failures with actionable hints (see `recipe.rs` for
    /// the known hazard classes).
    fn translate_pack_error(&self, msg: String) -> String {
        let profile = self.meta.profile;
        let hint = if msg.contains("reserved imm 0") {
            format!(
                "the linked image contains a 2-byte all-zero `unimp` filler, which profile {} \
                 rejects at decode; switch to `// calc: profile = rv64im`, build with --strict, \
                 or avoid unreachable code paths",
                profile.as_str()
            )
        } else if msg.contains("unknown opcode") || msg.contains("length not in profile") {
            format!(
                "the linked image contains an instruction outside profile {} (often A-extension \
                 atomics from the prebuilt core); use --strict, `// calc: profile = rv64im`, or \
                 avoid `core::sync::atomic`",
                profile.as_str()
            )
        } else if msg.contains("image cannot fit") || msg.contains("mem_kib") {
            format!(
                "the image does not fit the admitted memory (cap {} KiB); shrink statics or \
                 adjust `// calc: mem_kib`",
                self.cap.calc_mem_kib_max
            )
        } else if msg.contains("size overflow") {
            format!(
                "the encoded container exceeds calc_image_max ({} B); shrink the kernel or split \
                 it across sibling calc functions",
                self.cap.calc_image_max
            )
        } else {
            String::new()
        };
        let hint = if hint.is_empty() {
            String::new()
        } else {
            format!("\nhint: {hint}")
        };
        format!("kernel '{}': {msg}{hint}", self.name)
    }
}

fn manifest_meta(meta: &KernelMeta) -> serde_json::Value {
    json!({
        "profile": meta.profile.as_str(),
        "input_max": meta.input_max,
        "output_max": meta.output_max,
        "mem_kib": match meta.mem_kib {
            MemSpec::Auto => "auto".to_string(),
            MemSpec::KiB(k) => k.to_string(),
        },
        "no_host_ecall": meta.no_host_ecall,
    })
}

fn write_file<C: AsRef<[u8]>>(path: &Path, content: C) -> Result<(), String> {
    let mut f =
        std::fs::File::create(path).map_err(|e| format!("create {}: {e}", path.display()))?;
    f.write_all(content.as_ref())
        .map_err(|e| format!("write {}: {e}", path.display()))
}

#[cfg(test)]
mod build_tests {
    use super::*;
    use crate::meta::ProfileName;

    fn key_of(
        name: &str,
        meta: &KernelMeta,
        strict: bool,
        has_input: bool,
        source: &[u8],
    ) -> String {
        let cap = SpaceCap::new(0);
        let dir = std::env::temp_dir().join("fitshc-cli-keytest");
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join(format!("{name}.rs"));
        std::fs::write(&src, source).unwrap();
        let req = KernelBuildRequest {
            name,
            source_path: &src,
            meta,
            has_input,
            cap: &cap,
            strict,
            cache_root: None,
        };
        let recipe = meta.profile.recipe(strict);
        // Key uses the placeholder `-T` form, mirroring the build path.
        let mut flags = base_rustflags(Path::new("container.ld"), &src);
        if let Some(tf) = recipe.target_features {
            flags.push(format!("-C target-feature={tf}"));
        }
        req.cache_key(
            &recipe,
            &flags.join(" "),
            &("rustc 1.92.0".into(), "cargo 1.92.0".into()),
        )
    }

    #[test]
    fn cache_key_moves_with_every_input() {
        let meta = KernelMeta::default();
        let src: &[u8] = b"pub fn kernel(_: &[u8], _: &mut [u8]) -> usize { 0 }";
        let base = key_of("kernel", &meta, false, true, src);

        assert_ne!(
            base,
            key_of(
                "kernel",
                &KernelMeta {
                    input_max: 64,
                    ..KernelMeta::default()
                },
                false,
                true,
                src
            ),
            "metadata change must change the key"
        );
        assert_ne!(
            base,
            key_of("kernel", &meta, true, true, src),
            "strict must change the key"
        );
        assert_ne!(
            base,
            key_of("other", &meta, false, true, src),
            "kernel name must change the key"
        );
        assert_ne!(
            base,
            key_of("kernel", &meta, false, false, src),
            "has_input must change the key"
        );
        assert_ne!(
            base,
            key_of(
                "kernel",
                &meta,
                false,
                true,
                b"pub fn kernel(_: &[u8], _: &mut [u8]) -> usize { 1 }"
            ),
            "source change must change the key"
        );
        assert_ne!(
            base,
            key_of(
                "kernel",
                &KernelMeta {
                    profile: ProfileName::Rv64im,
                    ..KernelMeta::default()
                },
                false,
                true,
                src
            ),
            "profile change must change the key"
        );
    }
}
