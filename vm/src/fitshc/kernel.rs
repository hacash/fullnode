//! Native fitsh-body rv64im kernel compilation.
//!
//! A `function rv64im <name>(...) { <fitsh body> }` declaration compiles the
//! body through the ordinary fitsh front end ([`compile_body_with_macros`])
//! and lowers the resulting IR with the prototype RV64IM backend
//! ([`crate::ir::rv64`]) into an admitted calc container — no rust toolchain
//! involved. The pinned translatable subset of the backend defines what a
//! kernel body may contain; anything outside it is a compile-time error that
//! names the offending opcode.
//!
//! Both call paths share this entry:
//!
//! * the pure library path (`compile_with_rv64im` without an
//!   `external_rv64im` entry) lowers with [`Rv64imBodyMeta::default`];
//! * the CLI pre-lowers with metadata parsed from the body's leading
//!   `// calc:` comment block and injects the bytes via `external_rv64im`,
//!   so the same parser code runs in both cases.

use crate::calc::container::{self, CALC_MEM_KIB_MIN, CalcContainer};
use crate::calc::verify::verify_container_with_cap;
use crate::ir::rv64::{self, Rv64Mode, ARENA_SIZE_MAX};
use crate::lang::Tokenizer;
use crate::rt::{SpaceCap, Token, TokenSpan};
use field::Address;
use crate::value::ValueTy;
use sys::{Ret, errf};

use super::state::MacroDef;
use super::compile_body::compile_body_with_macros;

/// Build metadata for a fitsh kernel body (the `// calc:` equivalent).
///
/// Defaults: the smallest-code lowering (`ImcRemap`, consensus profile 2),
/// L1 IO ceilings (`call_data_size` / `value_size`) and the 64 KiB L1
/// memory cap — the arena is declared at its maximum and the kernel traps
/// on exhaustion (a bounded-kernel guarantee, not a leak).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rv64imBodyMeta {
    /// Lowering mode. `Im` emits uncompressed profile 0; `Imc` and
    /// `ImcRemap` emit compressed profile 2 (`ImcRemap` is strictly
    /// smaller and is the default).
    pub mode: Rv64Mode,
    pub input_max: u16,
    pub output_max: u16,
    pub mem_kib: u32,
}

impl Default for Rv64imBodyMeta {
    fn default() -> Self {
        Self {
            mode: Rv64Mode::ImcRemap,
            input_max: SpaceCap::new(0).call_data_size as u16,
            output_max: SpaceCap::new(0).value_size as u16,
            mem_kib: 64,
        }
    }
}

impl Rv64imBodyMeta {
    /// Container profile byte for the lowering mode.
    pub fn profile_byte(&self) -> u8 {
        match self.mode {
            Rv64Mode::Im => container::CALC_PROFILE_RV64IM,
            _ => container::CALC_PROFILE_RV64IMC,
        }
    }

    /// Validate against `cap` (the same admission the chain applies).
    pub fn validate(&self, name: &str, cap: &SpaceCap) -> Ret<()> {
        if self.input_max as usize > cap.call_data_size {
            return errf!(
                "rv64im kernel '{name}': input_max {} exceeds call_data_size {}",
                self.input_max,
                cap.call_data_size
            );
        }
        if self.output_max as usize > cap.value_size {
            return errf!(
                "rv64im kernel '{name}': output_max {} exceeds value_size {}",
                self.output_max,
                cap.value_size
            );
        }
        if self.output_max == 0 {
            return errf!("rv64im kernel '{name}': output_max must be at least 1");
        }
        if self.mem_kib < CALC_MEM_KIB_MIN || self.mem_kib as u64 > cap.calc_mem_kib_max as u64 {
            return errf!(
                "rv64im kernel '{name}': mem_kib {} outside [{}, {}]",
                self.mem_kib,
                CALC_MEM_KIB_MIN,
                cap.calc_mem_kib_max
            );
        }
        Ok(())
    }
}

/// Compile one fitsh kernel body to verified calc-container bytes.
///
/// `body_tokens` are the tokens inside the braces (no `// calc:` comments —
/// the tokenizer already dropped them; metadata arrives via `meta`).
/// `arg_name` is the declared parameter name (`Some` for the
/// `(data: bytes)` form); the IR-level prelude binds it to slot 0 and the
/// lowering maps that slot to the ABI input registers.
pub fn compile_rv64im_body_kernel(
    name: &str,
    body_tokens: Vec<Token>,
    arg_name: Option<&str>,
    libs: &[(String, Address)],
    consts: &[(String, Box<dyn crate::ir::IRNode>)],
    macros: &[MacroDef],
    meta: &Rv64imBodyMeta,
    cap: &SpaceCap,
) -> Ret<Vec<u8>> {
    compile_rv64im_body_kernel_with_spans(
        name, body_tokens, &[], arg_name, libs, consts, macros, meta, cap,
    )
}

/// P2-D03 T1: same with the tokenizer's parallel span table — body-level
/// errors (`cannot find symbol`, subset rejects, …) carry `{line}:{col}`.
/// Token stream and compiled bytes are identical to
/// [`compile_rv64im_body_kernel`].
#[allow(clippy::too_many_arguments)]
pub fn compile_rv64im_body_kernel_with_spans(
    name: &str,
    body_tokens: Vec<Token>,
    body_spans: &[TokenSpan],
    arg_name: Option<&str>,
    libs: &[(String, Address)],
    consts: &[(String, Box<dyn crate::ir::IRNode>)],
    macros: &[MacroDef],
    meta: &Rv64imBodyMeta,
    cap: &SpaceCap,
) -> Ret<Vec<u8>> {
    compile_rv64im_body_kernel_at(
        name, body_tokens, body_spans, arg_name, libs, consts, macros, meta, cap, None,
    )
}

/// P2-D03 T1: variant with the source-file name for `{file}:{line}:{col}`
/// error prefixes (CLI entry).
#[allow(clippy::too_many_arguments)]
pub fn compile_rv64im_body_kernel_at(
    name: &str,
    body_tokens: Vec<Token>,
    body_spans: &[TokenSpan],
    arg_name: Option<&str>,
    libs: &[(String, Address)],
    consts: &[(String, Box<dyn crate::ir::IRNode>)],
    macros: &[MacroDef],
    meta: &Rv64imBodyMeta,
    cap: &SpaceCap,
    file: Option<&str>,
) -> Ret<Vec<u8>> {
    meta.validate(name, cap)?;
    let args: Vec<(String, ValueTy)> = arg_name
        .map(|n| (n.to_string(), ValueTy::Bytes))
        .into_iter()
        .collect();
    // is_ircode=false: the rv64 lowering consumes the IRNodeArray directly;
    // the HVM bytecode channel would additionally require the runtime-IR
    // conversion, which the kernel subset never needs.
    let (ir, _compiled, _smap) =
        compile_body_with_macros(
            body_tokens, args, libs, consts, false, macros, body_spans, file,
        )
        .map_err(|e| sys::Error::fault(format!("rv64im kernel '{name}': {e}")))?;
    // Arena budget: the memory window is [CODE_BASE, CODE_BASE + mem_kib·1K)
    // and the arena must end far enough below the IO+stack region that the
    // layout fits (guest_end + guard + stack ≤ output_base, with up to 15 B
    // of alignment loss on each IO base). ARENA_SIZE_MAX is the calibration
    // for the L1 ceilings with small IO; a large input/output declaration
    // shrinks the arena (kernels trap on exhaustion instead of failing
    // admission).
    const STACK_GAP: u64 = crate::calc::CALC_STACK_GUARD + crate::calc::CALC_STACK_SIZE;
    let arena_budget = (meta.mem_kib as u64)
        .saturating_mul(1024)
        .saturating_sub(rv64::ARENA_BASE - crate::calc::container::CODE_BASE as u64)
        .saturating_sub(meta.input_max as u64)
        .saturating_sub(meta.output_max as u64)
        .saturating_sub(STACK_GAP)
        .saturating_sub(30); // (& !15) alignment loss on both IO bases
    if arena_budget < 1024 {
        return errf!(
            "rv64im kernel '{name}': mem_kib {} leaves no arena room (need ≥ {} KiB total)",
            meta.mem_kib,
            (rv64::ARENA_BASE + meta.input_max as u64 + meta.output_max as u64 + STACK_GAP + 1024 + 64 + 1023) / 1024
        );
    }
    let arena_size = ARENA_SIZE_MAX.min(arena_budget);
    // task 7: the per-grow heap segment cap rides the same parameter
    // channel as the arena budget — the live SpaceCap field, never a
    // kernel-side literal
    let lo =
        rv64::lower_body_mode_arena_heap(&ir, arg_name.is_some(), meta.mode, arena_size, cap.heap_grow_segment)
        .map_err(|e| {
            sys::Error::fault(annotate_backend_reject(&format!(
                "rv64im kernel '{name}': {e}"
            )))
        })?;
    let container = lo
        .pack_segments_profile(cap, meta.mem_kib, meta.input_max, meta.output_max, meta.profile_byte())
        .map_err(|e| sys::Error::fault(format!("rv64im kernel '{name}': {e}")))?;
    verify_container_with_cap(&container, cap).map_err(|e| sys::Error::fault(format!("rv64im kernel '{name}': {e}")))?;
    let buf = container.encode().map_err(|e| sys::Error::fault(format!("rv64im kernel '{name}': {e}")))?;
    Ok(buf)
}

/// Tokenize raw fitsh body source (the CLI keeps the comment-bearing text;
/// the tokens are re-derived so the compile path is identical to the
/// library's parser-driven one).
pub fn tokenize_body(body: &str) -> Ret<Vec<Token>> {
    Tokenizer::new(body.as_bytes())
        .parse()
        .map_err(|e| sys::Error::fault(e.to_string()))
}

/// P2-D03 T9/T2: user-facing rewrite of backend-reject messages at the
/// fitshc error boundary (pure message rewriting; no accept/reject semantics
/// change; the rv64 backend's own messages stay byte-identical for direct
/// lowering callers like the oracle/C3 harnesses).
///
/// * T9 — the expression-nesting reject gains the split hint.
/// * T2 — `opcode {X} is outside the translatable kernel subset` is
///   translated to the source-level construct for the opcodes in
///   [`OP_CONSTRUCT`]. Opcodes not in the table pass through verbatim — in
///   particular the u128 reject family, which external suites pin by the
///   `CU128` name.
pub fn annotate_backend_reject(msg: &str) -> String {
    const NESTING: &str =
        "expression nesting exceeds the 8-value register pool (kernel subset limit)";
    if let Some(idx) = msg.find(NESTING) {
        let cut = idx + NESTING.len();
        let mut out = String::with_capacity(msg.len() + 96);
        out.push_str(&msg[..cut]);
        out.push_str(
            "; hint: split the expression with intermediate variables \
             (e.g. `var t1 = a + b`) so fewer operands are live at once",
        );
        out.push_str(&msg[cut..]);
        return out;
    }

    const REJECT_MARK: &str = " is outside the translatable kernel subset";
    if let Some(pos) = msg.find(REJECT_MARK) {
        // `...: rv64 lower: opcode {OP}` + mark.
        if let Some(op_start) = msg[..pos].rfind("opcode ") {
            let op = &msg[op_start + "opcode ".len()..pos];
            if let Some((desc, hint)) = op_construct(op) {
                // Everything before `rv64 lower:` is the kernel-name prefix
                // (`rv64im kernel '{name}': `) — keep it (external suites
                // assert it).
                let head = &msg[..op_start.saturating_sub("rv64 lower: ".len())];
                let head = if head.is_empty() { "" } else { head };
                return format!(
                    "{head}the kernel subset does not support {desc}; hint: {hint}"
                );
            }
        }
    }
    msg.to_string()
}

/// IR opcode (Debug name) → the user-facing source construct it comes from,
/// with an actionable hint. Conservative table: only opcodes whose source
/// construct is unambiguous. `**` (POW) is in the subset, so no entry is
/// needed for it despite doc 66 §3's provisional `IRPOW` name.
///
/// P3-V2IMPL (A-25 M1): the compo-list v2 shape set LOWERS in-shape
/// (`[e0..eN-1]` creation / `[]` / append / item_get / clear / length /
/// has_key on list VARIABLES). T17 (the arena container model) added the
/// general list VALUE forms — alias/re-bind/CLONE, nested elements (the
/// runtime CastBeValueFail 76 face), TAKEFIRST/TAKELAST, MERGE, the true
/// shifting INSERT/REMOVE, KEYS/VALUES(list) (CompoOpNotMatch 83), list
/// EQ/NEQ, the list cast/ordered-compare/XOP fault faces and the CHOOSE
/// descriptor select. The rows below describe only the shapes that still
/// reject (non-variable receivers, unrepresentable element widths, may-Nil
/// stores, tuples).
const OP_CONSTRUCT: &[(&str, &str, &str)] = &[
    (
        "ITEMGET",
        // doc 73 E6 correction: `data[i]` on a bytes receiver is a RUNTIME
        // CompoOpNotMatch(83) on the interpreter, not an indexing form —
        // the kernel subset rejects it at compile time instead
        "indexing a value that is not a list variable (`data[i]` indexes \
         bytes, which the interpreter rejects at run time)",
        "read fixed byte positions with `byte(data, i)`, take whole ranges \
         with buf_left/buf_right/cut, and index only list variables \
         (`var l = [..]; l[i]`)",
    ),
    (
        "PACKLIST",
        "a list construction that escapes the supported list shapes (more \
         items than the block budget (128 at u8..u64 width, 64 at u128 \
         width), mixed element widths, or an element that may be Nil)",
        "lists support up to 128 unconditional uint elements at u8..u64 \
         width (64 at u128 width) — make every element an \
         unconditionally-assigned uint of one width",
    ),
    (
        "NEWLIST",
        "an empty list literal (`[]` / new_list()) used as a value instead \
         of bound to a variable",
        "bind the list first (`var w = []`) and mutate it with append/clear",
    ),
    (
        "APPEND",
        "append into a value that is not a list variable",
        "append(l, v) requires `l` to be a list variable created with \
         `[..]` or `[]` (appending into expressions or parameters is not \
         supported)",
    ),
    (
        "CLEAR",
        "clear of a value that is not a list variable",
        "clear(l) requires `l` to be a list variable created with `[..]` \
         or `[]`",
    ),
    (
        "LENGTH",
        "length() of a value that is not a list variable",
        "length(l) requires `l` to be a list variable created with `[..]` \
         or `[]` (for byte strings use size(b))",
    ),
    (
        "HASKEY",
        "has_key() of a value that is not a list variable",
        "has_key(l, i) requires `l` to be a list variable created with \
         `[..]` or `[]`",
    ),
    (
        "INSERT",
        "an insertion into a value that is not a list or map variable, or a \
         list insert whose index or value is not a definite uint",
        "lists support `insert(l, i, v)` with a u64 index and a u64 value \
         (pair it with remove(l, i) for a shifting edit); maps support \
         `insert(m, k, v)` freely",
    ),
    (
        "PACKTUPLE",
        "tuple construction (tuple(..))",
        "the kernel subset has no tuples — return values one at a time or pack \
         them into bytes",
    ),
    (
        "CALL",
        "calling another contract function",
        "inline the callee's body, or move shared code into the kernel",
    ),
];

fn op_construct(op: &str) -> Option<(&'static str, &'static str)> {
    OP_CONSTRUCT
        .iter()
        .find(|(name, _, _)| *name == op)
        .map(|(_, desc, hint)| (*desc, *hint))
}

/// Decode the compiled container (for artifact reporting).
pub fn decode_container(bytes: &[u8]) -> Ret<CalcContainer> {
    let (c, used) = container::CalcContainer::decode(bytes).map_err(|e| sys::Error::fault(e.to_string()))?;
    if used != bytes.len() {
        return errf!("calc container trailing bytes");
    }
    Ok(c)
}

#[cfg(test)]
mod annotate_tests {
    use super::annotate_backend_reject;

    #[test]
    fn nesting_reject_appends_split_hint() {
        let m = annotate_backend_reject(
            "rv64im kernel 'k': rv64 lower: expression nesting exceeds the \
             8-value register pool (kernel subset limit)",
        );
        assert!(m.contains("nesting exceeds"), "{m}");
        assert!(m.contains("hint: split the expression with intermediate"), "{m}");
    }

    #[test]
    fn itemget_reject_is_translated_with_prefix_kept() {
        let m = annotate_backend_reject(
            "rv64im kernel 'fold': rv64 lower: opcode ITEMGET is outside the \
             translatable kernel subset",
        );
        assert!(m.starts_with("rv64im kernel 'fold': "), "{m}");
        // P3-V2IMPL: the desc names the bytes-receiver runtime mismatch
        // (doc 73 E6 correction) — indexing bytes is not a subset form
        assert!(m.contains("not a list variable"), "{m}");
        assert!(m.contains("hint: read fixed byte positions"), "{m}");
        assert!(!m.contains("opcode"), "{m}");
    }

    #[test]
    fn list_escape_rows_translate_with_reason() {
        // P3-V2IMPL: the escape paths keep the canonical backend text (the
        // fitshc boundary rewrites them); each row names the construct and
        // the escape reason
        for (op, needle) in [
            ("PACKLIST", "escapes the supported list shapes"),
            ("NEWLIST", "used as a value instead of bound"),
            ("APPEND", "not a list variable"),
            ("CLEAR", "not a list variable"),
            ("LENGTH", "not a list variable"),
            ("HASKEY", "not a list variable"),
        ] {
            let raw = format!(
                "rv64im kernel 'k': rv64 lower: opcode {op} is outside the \
                 translatable kernel subset"
            );
            let m = annotate_backend_reject(&raw);
            assert!(m.contains(needle), "{op}: {m}");
            assert!(!m.contains("opcode"), "{op}: {m}");
        }
    }

    #[test]
    fn unmapped_opcodes_pass_through_verbatim() {
        // The CU128 family is pinned by external suites via the opcode name.
        let raw = "rv64im kernel 'k': rv64 lower: opcode CU128 is outside the \
                   translatable kernel subset";
        assert_eq!(annotate_backend_reject(raw), raw);
        let raw2 = "rv64im kernel 'k': rv64 lower: u128 values are not in the kernel subset";
        assert_eq!(annotate_backend_reject(raw2), raw2);
    }

    #[test]
    fn unknown_messages_pass_through() {
        let raw = "rv64im kernel 'k': rv64 lower: value stack not empty at function end";
        assert_eq!(annotate_backend_reject(raw), raw);
    }
}
