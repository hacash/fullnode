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
    CALC_MEM_KIB_MIN, CALC_PROFILE_BITMANIP, CALC_PROFILE_COMPRESSED, CALC_PROFILE_HASH,
};

/// Bump when the meaning of a key or its default changes; part of the build
/// cache key so a schema change never reuses stale containers.
pub const META_SCHEMA_VERSION: u32 = 3;

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

    /// Container profile bits without the HASH opt-in.
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
/// 2 KiB minimum stack (`pack_elf_min_mem`), or an explicit value.
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
    pub hash: bool,
}

impl Default for KernelMeta {
    fn default() -> Self {
        KernelMeta {
            profile: ProfileName::Rv64im,
            input_max: 1024,
            output_max: 1024,
            mem_kib: MemSpec::Auto,
            hash: false,
        }
    }
}

impl KernelMeta {
    /// Final container profile byte (ISA bits plus the optional HASH bit).
    pub fn profile_byte(&self) -> u8 {
        let mut b = self.profile.profile_bits();
        if self.hash {
            b |= CALC_PROFILE_HASH;
        }
        b
    }

    /// Chain admission checks against the target `SpaceCap`: the IO regions
    /// the container declares must fit what the chain accepts, and an
    /// explicit `mem_kib` must be inside the admission range. Mirrors the
    /// consensus rule: every B / RVC profile is admitted, the HASH bit is
    /// not.
    pub fn validate(&self, cap: &SpaceCap) -> Result<(), String> {
        if self.hash {
            return Err("hash ecalls are not admitted in v1".to_string());
        }
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
            "profile={} input_max={} output_max={} mem_kib={} hash={}",
            self.profile.as_str(),
            self.input_max,
            self.output_max,
            mem,
            self.hash
        )
    }
}

/// Parse the `// calc:` header of a kernel source. Only the leading comment
/// block is scanned: blank lines may interleave, `/* ... */` block comments
/// (license headers) are skipped whole, and the first line that is neither
/// blank nor a comment ends the header. Returns the metadata with defaults
/// applied for absent keys.
pub fn parse_kernel_meta(file: &Path, source: &str) -> Result<KernelMeta, String> {
    let mut meta = KernelMeta::default();
    let mut seen: Vec<String> = Vec::new();
    let mut in_block_comment = false;
    for (idx, raw) in source.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        // Inside a block comment: stay until the closing `*/`.
        if in_block_comment {
            if line.contains("*/") {
                in_block_comment = false;
            }
            continue;
        }
        if line.starts_with("/*") {
            if !line.contains("*/") {
                in_block_comment = true;
            }
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
            "hash" => {
                meta.hash = match value {
                    "true" => true,
                    "false" => false,
                    _ => {
                        return Err(format!(
                            "{}:{}: hash must be true/false, got {:?}",
                            file.display(),
                            line_no,
                            value
                        ));
                    }
                };
            }
            _ => {
                return Err(format!(
                    "{}:{}: unknown '// calc:' key '{}' (schema: profile, input_max, output_max, mem_kib, hash)",
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

/// Infer whether a kernel source defines the with-input or the zero-arg
/// form of its entry function (`fitshc run` has no fitsh declaration to
/// ask). This is a light textual scan for the *two accepted shapes* —
/// `fn name(input: &[u8], out: &mut [u8])` and `fn name(out: &mut [u8])` —
/// not a Rust parser; anything else is an error telling the user the two
/// supported signatures (and rustc remains the authority on the build).
pub fn infer_kernel_has_input(source: &str, name: &str) -> Result<bool, String> {
    let Some(params) = find_fn_params(source, name) else {
        return Err(format!(
            "no `fn {name}` found in the kernel source; define \
             `pub fn {name}(input: &[u8], out: &mut [u8]) -> usize` or \
             `pub fn {name}(out: &mut [u8]) -> usize`"
        ));
    };
    let parts = split_top_level_commas(&params);
    let norm = |p: &str| p.replace(char::is_whitespace, "");
    match parts.len() {
        2 => {
            let (a, b) = (norm(&parts[0]), norm(&parts[1]));
            if a.contains("&[u8]") && !a.contains("mut") && b.contains("&mut[u8]") {
                Ok(true)
            } else {
                Err(two_shapes_error(name))
            }
        }
        1 => {
            if norm(&parts[0]).contains("&mut[u8]") {
                Ok(false)
            } else {
                Err(two_shapes_error(name))
            }
        }
        _ => Err(two_shapes_error(name)),
    }
}

fn two_shapes_error(name: &str) -> String {
    format!(
        "kernel function `{name}` does not match a supported signature; expected \
         `pub fn {name}(input: &[u8], out: &mut [u8]) -> usize` or \
         `pub fn {name}(out: &mut [u8]) -> usize`"
    )
}

fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// Text of the parameter list of the first `fn <name>(...)` in `source`.
///
/// This is deliberately a small scanner, not a Rust parser: it only has to
/// avoid being fooled by `fn <name>` appearing inside a comment or literal
/// (doc comments are the common case) and must never panic on truncated
/// input. rustc remains the authority on whether the source compiles.
fn find_fn_params(source: &str, name: &str) -> Option<String> {
    let b = source.as_bytes();
    let mut i = 0usize;
    while i < b.len() {
        match b[i] {
            b'/' if b.get(i + 1) == Some(&b'/') => {
                i += 2;
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
                continue;
            }
            b'/' if b.get(i + 1) == Some(&b'*') => {
                i = skip_block_comment(b, i + 2);
                continue;
            }
            b'"' => {
                i = skip_quoted(b, i + 1, b'"');
                continue;
            }
            b'\'' => {
                if let Some(end) = char_literal_end(b, i) {
                    i = end;
                    continue;
                }
                i += 1;
                continue;
            }
            b'r' if b.get(i + 1) == Some(&b'"') || b.get(i + 1) == Some(&b'#') => {
                if let Some(end) = raw_string_end(b, i) {
                    i = end;
                    continue;
                }
            }
            _ => {}
        }
        if b[i] == b'f' && b.get(i + 1) == Some(&b'n') {
            let after = i + 2;
            let keyword_ok = i == 0 || !is_ident_byte(b[i - 1]);
            let name_ok = b.get(after).map_or(true, |&c| !is_ident_byte(c));
            if keyword_ok && name_ok {
                let mut j = after;
                while j < b.len() && b[j].is_ascii_whitespace() {
                    j += 1;
                }
                if b.get(j..)
                    .is_some_and(|rest| rest.starts_with(name.as_bytes()))
                {
                    let end = j + name.len();
                    if b.get(end).map_or(true, |&c| !is_ident_byte(c)) {
                        let mut k = end;
                        while k < b.len() && b[k].is_ascii_whitespace() {
                            k += 1;
                        }
                        if b.get(k) == Some(&b'(') {
                            return capture_params(source, k);
                        }
                    }
                }
            }
        }
        i += 1;
    }
    None
}

fn capture_params(source: &str, open: usize) -> Option<String> {
    let b = source.as_bytes();
    let start = open + 1;
    let mut k = start;
    let mut depth = 1usize;
    while k < b.len() && depth > 0 {
        match b[k] {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth -= 1,
            _ => {}
        }
        k += 1;
    }
    (depth == 0).then(|| source[start..k - 1].to_string())
}

fn skip_block_comment(b: &[u8], mut i: usize) -> usize {
    let mut depth = 1usize;
    while i < b.len() && depth > 0 {
        if b[i] == b'/' && b.get(i + 1) == Some(&b'*') {
            depth += 1;
            i += 2;
        } else if b[i] == b'*' && b.get(i + 1) == Some(&b'/') {
            depth -= 1;
            i += 2;
        } else {
            i += 1;
        }
    }
    i
}

fn skip_quoted(b: &[u8], mut i: usize, quote: u8) -> usize {
    while i < b.len() {
        if b[i] == b'\\' {
            i = (i + 2).min(b.len());
        } else if b[i] == quote {
            return i + 1;
        } else {
            i += 1;
        }
    }
    b.len()
}

/// End of a character literal at `start`, or `None` when the quote starts a
/// lifetime (no closing quote within a scalar's width).
fn char_literal_end(b: &[u8], start: usize) -> Option<usize> {
    let mut j = start + 1;
    if b.get(j) == Some(&b'\\') {
        j += 2;
        while j < b.len() && b[j] != b'\'' && b[j] != b'\n' {
            j += 1;
        }
    } else {
        // One scalar value is 1..=4 bytes; an escape is wider.
        let limit = (start + 12).min(b.len());
        j += 1;
        while j < limit && b[j] != b'\'' && b[j] != b'\n' {
            j += 1;
        }
    }
    (b.get(j) == Some(&b'\'')).then_some(j + 1)
}

/// End of a raw string starting at the `r` in `r"..."` / `r#"..."#`.
fn raw_string_end(b: &[u8], start: usize) -> Option<usize> {
    let mut j = start + 1;
    let mut hashes = 0usize;
    while b.get(j) == Some(&b'#') {
        hashes += 1;
        j += 1;
    }
    if b.get(j) != Some(&b'"') {
        return None; // `r#ident` raw identifier, not a string
    }
    j += 1;
    while j < b.len() {
        if b[j] == b'"' {
            let mut k = j + 1;
            let mut n = 0usize;
            while n < hashes && b.get(k) == Some(&b'#') {
                n += 1;
                k += 1;
            }
            if n == hashes {
                return Some(k);
            }
        }
        j += 1;
    }
    Some(b.len())
}

/// Split a parameter list on commas that are not nested in brackets.
fn split_top_level_commas(params: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut cur = String::new();
    for c in params.chars() {
        match c {
            '(' | '[' | '{' | '<' => {
                depth += 1;
                cur.push(c);
            }
            ')' | ']' | '}' | '>' => {
                depth -= 1;
                cur.push(c);
            }
            ',' if depth == 0 => out.push(std::mem::take(&mut cur)),
            _ => cur.push(c),
        }
    }
    if !cur.trim().is_empty() {
        out.push(cur);
    }
    out
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
        assert_eq!(m.profile, ProfileName::Rv64im);
        assert_eq!(m.profile_byte(), 0);
    }

    #[test]
    fn parses_all_keys_and_blank_lines() {
        let m = parse(
            "// calc: profile = rv64imb\n\n// some prose\n\n// calc: input_max = 64\n//   calc: output_max = 32\n//calc:mem_kib=16\n// calc: hash = true\n",
        )
        .unwrap();
        assert_eq!(m.profile, ProfileName::Rv64imb);
        assert_eq!(m.input_max, 64);
        assert_eq!(m.output_max, 32);
        assert_eq!(m.mem_kib, MemSpec::KiB(16));
        assert!(m.hash);
        assert_eq!(m.profile_byte(), 0b101);
        let err = m.validate(&SpaceCap::new(0)).unwrap_err();
        assert!(err.contains("not admitted"), "{err}");
    }

    #[test]
    fn header_ends_at_first_code_line() {
        // A `// calc:` line after code must be ignored, not parsed.
        let m = parse(
            "pub fn kernel(_: &[u8], _: &mut [u8]) -> usize { 0 }\n// calc: profile = rv64im\n",
        )
        .unwrap();
        assert_eq!(m.profile, ProfileName::Rv64im);
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
            parse("// calc: hash = maybe\n")
                .unwrap_err()
                .contains("hash")
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
    fn block_comment_license_header_is_skipped() {
        let src = "/*\n * Copyright (c) 2026 nobody\n * SPDX-License-Identifier: MIT\n */\n// calc: profile = rv64imb\npub fn kernel(_: &[u8], _: &mut [u8]) -> usize { 0 }\n";
        let m = parse(src).unwrap();
        assert_eq!(m.profile, ProfileName::Rv64imb);
        // Single-line block comment + same-line close.
        let m2 = parse("/* header */\n// calc: input_max = 8\npub fn kernel(_: &[u8], _: &mut [u8]) -> usize { 0 }").unwrap();
        assert_eq!(m2.input_max, 8);
        assert_eq!(m2.profile, ProfileName::Rv64im);
    }

    #[test]
    fn infer_kernel_has_input_matches_both_shapes() {
        assert_eq!(
            infer_kernel_has_input(
                "// calc: profile = rv64im\npub fn hash(input: &[u8], out: &mut [u8]) -> usize { 0 }",
                "hash"
            )
            .unwrap(),
            true
        );
        assert_eq!(
            infer_kernel_has_input("pub fn tick( out : & mut [u8] ) -> usize { 0 }", "tick")
                .unwrap(),
            false
        );
        // `_input: &[u8]` counts as the input form (name is arbitrary).
        assert_eq!(
            infer_kernel_has_input(
                "pub fn hash(_input: &[u8], out: &mut [u8]) -> usize { 0 }",
                "hash"
            )
            .unwrap(),
            true
        );
        // Prefixed names must not match (`fn hashing` is not `fn hash`).
        assert!(
            infer_kernel_has_input(
                "pub fn hashing(input: &[u8], out: &mut [u8]) -> usize { 0 }",
                "hash"
            )
            .is_err()
        );
        // Wrong shapes error with the expected signatures.
        let err = infer_kernel_has_input("pub fn hash(x: u32) -> u32 { x }", "hash").unwrap_err();
        assert!(err.contains("input: &[u8]"), "{err}");
        assert!(infer_kernel_has_input("pub fn other() {}", "hash").is_err());
    }

    #[test]
    fn infer_ignores_fn_mentions_in_comments_and_literals() {
        // A doc comment before the real definition must not shadow it.
        let src = "/// Usage: `fn hash()` is not this one.\n\
                   // fn hash(x: u32)\n\
                   /* fn hash(y: u32) */\n\
                   const NOTE: &str = \"fn hash(z: u32)\";\n\
                   pub fn hash(input: &[u8], out: &mut [u8]) -> usize { 0 }";
        assert_eq!(infer_kernel_has_input(src, "hash").unwrap(), true);

        // Raw strings and char literals are skipped too.
        let src = "const A: &str = r#\"fn hash(q: u32)\"#;\n\
                   const B: char = 'f';\n\
                   pub fn tick(out: &mut [u8]) -> usize { 0 }";
        assert_eq!(infer_kernel_has_input(src, "tick").unwrap(), false);

        // Truncated input must not panic (used to index past `fn`).
        for src in ["fn", "fn hash", "pub fn hash(", "/* fn hash", "'"] {
            assert!(infer_kernel_has_input(src, "hash").is_err(), "{src:?}");
        }
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
