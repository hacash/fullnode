use std::cell::RefCell;
use std::collections::HashSet as PrintHashSet;
use std::rc::Rc;


#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PrintContext {
    /// Statement position: the node is (part of) a statement line.
    Stmt,
    /// Expression position (call args, initializers, operands): block
    /// wrappers must survive and call short-syntax does not apply.
    Expr,
}

#[derive(Clone)]
pub struct PrintOption<'a> {
    pub indent: &'a str,
    pub tab: usize,
    pub map: Option<&'a SourceMap>,
    /// When enabled, emits source-map-derived `lib ...` declarations as a prelude. Top-level
    /// printing only — inline printing must disable it to avoid injecting file-level declarations.
    pub emit_lib_prelude: bool,
    /// When enabled, `resolve_lib_name` maps lib indices to sourcemap alias
    /// names (`AMM.swap` instead of `ext(1).swap`). Orthogonal to
    /// `emit_lib_prelude` (prelude output): a caller may resolve aliases
    /// without printing the binding lines, or print them without resolving.
    /// Callers that want the historical combined behavior set both.
    pub resolve_lib_alias: bool,
    pub trim_root_block: bool,
    pub trim_head_alloc: bool,
    pub trim_param_unpack: bool,
    /// When enabled, hides the compiler-injected `nil` placeholder for packed call argv. Opt-in
    /// because `foo()` and `foo(nil)` are not equivalent in packed-call syntax after decompiling.
    pub hide_default_call_argv: bool,
    pub call_short_syntax: bool,
    pub flatten_call_list: bool,
    pub flatten_array_list: bool,
    pub flatten_syscall_cat: bool,
    pub recover_literals: bool,
    /// Explicit statement/expression position. Replaces the old implicit
    /// inference (`trim_root_block` flipping in inline contexts) that decided
    /// call short-syntax; block-trim behavior keeps its own flag below.
    pub context: PrintContext,
    pub inline_mode: bool,
    /// When enabled, prints numeric literals with type suffix (e.g., `100u64`)
    /// instead of using `as` keyword (e.g., `100 as u64`).
    pub simplify_numeric_as_suffix: bool,
    // Tracking of printed slots/constants to avoid duplication.
    allocated: Rc<RefCell<PrintHashSet<u8>>>,
    printed_consts: Rc<RefCell<PrintHashSet<String>>>,
    pending_consts: Rc<RefCell<Vec<String>>>,
    print_error: Rc<RefCell<Option<String>>>,
    print_depth: Rc<RefCell<usize>>,
    // Slots whose sourcemap names must NOT be printed: bound by a raw
    // `unpack(...)` prelude that was not rewritten to `param { ... }`, so no
    // printed line declares the name — referencing it would not reparse.
    slot_names_suppressed: Rc<RefCell<PrintHashSet<u8>>>,
    // One-shot ticket: `trim_root_block` may trim exactly ONE tab-0 IRBLOCK
    // per root print session — the actual file-level root. Without it, root
    // statements printed at tab 0 (named channel) would let nested bodies
    // (e.g. a while body block) satisfy the same predicate and lose braces.
    root_trim_pending: Rc<RefCell<bool>>,
}

impl<'a> PrintOption<'a> {
    pub fn new(indent: &'a str, tab: usize) -> Self {
        Self {
            indent,
            tab,
            map: None,
            emit_lib_prelude: false,
            resolve_lib_alias: false,
            trim_root_block: false,
            trim_head_alloc: false,
            trim_param_unpack: false,
            hide_default_call_argv: false,
            call_short_syntax: false,
            flatten_call_list: false,
            flatten_array_list: false,
            flatten_syscall_cat: false,
            recover_literals: false,
            context: PrintContext::Stmt,
            inline_mode: false,
            simplify_numeric_as_suffix: false,
            allocated: Rc::new(RefCell::new(PrintHashSet::new())),
            printed_consts: Rc::new(RefCell::new(PrintHashSet::new())),
            pending_consts: Rc::new(RefCell::new(Vec::new())),
            print_error: Rc::new(RefCell::new(None)),
            print_depth: Rc::new(RefCell::new(0)),
            root_trim_pending: Rc::new(RefCell::new(false)),
            slot_names_suppressed: Rc::new(RefCell::new(PrintHashSet::new())),
        }
    }

    pub fn fresh_runtime_state_clone(&self) -> Self {
        let mut next = self.clone();
        next.allocated = Rc::new(RefCell::new(PrintHashSet::new()));
        next.printed_consts = Rc::new(RefCell::new(PrintHashSet::new()));
        next.pending_consts = Rc::new(RefCell::new(Vec::new()));
        next.print_error = Rc::new(RefCell::new(None));
        next.print_depth = Rc::new(RefCell::new(0));
        next.root_trim_pending = Rc::new(RefCell::new(false));
        next.slot_names_suppressed = Rc::new(RefCell::new(PrintHashSet::new()));
        next
    }

    pub fn reset_runtime_state(&self) {
        self.allocated.borrow_mut().clear();
        self.printed_consts.borrow_mut().clear();
        self.pending_consts.borrow_mut().clear();
        self.slot_names_suppressed.borrow_mut().clear();
        self.print_error.borrow_mut().take();
    }

    /// Arm the one-shot root-trim ticket for a fresh root print session.
    pub fn arm_root_trim_ticket(&self) {
        *self.root_trim_pending.borrow_mut() = true;
    }

    /// Consume the root-trim ticket if armed; only the first eligible block
    /// (the actual file-level root) gets it.
    pub fn take_root_trim_ticket(&self) -> bool {
        let mut pending = self.root_trim_pending.borrow_mut();
        if *pending {
            *pending = false;
            true
        } else {
            false
        }
    }

    pub fn begin_print_session(&self) -> bool {
        let mut depth = self.print_depth.borrow_mut();
        let is_root = *depth == 0;
        *depth += 1;
        is_root
    }

    pub fn end_print_session(&self) {
        let mut depth = self.print_depth.borrow_mut();
        if *depth > 0 {
            *depth -= 1;
        }
    }

    pub fn with_source_map(mut self, map: &'a SourceMap) -> Self {
        self.map = Some(map);
        self
    }

    pub fn with_tab(&self, tab: usize) -> Self {
        let mut next = self.clone();
        next.tab = tab;
        next
    }

    pub fn child(&self) -> Self {
        self.with_tab(self.tab + 1)
    }

    pub fn mark_slot_put(&self, slot: u8) -> bool {
        self.allocated.borrow_mut().insert(slot)
    }

    /// Suppress the sourcemap display name of `slot` (falls back to `$n`).
    /// Shared through child printers via the `Rc`, so a prelude decision made
    /// before the body prints stays in force for the whole block.
    pub fn suppress_slot_name(&self, slot: u8) {
        self.slot_names_suppressed.borrow_mut().insert(slot);
    }

    pub fn is_slot_name_suppressed(&self, slot: u8) -> bool {
        self.slot_names_suppressed.borrow().contains(&slot)
    }

    pub fn clear_slot_put(&self, slot: u8) {
        self.allocated.borrow_mut().remove(&slot);
    }

    pub fn clear_all_slot_puts(&self) {
        self.allocated.borrow_mut().clear();
    }

    pub fn mark_const_printed(&self, name: String) -> bool {
        self.printed_consts.borrow_mut().insert(name)
    }

    pub fn is_const_printed(&self, name: &str) -> bool {
        self.printed_consts.borrow_mut().contains(name)
    }

    pub fn set_print_error<S: Into<String>>(&self, msg: S) {
        let mut err = self.print_error.borrow_mut();
        if err.is_none() {
            *err = Some(msg.into());
        }
    }

    pub fn take_print_error(&self) -> Option<String> {
        self.print_error.borrow_mut().take()
    }

    pub fn add_pending_const(&self, name: String) {
        let mut pending = self.pending_consts.borrow_mut();
        if !pending.contains(&name) {
            pending.push(name);
        }
    }

    pub fn take_pending_consts(&self) -> Vec<String> {
        let mut pending = self.pending_consts.borrow_mut();
        std::mem::take(&mut *pending)
    }
}
