//! Kernel build engine: generated wrapper crate → cargo/rustc → ELF →
//! packed, verified container, with a content-addressed cache.
//!
//! Layout of a build directory (the cache entry lives at
//! `<cache_root>/k-<key>/`, keyed by a hash over every build input, so a
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
//!   out/cache.key         the key this entry was built under
//!   out/deps.json         transitive source dependencies + hashes
//!   out/manifest.json     toolchain + image record
//! ```
//!
//! Cache semantics: the *key* covers the primary source, metadata, recipes,
//! wrapper template, flags, toolchain versions and the `SpaceCap` snapshot.
//! Transitive dependencies (`mod helpers;`, `include!`, …) cannot feed the
//! key without building first, so they are validated the other way round:
//! every build records `out/deps.json` (parsed from rustc's dep-info) and a
//! hit re-hashes each listed file — any change is a miss. A hit also
//! re-decodes and re-verifies the stored container, so a corrupted or
//! poisoned entry fails loudly instead of injecting garbage.
//!
//! Builds run in a private `<cache_root>/.tmp-k-<key>-<pid>-<nanos>/`
//! directory and are published with one atomic `rename`, so concurrent
//! builds of the same key never observe interleaved files.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::json;

use vm::calc::SpaceCap;
use vm::calc::container::CalcContainer;
use vm::calc::verify::verify_container_with_cap;
use vm::calc::{pack_elf, pack_elf_min_mem, sha256_bytes};

use crate::meta::{KernelMeta, MemSpec};
use crate::recipe::{
    RECIPE_VERSION, Recipe, WRAPPER_CARGO_TOML, base_rustflags, cargo_extra_args, wrapper_source,
};

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
    /// Skip the cache entirely: build in a throwaway temp dir, never read or
    /// write a shared entry (release pipelines, untrusted cache roots).
    pub no_cache: bool,
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
    for home in [std::env::var_os("HOME"), std::env::var_os("USERPROFILE")] {
        if let Some(home) = home {
            if !home.is_empty() {
                return PathBuf::from(home).join(".cache").join("fitshc");
            }
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

fn env_raw(bin_env: &str) -> String {
    std::env::var(bin_env)
        .unwrap_or_default()
        .trim()
        .to_string()
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

fn unique_suffix() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{}-{}", std::process::id(), nanos)
}

// ---------------------------------------------------------------------------
// Transitive dependency tracking (cache-hit validation)
// ---------------------------------------------------------------------------

/// One recorded dependency: absolute path + SHA-256 at build time.
fn hash_of(path: &Path) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    Some(hex::encode(sha256_bytes(&bytes)))
}

/// Parse a makefile-style rustc dep-info file into absolute dependency
/// paths (unquoting escaped paths, resolving relatives against `base`).
fn parse_dep_info(text: &str, base: &Path) -> Vec<PathBuf> {
    // Join line continuations first.
    let joined = text.replace("\\\r\n", " ").replace("\\\n", " ");
    // Drop the target rule ("out: deps..."); find the first ':' outside quotes.
    let bytes = joined.as_bytes();
    let mut in_quotes = false;
    let mut colon = None;
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'"' => in_quotes = !in_quotes,
            b':' if !in_quotes => {
                colon = Some(i);
                break;
            }
            _ => {}
        }
    }
    let deps_part = match colon {
        Some(i) => &joined[i + 1..],
        None => joined.as_str(),
    };
    let mut out = Vec::new();
    let mut token = String::new();
    let mut chars = deps_part.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                // Quoted path: take until the closing quote, honoring escapes.
                let mut q = String::new();
                while let Some(qc) = chars.next() {
                    if qc == '\\' {
                        if let Some(esc) = chars.next() {
                            // Makefile escapes: \  \" \\ \# etc.
                            q.push(esc);
                        }
                    } else if qc == '"' {
                        break;
                    } else {
                        q.push(qc);
                    }
                }
                token.push_str(&q);
            }
            '\\' => {
                if let Some(esc) = chars.next() {
                    if esc == ' ' || esc == '"' || esc == '\\' || esc == '#' {
                        token.push(esc);
                    } else {
                        token.push('\\');
                        token.push(esc);
                    }
                }
            }
            c if c.is_whitespace() => {
                if !token.is_empty() {
                    out.push(PathBuf::from(std::mem::take(&mut token)));
                }
            }
            c => token.push(c),
        }
    }
    if !token.is_empty() {
        out.push(PathBuf::from(token));
    }
    out.into_iter()
        .map(|p| if p.is_absolute() { p } else { base.join(p) })
        .collect()
}

/// Collect the source dependencies of a finished build. Primary source is
/// rustc's dep-info (`target/<triple>/release/calc_kernel.d`); files inside
/// the build dir (wrapper, target artifacts) are excluded — they are covered
/// by the key. Fallback when dep-info is unreadable: every `*.rs` under the
/// kernel's directory (over-invalidation, but never stale).
fn collect_deps(
    dep_info: Option<&Path>,
    build_dir: &Path,
    source_path: &Path,
) -> Vec<(PathBuf, String)> {
    let candidates: Vec<PathBuf> = match dep_info
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|text| parse_dep_info(&text, build_dir))
    {
        Some(list) if !list.is_empty() => list,
        _ => scan_kernel_dir(source_path.parent().unwrap_or(Path::new("."))),
    };
    let mut seen = std::collections::BTreeSet::new();
    let mut out = Vec::new();
    for p in candidates {
        let p = p.canonicalize().unwrap_or(p);
        if p.starts_with(build_dir) || !p.is_file() {
            continue;
        }
        if let Some(sha) = hash_of(&p) {
            if seen.insert(p.clone()) {
                out.push((p, sha));
            }
        }
    }
    out
}

/// Recursively list `*.rs` files under `dir`, skipping hidden and build dirs.
fn scan_kernel_dir(dir: &Path) -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => return,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                if name.starts_with('.') || name == "target" || name == "node_modules" {
                    continue;
                }
                walk(&path, out);
            } else if name.ends_with(".rs") {
                out.push(path);
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, &mut out);
    out
}

fn write_deps_json(path: &Path, deps: &[(PathBuf, String)]) -> Result<(), String> {
    let doc = json!({ "deps": deps
        .iter()
        .map(|(p, sha)| json!({"path": p.display().to_string(), "sha256": sha}))
        .collect::<Vec<_>>() });
    write_file(path, serde_json::to_string_pretty(&doc).unwrap_or_default())
}

/// True when every recorded dependency still hashes to its recorded value.
fn deps_unchanged(path: &Path) -> bool {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(_) => return false,
    };
    let doc: serde_json::Value = match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(_) => return false,
    };
    let Some(list) = doc.get("deps").and_then(|v| v.as_array()) else {
        return false;
    };
    if list.is_empty() {
        return false; // never recorded → cannot validate → rebuild
    }
    list.iter().all(|e| {
        e.get("path")
            .and_then(|p| p.as_str())
            .and_then(|p| hash_of(Path::new(p)))
            .is_some_and(|sha| Some(sha.as_str()) == e.get("sha256").and_then(|s| s.as_str()))
    })
}

// ---------------------------------------------------------------------------
// Build
// ---------------------------------------------------------------------------

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

    /// Placeholder module path so the wrapper template hash is stable no
    /// matter where the kernel lives (the `#[path]` string itself never
    /// reaches the image).
    fn wrapper_template(&self) -> String {
        wrapper_source("<kernel>", self.name, self.has_input)
    }

    fn cache_key(&self, recipe: &Recipe, flags: &str, versions: &(String, String)) -> String {
        let source = std::fs::read(self.source_path).unwrap_or_default();
        let mut key = Vec::new();
        key.extend_from_slice(b"fitshc-kernel-v2\0");
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
        // Wrapper template: hashed directly so a template edit can never
        // reuse a stale entry even if the manual version bump is forgotten.
        key.extend_from_slice(&sha256_bytes(self.wrapper_template().as_bytes()));
        key.push(0);
        key.extend_from_slice(recipe.id.as_bytes());
        key.push(0);
        key.extend_from_slice(flags.as_bytes());
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
        // Tool selection envs: `cargo +nightly` / a wrapped rustc select a
        // different toolchain than the probed defaults must suggest.
        key.push(0);
        key.extend_from_slice(
            format!(
                "cargo_env={} rustc_env={}\0",
                env_raw("FITSHC_CARGO"),
                env_raw("FITSHC_RUSTC")
            )
            .as_bytes(),
        );
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

    /// Validate a finished cache entry against the current request: key
    /// match, all transitive deps unchanged, container decodes and verifies
    /// under the live cap.
    fn try_cache_hit(&self, entry: &Path, key: &str) -> Option<KernelBuildResult> {
        let out_dir = entry.join("out");
        let stored = std::fs::read_to_string(out_dir.join("cache.key")).ok()?;
        if stored != key {
            return None;
        }
        if !deps_unchanged(&out_dir.join("deps.json")) {
            return None;
        }
        let bytes = std::fs::read(out_dir.join("kernel.hcf1")).ok()?;
        let (c, used) = CalcContainer::decode(&bytes).ok()?;
        if used != bytes.len() || verify_container_with_cap(&c, self.cap).is_err() {
            return None;
        }
        let elf = std::fs::read(out_dir.join("kernel.elf")).unwrap_or_default();
        let mem_kib = c.mem_kib;
        Some(KernelBuildResult {
            elf_sha256: hex::encode(sha256_bytes(&elf)),
            container_bytes: bytes,
            container: c,
            elf,
            mem_kib,
            cache_key: key.to_string(),
            cache_hit: true,
            build_dir: entry.to_path_buf(),
        })
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

        // Flags are passed to cargo through CARGO_ENCODED_RUSTFLAGS (\x1f
        // separated) so paths with spaces survive and ambient RUSTFLAGS
        // cannot leak in (both are scrubbed from the child env).
        let ld_path = PathBuf::from("container.ld");
        let mut flags = base_rustflags(&ld_path, source_path);
        if let Some(tf) = recipe.target_features {
            flags.push(format!("-Ctarget-feature={tf}"));
        }
        let flags = flags.join("\x1f");
        let key = self.cache_key(&recipe, &flags, &versions);

        // Resolve the build location: a persistent cache entry, or a
        // throwaway temp dir for --no-cache / unwritable cache roots.
        let cache_root = self.cache_root.clone().unwrap_or_else(default_cache_root);
        let persist = !self.no_cache && std::fs::create_dir_all(&cache_root).is_ok();
        let (build_dir, publish_to) = if persist {
            let entry = cache_root.join(format!("k-{key}"));
            if let Some(hit) = self.try_cache_hit(&entry, &key) {
                return Ok(hit);
            }
            let tmp = cache_root.join(format!(".tmp-k-{key}-{}", unique_suffix()));
            (tmp, Some(entry))
        } else {
            let tmp =
                std::env::temp_dir().join(format!("fitshc-build-k-{key}-{}", unique_suffix()));
            (tmp, None)
        };
        // The `-T` script path must be absolute for the child cargo run; it
        // never reaches the image bytes (kept out of the key on purpose —
        // moving the cache root only relocates entries).
        let ld_path = build_dir.join("container.ld");
        let rustflags = flags.replace(
            "-Clink-arg=-Tcontainer.ld",
            &format!("-Clink-arg=-T{}", ld_path.display()),
        );

        // Materialize the wrapper crate. The user's file is referenced by
        // absolute `#[path]`, never copied, so `mod helpers;` and rustc
        // diagnostics keep pointing at the original file.
        let src_dir = build_dir.join("src");
        std::fs::create_dir_all(&src_dir)
            .map_err(|e| format!("create {}: {e} (cache root writable?)", src_dir.display()))?;
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
            // Hermetic flag/toolchain environment: ambient RUSTFLAGS /
            // CARGO_ENCODED_RUSTFLAGS / RUSTC would override ours and break
            // the layout (`-T` lost) or the cache-key/toolchain agreement.
            .env_remove("RUSTFLAGS")
            .env_remove("CARGO_ENCODED_RUSTFLAGS")
            .env_remove("RUSTC")
            .env("CARGO_ENCODED_RUSTFLAGS", &rustflags)
            .env("CARGO_TARGET_DIR", build_dir.join("target"))
            .current_dir(&build_dir);
        if let Ok(rv) = std::env::var("FITSHC_RUSTC") {
            if !rv.trim().is_empty() {
                cmd.env("RUSTC", rv);
            }
        }
        if recipe.build_std {
            cmd.env("RUSTC_BOOTSTRAP", "1");
        }
        let out = cmd
            .output()
            .map_err(|e| format!("failed to run cargo: {e}"))?;
        if !out.status.success() {
            let _ = std::fs::remove_dir_all(&build_dir);
            return Err(format!(
                "rustc failed building kernel '{}' from {} — it must define `{}`:\n{}",
                self.name,
                source_path.display(),
                self.expected_signature(),
                String::from_utf8_lossy(&out.stderr).trim_end()
            ));
        }

        // cargo names the output dir after the target triple / JSON stem.
        let triple_dir = target_stem(&target_arg, &recipe.target);
        let release_dir = build_dir.join("target").join(&triple_dir).join("release");
        let elf_path = release_dir.join("calc_kernel");
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
        .map_err(|e| {
            let _ = std::fs::remove_dir_all(&build_dir);
            self.translate_pack_error(e.to_string())
        })?;
        verify_container_with_cap(&container, self.cap).map_err(|e| {
            let _ = std::fs::remove_dir_all(&build_dir);
            self.translate_pack_error(e.to_string())
        })?;
        let container_bytes = container
            .encode()
            .map_err(|e| format!("encode container: {e}"))?;
        let container_mem = container.mem_kib;

        // Record the entry (in the private dir first; published atomically).
        let out_dir = build_dir.join("out");
        std::fs::create_dir_all(&out_dir)
            .map_err(|e| format!("create {}: {e}", out_dir.display()))?;
        let deps = collect_deps(
            Some(&release_dir.join("calc_kernel.d")),
            &build_dir,
            source_path,
        );
        write_file(&out_dir.join("kernel.hcf1"), &container_bytes)?;
        write_file(&out_dir.join("kernel.elf"), &elf)?;
        write_file(&out_dir.join("cache.key"), &key)?;
        write_deps_json(&out_dir.join("deps.json"), &deps)?;
        let manifest = json!({
            "kernel": self.name,
            "wrapper_abi": 1,
            "recipe": recipe.id,
            "toolchain": { "rustc": versions.0, "cargo": versions.1 },
            "meta": manifest_meta(self.meta),
            "profile_byte": self.meta.profile_byte(),
            "source_sha256": hex::encode(sha256_bytes(&std::fs::read(source_path).unwrap_or_default())),
            "elf_sha256": hex::encode(sha256_bytes(&elf)),
            "deps": deps.iter().map(|(p, s)| json!({"path": p.display().to_string(), "sha256": s})).collect::<Vec<_>>(),
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

        // Publish atomically. If another process completed the same key in
        // the meantime, prefer their validated entry; a corrupt incumbent is
        // replaced. Rename failure of any other kind keeps our private dir —
        // the build result is already complete there.
        let mut final_dir = build_dir.clone();
        if let Some(entry) = &publish_to {
            match std::fs::rename(&build_dir, entry) {
                Ok(()) => final_dir = entry.clone(),
                Err(_) => {
                    if let Some(hit) = self.try_cache_hit(entry, &key) {
                        let _ = std::fs::remove_dir_all(&build_dir);
                        return Ok(hit);
                    }
                    let _ = std::fs::remove_dir_all(entry);
                    if std::fs::rename(&build_dir, entry).is_ok() {
                        final_dir = entry.clone();
                    }
                }
            }
        }

        Ok(KernelBuildResult {
            elf_sha256: hex::encode(sha256_bytes(&elf)),
            container_bytes,
            container,
            elf,
            mem_kib: container_mem,
            cache_key: key,
            cache_hit: false,
            build_dir: final_dir,
        })
    }

    /// Wrap pack/verify failures with actionable hints (see `recipe.rs` for
    /// the known hazard classes).
    fn translate_pack_error(&self, msg: String) -> String {
        let profile = self.meta.profile;
        let hint = if msg.contains("unknown allocated section") {
            format!(
                "the linked image carries an extra allocated section (e.g. `.eh_frame` from the \
                 prebuilt core's panic paths); the generated linker script discards it — rebuild \
                 with an up-to-date fitshc, or use `// calc: profile = rv64im` / --strict"
            )
        } else if msg.contains("missing .text")
            || msg.contains("does not start at")
            || msg.contains("relocation section")
        {
            format!(
                "the ELF layout does not match the container map — the `-T container.ld` link \
                 argument was not applied (check that no ambient RUSTFLAGS/CARGO_ENCODED_RUSTFLAGS \
                 reaches the build; fitshc scrubs them) or an orphan section displaced `.text`; \
                 rebuild with an up-to-date fitshc"
            )
        } else if msg.contains("reserved imm 0") {
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

/// Directory name cargo builds into for a `--target` argument: the file
/// name of a custom JSON spec (`/abs/riscv64im_zicclsm.json` →
/// `riscv64im_zicclsm`), or the triple itself. Path-separator agnostic.
fn target_stem(target_arg: &str, fallback: &str) -> String {
    let file = Path::new(target_arg)
        .file_name()
        .map(|f| f.to_string_lossy().to_string())
        .unwrap_or_else(|| target_arg.to_string());
    let stem = file.strip_suffix(".json").unwrap_or(&file);
    if stem.is_empty() {
        fallback.trim_end_matches(".json").to_string()
    } else {
        stem.to_string()
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
            no_cache: false,
        };
        let recipe = meta.profile.recipe(strict);
        let mut flags = base_rustflags(Path::new("container.ld"), &src);
        if let Some(tf) = recipe.target_features {
            flags.push(format!("-Ctarget-feature={tf}"));
        }
        req.cache_key(
            &recipe,
            &flags.join("\x1f"),
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
            "has_input must change the key (wrapper ABI differs)"
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

    #[test]
    fn dep_info_parser_handles_quoting_and_continuations() {
        // Makefile dep-info quotes paths that contain spaces; unquoted
        // entries never contain whitespace.
        let dir = Path::new("/build");
        let text = "/build/out.elf: src/main.rs \\\n  \"/path with space/kernel.rs\" \\\n  \"/quoted path/helpers.rs\" \\\n  /build/target/x/release/libcore.rlib\n";
        let deps = parse_dep_info(text, dir);
        let names: Vec<String> = deps.iter().map(|p| p.display().to_string()).collect();
        assert!(
            names.contains(&"/build/src/main.rs".to_string()),
            "{names:?}"
        );
        assert!(
            names.contains(&"/path with space/kernel.rs".to_string()),
            "{names:?}"
        );
        assert!(
            names.contains(&"/quoted path/helpers.rs".to_string()),
            "{names:?}"
        );
        assert!(
            names.contains(&"/build/target/x/release/libcore.rlib".to_string()),
            "{names:?}"
        );
    }

    #[test]
    fn deps_json_roundtrip_detects_change() {
        let dir = std::env::temp_dir().join("fitshc-cli-depstest");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let helper = dir.join("helpers.rs");
        std::fs::write(&helper, b"pub const V: u8 = 1;").unwrap();
        let deps = vec![(helper.clone(), hash_of(&helper).unwrap())];
        let dj = dir.join("deps.json");
        write_deps_json(&dj, &deps).unwrap();
        assert!(deps_unchanged(&dj), "unchanged deps must hit");
        std::fs::write(&helper, b"pub const V: u8 = 2;").unwrap();
        assert!(!deps_unchanged(&dj), "changed dep must miss");
        std::fs::remove_file(&helper).unwrap();
        assert!(!deps_unchanged(&dj), "deleted dep must miss");
        assert!(
            !deps_unchanged(&dir.join("missing.json")),
            "no record must miss"
        );
    }

    #[test]
    fn target_stem_trims_json_and_separators() {
        assert_eq!(
            target_stem("/abs/dir/riscv64im_zicclsm.json", "x"),
            "riscv64im_zicclsm"
        );
        assert_eq!(
            target_stem("riscv64imac-unknown-none-elf", "x"),
            "riscv64imac-unknown-none-elf"
        );
        assert_eq!(target_stem("", "fallback.json"), "fallback");
    }

    #[test]
    fn scan_kernel_dir_finds_nested_rs_only() {
        let dir = std::env::temp_dir().join("fitshc-cli-scantest");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::create_dir_all(dir.join(".git")).unwrap();
        std::fs::create_dir_all(dir.join("target")).unwrap();
        std::fs::write(dir.join("hash.rs"), b"").unwrap();
        std::fs::write(dir.join("sub/helpers.rs"), b"").unwrap();
        std::fs::write(dir.join(".git/x.rs"), b"").unwrap();
        std::fs::write(dir.join("target/y.rs"), b"").unwrap();
        std::fs::write(dir.join("notes.txt"), b"").unwrap();
        let found = scan_kernel_dir(&dir);
        let names: Vec<String> = found
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        assert!(names.contains(&"hash.rs".to_string()));
        assert!(names.contains(&"helpers.rs".to_string()));
        assert!(!names.iter().any(|n| n == "x.rs" || n == "y.rs"));
    }
}
