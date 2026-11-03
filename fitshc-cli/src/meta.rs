//! `// calc:` header metadata for auto-built rv64im kernels.
//!
//! The user's `<name>.rs` is a plain Rust *module* (the build tool wraps it
//! in a generated crate), so crate-level attributes and proc-macros are not
//! available for metadata; a sidecar TOML file would drift from the source.
//! The header comment block is inert to rustc, sits next to the code it
//! describes, and has a strict five-key schema: unknown keys, duplicate keys
//! and malformed values are errors carrying the file name and line number.
//!
//! Metadata only steers the **build** (target recipe, IO regions, memory
//! admission). The runtime source of truth is always the container header.

use std::path::Path;

use vm::calc::SpaceCap;
use vm::calc::container::{
    CALC_MEM_KIB_MIN, CALC_PROFILE_BITMANIP, CALC_PROFILE_COMPRESSED, CALC_PROFILE_NO_HOST_ECALL,
};

/// Bump when the meaning of a key or its default changes; part of the build
/// cache key so a schema change never reuses stale containers.
pub const META_SCHEMA_VERSION: u32 = 1;

/// Kernel ISA profile names accepted by `// calc: profile = ...`. The fitsh
/// keyword stays `rv64im` (the engine family); the profile only decides how
/// the kernel is compiled and which container-profile bit is set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProfileName {
    Rv64im,
    Rv64imb,
    Rv64imc,
    Rv64imcb,
}

impl ProfileName {
    pub const ALL: [ProfileName; 4] = [
        ProfileName::Rv64im,
        ProfileName::Rv64imb,
        ProfileName::Rv64imc,
        ProfileName::Rv64imcb,
    ];

    pub fn as_str(&self) -> &'static str {
        match self {
            ProfileName::Rv64im => "rv64im",
            ProfileName::Rv64imb => "rv64imb",
            ProfileName::Rv64imc => "rv64imc",
            ProfileName::Rv64imcb => "rv64imcb",
        }
    }

    pub fn parse(word: &str) -> Option<ProfileName> {
        ProfileName::ALL.into_iter().find(|p| p.as_str() == word)
    }

    /// Container profile bits without the no-host-ecall bit.
    pub fn profile_bits(&self) -> u8 {
        match self {
            ProfileName::Rv64im => 0,
            ProfileName::Rv64imb => CALC_PROFILE_BITMANIP,
            ProfileName::Rv64imc => CALC_PROFILE_COMPRESSED,
            ProfileName::Rv64imcb => CALC_PROFILE_BITMANIP | CALC_PROFILE_COMPRESSED,
        }
    }

    pub fn has_bitmanip(&self) -> bool {
        self.profile_bits() & CALC_PROFILE_BITMANIP != 0
    }

    pub fn has_compressed(&self) -> bool {
        self.profile_bits() & CALC_PROFILE_COMPRESSED != 0
    }
}

/// `mem_kib` admission: the smallest memory that fits the image plus the
/// guard and the 2 KiB minimum stack (`pack_elf_min_mem`), or an explicit
/// value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemSpec {
    Auto,
    KiB(u32),
}

/// Parsed `// calc:` header of a kernel source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KernelMeta {
    pub profile: ProfileName,
    pub input_max: u16,
    pub output_max: u16,
    pub mem_kib: MemSpec,
    pub no_host_ecall: bool,
}

impl Default for KernelMeta {
    fn default() -> Self {
        KernelMeta {
            profile: ProfileName::Rv64imc,
            input_max: 1024,
            output_max: 1024,
            mem_kib: MemSpec::Auto,
            no_host_ecall: true,
        }
    }
}

impl KernelMeta {
    /// Final container profile byte (metadata bits plus the HALT-only
    /// declaration bit).
    pub fn profile_byte(&self) -> u8 {
        let mut b = self.profile.profile_bits();
        if self.no_host_ecall {
            b |= CALC_PROFILE_NO_HOST_ECALL;
        }
        b
    }

    /// Chain admission checks against the target `SpaceCap`: the IO regions
    /// the container declares must fit what the chain accepts, and an
    /// explicit `mem_kib` must be inside the admission range.
    pub fn validate(&self, cap: &SpaceCap) -> Result<(), String> {
        if self.input_max as usize > cap.call_data_size {
            return Err(format!(
                "input_max {} exceeds the chain call_data_size {}",
                self.input_max, cap.call_data_size
            ));
        }
        if self.output_max as usize > cap.value_size {
            return Err(format!(
                "output_max {} exceeds value_size {}",
                self.output_max, cap.value_size
            ));
        }
        if self.output_max == 0 {
            return Err("output_max must be at least 1".to_string());
        }
        if let MemSpec::KiB(kib) = self.mem_kib {
            if kib < CALC_MEM_KIB_MIN || kib > cap.calc_mem_kib_max {
                return Err(format!(
                    "mem_kib {} outside admitted range [{}, {}]",
                    kib, CALC_MEM_KIB_MIN, cap.calc_mem_kib_max
                ));
            }
        }
        Ok(())
    }

    /// Canonical text form used in the cache key (stable across map
    /// iteration order and comment formatting).
    pub fn cache_key_text(&self) -> String {
        let mem = match self.mem_kib {
            MemSpec::Auto => "auto".to_string(),
            MemSpec::KiB(k) => k.to_string(),
        };
        format!(
            "profile={} input_max={} output_max={} mem_kib={} no_host_ecall={}",
            self.profile.as_str(),
            self.input_max,
            self.output_max,
            mem,
            self.no_host_ecall
        )
    }
}

/// Parse the `// calc:` header of a kernel source. Only the leading comment
/// block is scanned (blank lines may interleave; the first line that is
/// neither blank nor a `//` comment ends the header). Returns the metadata
/// with defaults applied for absent keys.
pub fn parse_kernel_meta(file: &Path, source: &str) -> Result<KernelMeta, String> {
    let mut meta = KernelMeta::default();
    let mut seen: Vec<String> = Vec::new();
    for (idx, raw) in source.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        let Some(comment) = line.strip_prefix("//") else {
            break; // first code line ends the header block
        };
        let body = comment.trim();
        let Some(kv) = body.strip_prefix("calc:") else {
            continue; // ordinary comment line
        };
        let (key, value) = split_kv(kv).ok_or_else(|| {
            format!(
                "{}:{}: malformed '// calc:' line: {:?}",
                file.display(),
                idx + 1,
                raw
            )
        })?;
        let line_no = idx + 1;
        if seen.iter().any(|k| k == key) {
            return Err(format!(
                "{}:{}: duplicate '// calc:' key '{}'",
                file.display(),
                line_no,
                key
            ));
        }
        match key {
            "profile" => {
                meta.profile = ProfileName::parse(value).ok_or_else(|| {
                    format!(
                        "{}:{}: unknown profile '{}' (expected one of {})",
                        file.display(),
                        line_no,
                        value,
                        ProfileName::ALL
                            .iter()
                            .map(|p| p.as_str())
                            .collect::<Vec<_>>()
                            .join("/")
                    )
                })?;
            }
            "input_max" => {
                meta.input_max = parse_u16(file, line_no, key, value, 0)?;
            }
            "output_max" => {
                meta.output_max = parse_u16(file, line_no, key, value, 1)?;
            }
            "mem_kib" => {
                meta.mem_kib = if value == "auto" {
                    MemSpec::Auto
                } else {
                    let n: u32 = value.parse().map_err(|_| {
                        format!(
                            "{}:{}: mem_kib must be 'auto' or an integer, got {:?}",
                            file.display(),
                            line_no,
                            value
                        )
                    })?;
                    MemSpec::KiB(n)
                };
            }
            "no_host_ecall" => {
                meta.no_host_ecall = match value {
                    "true" => true,
                    "false" => false,
                    _ => {
                        return Err(format!(
                            "{}:{}: no_host_ecall must be true/false, got {:?}",
                            file.display(),
                            line_no,
                            value
                        ));
                    }
                };
            }
            _ => {
                return Err(format!(
                    "{}:{}: unknown '// calc:' key '{}' (schema: profile, input_max, output_max, mem_kib, no_host_ecall)",
                    file.display(),
                    line_no,
                    key
                ));
            }
        }
        seen.push(key.to_string());
    }
    Ok(meta)
}

fn split_kv(kv: &str) -> Option<(&str, &str)> {
    let (k, v) = kv.split_once('=')?;
    let k = k.trim();
    let v = v.trim();
    if k.is_empty() || v.is_empty() {
        return None;
    }
    Some((k, v))
}

fn parse_u16(file: &Path, line_no: usize, key: &str, value: &str, min: u16) -> Result<u16, String> {
    value
        .parse::<u16>()
        .map_err(|_| {
            format!(
                "{}:{}: {} must be an integer in [{}, 65535], got {:?}",
                file.display(),
                line_no,
                key,
                min,
                value
            )
        })
        .and_then(|n| {
            if n < min {
                Err(format!(
                    "{}:{}: {} must be at least {}, got {}",
                    file.display(),
                    line_no,
                    key,
                    min,
                    n
                ))
            } else {
                Ok(n)
            }
        })
}

#[cfg(test)]
mod meta_tests {
    use super::*;

    fn parse(src: &str) -> Result<KernelMeta, String> {
        parse_kernel_meta(Path::new("kernel.rs"), src)
    }

    #[test]
    fn defaults_apply_when_no_header() {
        let m = parse("pub fn kernel(input: &[u8], out: &mut [u8]) -> usize { 0 }").unwrap();
        assert_eq!(m, KernelMeta::default());
        assert_eq!(m.profile, ProfileName::Rv64imc);
        assert_eq!(m.profile_byte(), 0b110); // RVC + no-host-ecall
    }

    #[test]
    fn parses_all_keys_and_blank_lines() {
        let m = parse(
            "// calc: profile = rv64imb\n\n// some prose\n\n// calc: input_max = 64\n//   calc: output_max = 32\n//calc:mem_kib=16\n// calc: no_host_ecall = false\n",
        )
        .unwrap();
        assert_eq!(m.profile, ProfileName::Rv64imb);
        assert_eq!(m.input_max, 64);
        assert_eq!(m.output_max, 32);
        assert_eq!(m.mem_kib, MemSpec::KiB(16));
        assert!(!m.no_host_ecall);
        assert_eq!(m.profile_byte(), 0b001);
    }

    #[test]
    fn header_ends_at_first_code_line() {
        // A `// calc:` line after code must be ignored, not parsed.
        let m = parse(
            "pub fn kernel(_: &[u8], _: &mut [u8]) -> usize { 0 }\n// calc: profile = rv64im\n",
        )
        .unwrap();
        assert_eq!(m.profile, ProfileName::Rv64imc);
    }

    #[test]
    fn rejects_unknown_duplicate_and_malformed() {
        assert!(
            parse("// calc: turbo = 1\n")
                .unwrap_err()
                .contains("unknown")
        );
        assert!(
            parse("// calc: profile = rv64im\n// calc: profile = rv64imc\n")
                .unwrap_err()
                .contains("duplicate")
        );
        assert!(
            parse("// calc: profile = avx512\n")
                .unwrap_err()
                .contains("unknown profile")
        );
        assert!(
            parse("// calc: input_max = big\n")
                .unwrap_err()
                .contains("input_max")
        );
        assert!(
            parse("// calc: output_max = 0\n")
                .unwrap_err()
                .contains("output_max")
        );
        assert!(
            parse("// calc: mem_kib = soon\n")
                .unwrap_err()
                .contains("mem_kib")
        );
        assert!(
            parse("// calc: no_host_ecall = maybe\n")
                .unwrap_err()
                .contains("no_host_ecall")
        );
        assert!(
            parse("// calc: broken\n")
                .unwrap_err()
                .contains("malformed")
        );
    }

    #[test]
    fn errors_carry_file_and_line() {
        let err = parse_kernel_meta(
            Path::new("/x/hash.rs"),
            "// calc: profile = rv64imc\n// calc: turbo = 1\n",
        )
        .unwrap_err();
        assert!(err.contains("/x/hash.rs:2"), "{err}");
    }

    #[test]
    fn validate_against_cap() {
        let mut m = KernelMeta::default();
        assert!(m.validate(&SpaceCap::new(0)).is_ok());
        m.input_max = 5121;
        assert!(
            m.validate(&SpaceCap::new(0))
                .unwrap_err()
                .contains("input_max")
        );
        m = KernelMeta {
            input_max: 1024,
            output_max: 1281,
            ..KernelMeta::default()
        };
        assert!(
            m.validate(&SpaceCap::new(0))
                .unwrap_err()
                .contains("output_max")
        );
        m = KernelMeta {
            mem_kib: MemSpec::KiB(65),
            ..KernelMeta::default()
        };
        assert!(
            m.validate(&SpaceCap::new(0))
                .unwrap_err()
                .contains("mem_kib")
        );
    }
}
