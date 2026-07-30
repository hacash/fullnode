use std::cell::RefCell;
use std::collections::HashSet as PrintHashSet;
use std::rc::Rc;

use crate::rt::SourceMap;

#[derive(Clone)]
pub struct PrintOption<'a> {
    pub indent: &'a str,
    pub tab: usize,
    pub map: Option<&'a SourceMap>,
    /// When enabled, emits source-map-derived `lib ...` declarations as a prelude.
    /// This should only be enabled for top-level printing. Inline printing must disable
    /// it to avoid injecting file-level declarations into expressions.
    pub emit_lib_prelude: bool,
    pub trim_root_block: bool,
    pub trim_head_alloc: bool,
    pub trim_param_unpack: bool,
    /// When enabled, hides the compiler-injected `nil` placeholder used by packed call argv.
    ///
    /// This is intentionally opt-in because `foo()` and `foo(nil)` are not equivalent in packed-call
    /// syntax once decompiled back to source.
    pub hide_default_call_argv: bool,
    pub call_short_syntax: bool,
    pub flatten_call_list: bool,
    pub flatten_array_list: bool,
    pub flatten_syscall_cat: bool,
    pub recover_literals: bool,
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
}

impl<'a> PrintOption<'a> {
    pub fn new(indent: &'a str, tab: usize) -> Self {
        Self {
            indent,
            tab,
            map: None,
            emit_lib_prelude: false,
            trim_root_block: false,
            trim_head_alloc: false,
            trim_param_unpack: false,
            hide_default_call_argv: false,
            call_short_syntax: false,
            flatten_call_list: false,
            flatten_array_list: false,
            flatten_syscall_cat: false,
            recover_literals: false,
            inline_mode: false,
            simplify_numeric_as_suffix: false,
            allocated: Rc::new(RefCell::new(PrintHashSet::new())),
            printed_consts: Rc::new(RefCell::new(PrintHashSet::new())),
            pending_consts: Rc::new(RefCell::new(Vec::new())),
            print_error: Rc::new(RefCell::new(None)),
            print_depth: Rc::new(RefCell::new(0)),
        }
    }

    pub fn fresh_runtime_state_clone(&self) -> Self {
        let mut next = self.clone();
        next.allocated = Rc::new(RefCell::new(PrintHashSet::new()));
        next.printed_consts = Rc::new(RefCell::new(PrintHashSet::new()));
        next.pending_consts = Rc::new(RefCell::new(Vec::new()));
        next.print_error = Rc::new(RefCell::new(None));
        next.print_depth = Rc::new(RefCell::new(0));
        next
    }

    pub fn reset_runtime_state(&self) {
        self.allocated.borrow_mut().clear();
        self.printed_consts.borrow_mut().clear();
        self.pending_consts.borrow_mut().clear();
        self.print_error.borrow_mut().take();
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
