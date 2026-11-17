use super::super::*;


/* ------------------------------------------------------------------ */
/* P2 arena container object model (§3 shared prework for T16/T17).    */
/*                                                                     */
/* The interpreter keeps containers behind `Rc<RefCell<Compo>>`         */
/* (value/compo.rs:3-7,244-249): list = `VecDeque<Value>` (insertion    */
/* order), map = `BTreeMap<canonical-minimal-BE-key-bytes, Value>`      */
/* (byte lexicographic order). DUP/alias PUT clone the Rc (one shared   */
/* object; in-place mutation is visible through every alias), CLONE is  */
/* the only deep copy (`copy_with_stats`; nested Compo elements keep    */
/* their Rc's — the copy is one level deep).                           */
/*                                                                     */
/* The kernel reproduces this with arena objects + descriptor slots:    */
/*                                                                     */
/*   * OBJECTS live in the existing RW bump arena (S1, shared with the  */
/*     bytes CAT allocations; exhaustion = the kernel budget face that  */
/*     `emit_arena_check` already implements).                         */
/*   * A DESCRIPTOR is one u64: the object's arena base address. A List */
/*     slot's 8-byte frame cell holds it (the Growable length shadow's  */
/*     cell, repurposed); a container VALUE on the vstack is one pool   */
/*     register holding it (statically typed List/Map — a container     */
/*     never changes species). Alias PUT / stack DUP copies the word.   */
/*                                                                     */
/* List object layout (8-byte aligned, bump-allocated):                 */
/*                                                                     */
/*   +0      u64 len   — live element count (aliases mutate THIS)       */
/*   +8      u64 cap   — element capacity, fixed at alloc               */
/*   +16..   element cells, `CTR_ELEM_BYTES` each, `cap` of them:       */
/*             +0  u64 tag — the element's Value type id (Nil=0,       */
/*                 Bool=1, U8..U128=2..6, Bytes=8, Address=9,          */
/*                 Compo=14; see `ctr_uint_type_tag`)                  */
/*             +8  u64 a   — payload word 0: uint value / (lo) half /   */
/*                 bytes|address ptr / nested container descriptor      */
/*             +16 u64 b   — payload word 1: (hi) half / bytes len /    */
/*                 nested container sub-tag (0 list, 1 map)            */
/*                                                                     */
/* The tagged cell is the general element representation: it can hold   */
/* every scalar (and, for T17, a nested container descriptor). The P2   */
/* admission keeps the v2-shape element policy (one fixed uint width    */
/* per slot, may-Nil stores compile-rejected) — the tag machinery is    */
/* the hook T17 relaxes (mixed scalar elements, Nil/Bool/Bytes/Address  */
/* elements, nested lists with the runtime 76 face where the            */
/* interpreter's check_scalar fires).                                  */
/*                                                                     */
/* Map object layout (T16, source-lowered):                             */
/*                                                                     */
/*   +0  u64 len   — live entry count                                  */
/*   +8  u64 cap   — entry capacity, fixed at alloc (CTR_MAP_CAP = 128) */
/*   +16 u64 kused — reserved (kept for the 8-byte header alignment)    */
/*   +24.. entry cells, `CTR_MAP_ENT` (40) bytes each, kept SORTED by   */
/*        the canonical key bytes (byte-lexicographic, the BTreeMap     */
/*        order — KEYS/VALUES iterate entries in this order):          */
/*             +0  u64 kptr — ABSOLUTE arena address of the key bytes  */
/*             +8  u64 klen — key byte length                          */
/*             +16 u64 vtag — value type id (same table as elements;   */
/*                 8 = Bytes, ctr_uint_type_tag for uints)             */
/*             +24 u64 va   — value payload word 0 (lo / bytes ptr)    */
/*             +32 u64 vb   — value payload word 1 (hi / bytes len)    */
/*   Key bytes are private copies in the shared arena bump stream (a     */
/*   removed key's bytes are dead space; entries shift, the kptr stays  */
/*   valid). A MAP slot's descriptor is `ptr | CTR_MAP_DESC_TAG`; list  */
/*   descriptors are the bare pointer.                                  */
/*                                                                     */
/* T16 map faces: Bool/Nil/empty-Bytes key → 73; duplicate PACKMAP /   */
/* MERGE key → 84; missing itemget/remove/item_set key → 85; self-    */
/* MERGE → 80; 129th NEW key / merge overflow → 40; non-scalar map     */
/* value is a compile reject (B-family).                               */
/*                                                                     */
/* Fault faces (the iron rule: identical to the interpreter where the   */
/* rejection is not a kernel budget face): append past cap → 40         */
/* (`OutOfCompoLen`, compo.rs:160-172 — CTR_LIST_CAP mirrors the        */
/* `SpaceCap::compo_length` default 128, so the face fires at exactly   */
/* the interpreter's boundary); itemget/itemset index ≥ len → 85;       */
/* index ≥ 2³² / Nil index → 72; arena exhaustion → the CAT precedent    */
/* (fault_cast). Billing numbers differ (allowed).                     */
/* ------------------------------------------------------------------ */

/// P2: element capacity of every arena list object. Mirrors the
/// interpreter's `SpaceCap::compo_length` default (rt/cap.rs: 128), so the
/// append 40-face fires at the same element count the interpreter faults.
pub(crate) const CTR_LIST_CAP: u64 = 128;

/// P2: bytes per arena list element cell (tag, a, b — see the model doc).
pub(crate) const CTR_ELEM_BYTES: u64 = 24;

/// P2: list object header bytes (len, cap); element cells start at +16.
pub(crate) const CTR_LIST_HDR: u64 = 16;

/// P2: map entry cell bytes (kptr, klen, vtag, va, vb — T16).
pub(crate) const CTR_MAP_ENT: i64 = 40;

/// P2: map object header bytes (len, cap, kused); entries start at +24.
pub(crate) const CTR_MAP_HDR: i64 = 24;

/// T16: map element capacity — the interpreter's `SpaceCap::compo_length`
/// default (rt/cap.rs:128). INSERT of a NEW key at len ≥ 128 → 40; a map
/// value's length can never exceed it.
pub(crate) const CTR_MAP_CAP: u64 = 128;

/// T16: map descriptor low-bit species tag. Arena objects are 8-byte
/// aligned; a LIST descriptor is the bare pointer (bit 0 = 0), a MAP
/// descriptor is `ptr | CTR_MAP_DESC_TAG`. Map code masks the tag off
/// before dereferencing; list code is untouched. The object header word 0
/// stays the live length in both species, so a runtime species test can
/// also read the tag bit directly.
pub(crate) const CTR_MAP_DESC_TAG: u64 = 1;

/// P2: the Value type id a stored uint element of width `ew` bits carries
/// in its tag word (U8=2..U128=6 — the interpreter's ValueTy table, so a
/// T17/T12 type test on a spilled element reads the same id the
/// interpreter's `Value::type_id` answers).
pub(crate) fn ctr_uint_type_tag(ew: u32) -> u64 {
    match ew {
        w if w <= 8 => 2,
        w if w <= 16 => 3,
        w if w <= 32 => 4,
        w if w <= 64 => 5,
        _ => 6,
    }
}

/// P2: canonical map-key bytes of a uint (the interpreter's
/// `uint_key_bytes`, value/util.rs:37 — minimal big-endian, zero →
/// `[0x00]`; `U8(1)` and `U64(1)` share one encoding). Kept as a local
/// mirror so the compiler side never depends on the interpreter module
/// graph; `rv64_arena_containers` differentials pin it against
/// `Value::extract_key_bytes`.
#[allow(dead_code)] // T16 skeleton hook (constant-key rodata maps use it directly)
pub(crate) fn ctr_map_key_bytes(n: u128) -> Vec<u8> {
    let be = n.to_be_bytes();
    let first_nz = be.iter().position(|b| *b != 0).unwrap_or(be.len());
    let trimmed = &be[first_nz..];
    if trimmed.is_empty() {
        vec![0u8]
    } else {
        trimmed.to_vec()
    }
}

/* ------------------------------------------------------------------ */
/* P2 map skeleton (T16 hook surface). Free functions over `&mut Asm`  */
/* so they can be emitted and decoder-verified standalone (the live    */
/* list-model helpers sit on `Lower` because they read slot cells;     */
/* these take every operand as an explicit register). None touches a   */
/* pooled value register unless it is passed in.                       */
/* ------------------------------------------------------------------ */

/// Emit the canonical map-key normalization of a uint of width `wbytes`
/// ∈ 1..=16 (compile-time) held as `lo` (bits 0..64) and, at the u128
/// width, `hi` (bits 64..128): the minimal big-endian key bytes bumped
/// into the RW arena — outputs `kptr`/`klen`, zero → the single byte
/// `0x00`, exactly `uint_key_bytes` (value/util.rs:37). The value's arena
/// slot is `wbytes` bytes (the minimal key is never longer); exhaustion →
/// `fault` (the CAT budget face).
///
/// Mechanism (fully unrolled — `wbytes` is compile-time): BE byte index
/// `j` lives at bit offset `(wbytes-1-j)*8` of the value. The first
/// nonzero BE byte `j` fixes `klen = wbytes-j`; the key bytes are then
/// extracted with constant shifts, so a u8(1) and a u64(1) both yield the
/// canonical `[0x01]` and zero yields `[0x00]`.
pub(crate) fn emit_ctr_map_key_uint(
    asm: &mut Asm,
    lo: u8,
    hi: Option<u8>,
    wbytes: u32,
    kptr: u8,
    klen: u8,
    fault: usize,
    arena_end: u64,
) {
    debug_assert!((1..=16).contains(&wbytes));
    debug_assert!(wbytes == 16 || hi.is_none());
    // arena check (S1 + wbytes <= arena_end) and bump, using klen/kptr as
    // the two transient registers (both are outputs, set below)
    asm.li(klen, wbytes as u64);
    asm.add(klen, S1, klen);
    asm.li(kptr, arena_end);
    asm.bgeu(klen, kptr, fault);
    asm.mv(kptr, S1);
    asm.addi(S1, S1, wbytes as i32);
    // byte at LE position p lives in lo (p < 8) or hi (p >= 8)
    let byte_src = |p: u32| -> (u8, u32) {
        if p < 8 {
            (lo, p * 8)
        } else {
            (hi.expect("u128 key needs the hi half"), (p - 8) * 8)
        }
    };
    let l_done = asm.new_label();
    let mut marks = Vec::with_capacity(wbytes as usize);
    for j in 0..wbytes {
        let l_nz = asm.new_label();
        let (r, sh) = byte_src(wbytes - 1 - j); // BE byte j
        asm.srli_(klen, r, sh);
        asm.andi(klen, klen, 0xff);
        asm.bne(klen, ZERO, l_nz);
        marks.push(l_nz);
    }
    // every BE byte zero: klen = 1, key = [0x00]
    asm.addi(klen, ZERO, 1);
    asm.sb(ZERO, kptr, 0);
    asm.jal_(ZERO, l_done);
    for (j, l_nz) in marks.into_iter().enumerate() {
        let j = j as u32;
        asm.place(l_nz);
        for t in 0..(wbytes - j) {
            let (r, sh) = byte_src(wbytes - 1 - j - t);
            asm.srli_(klen, r, sh);
            asm.andi(klen, klen, 0xff);
            asm.sb(klen, kptr, t as i32);
        }
        asm.addi(klen, ZERO, (wbytes - j) as i32);
        asm.jal_(ZERO, l_done);
    }
    asm.place(l_done);
}

/// Emit the sorted-table lookup of a map object (`obj`): scan the entry
/// array for the first entry whose canonical key is byte-lexicographically
/// >= the target (`kptr`/`klen`). `pos` := that entry index — the INSERT
/// POSITION, `len` when every entry is smaller; `found` := 1 iff the
/// entry at `pos` exists and its key equals the target. This is the T16
/// NEWMAP/PACKMAP/INSERT/REMOVE/HASKEY/ITEMGET/ITEMSET/MERGE core: the
/// BTreeMap byte-lex order IS the entry-array order (KEYS/VALUES iterate
/// it as-is), and PACKMAP/MERGE duplicate-key checks are `found` reads
/// before any mutation (the interpreter's 84 ordering, compo.rs:352/507).
///
/// Entry layout (absolute key pointers — key bytes live in the RW arena
/// bump stream, so removing an entry never invalidates the live keys):
/// `+0 kptr, +8 klen, +16 vtag, +24 va, +32 vb`.
///
/// Register discipline: `obj`/`kptr`/`klen`/`pos`/`found` are caller
/// values; `x`,`y`,`z` are three pairwise-distinct scratch registers (in
/// ImcRemap `a1` and `sc2` are the SAME physical register, so the routine
/// is written for exactly three). It recomputes the entry base per
/// comparison byte, which keeps the byte compare inside three registers.
pub(crate) fn emit_ctr_map_find(
    asm: &mut Asm,
    obj: u8,
    kptr: u8,
    klen: u8,
    pos: u8,
    found: u8,
    x: u8,
    y: u8,
    z: u8,
) {
    debug_assert!(x != obj && x != kptr && x != klen && x != pos && x != found);
    debug_assert!(y != obj && y != kptr && y != klen && y != pos && y != found);
    debug_assert!(z != obj && z != kptr && z != klen && z != pos && z != found);
    debug_assert!(x != y && y != z && x != z);
    let l_loop = asm.new_label();
    let l_cmp = asm.new_label();
    let l_prefix = asm.new_label();
    let l_next = asm.new_label();
    let l_lt = asm.new_label();
    let l_gt = asm.new_label();
    let l_eq = asm.new_label();
    let l_nf = asm.new_label();
    let l_end = asm.new_label();
    let l_min = asm.new_label();
    let l_minv = asm.new_label();
    asm.addi(pos, ZERO, 0);
    asm.place(l_loop);
    asm.ld(x, obj, 0); // len
    asm.bgeu(pos, x, l_nf);
    // ebase = obj + 24 + pos*40 ; klen_e = [ebase+8]
    asm.slli_(x, pos, 5); // pos*32
    asm.slli_(y, pos, 3); // pos*8
    asm.add(x, x, y); // pos*40
    asm.addi(x, x, CTR_MAP_HDR as i32);
    asm.add(x, obj, x); // ebase
    asm.ld(y, x, 8); // klen_e
    // found := min(klen_e, klen) — the common-prefix compare bound
    asm.mv(found, klen);
    asm.bltu(y, klen, l_min);
    asm.jal_(ZERO, l_minv);
    asm.place(l_min);
    asm.mv(found, y);
    asm.place(l_minv);
    asm.addi(z, ZERO, 0); // m
    asm.place(l_cmp);
    asm.bgeu(z, found, l_prefix);
    // byte at entry key [m]: recompute ebase (3 scratch regs only)
    asm.slli_(x, pos, 5);
    asm.slli_(y, pos, 3);
    asm.add(x, x, y);
    asm.addi(x, x, CTR_MAP_HDR as i32);
    asm.add(x, obj, x);
    asm.ld(x, x, 0); // kptr_e
    asm.add(x, x, z);
    asm.lbu(y, x, 0); // a = kptr_e[m]
    asm.add(x, kptr, z);
    asm.lbu(x, x, 0); // b = kptr[m]
    asm.beq(y, x, l_next);
    asm.bltu(y, x, l_lt); // a < b
    asm.jal_(ZERO, l_gt); // a > b
    asm.place(l_next);
    asm.addi(z, z, 1);
    asm.jal_(ZERO, l_cmp);
    asm.place(l_prefix);
    // equal prefix: the shorter key is smaller (BTreeMap byte-lex)
    asm.slli_(x, pos, 5);
    asm.slli_(y, pos, 3);
    asm.add(x, x, y);
    asm.addi(x, x, CTR_MAP_HDR as i32);
    asm.add(x, obj, x);
    asm.ld(x, x, 8); // klen_e
    asm.bltu(x, klen, l_lt);
    asm.beq(x, klen, l_eq);
    asm.jal_(ZERO, l_gt);
    asm.place(l_lt);
    asm.addi(pos, pos, 1);
    asm.jal_(ZERO, l_loop);
    asm.place(l_gt);
    asm.addi(found, ZERO, 0);
    asm.jal_(ZERO, l_end);
    asm.place(l_eq);
    asm.addi(found, ZERO, 1);
    asm.jal_(ZERO, l_end);
    asm.place(l_nf);
    asm.addi(found, ZERO, 0);
    asm.place(l_end);
}

impl<'a> Lower<'a> {

    /* ================================================================ *
     * P2 (§3 shared prework): arena container descriptor emission.      *
     *                                                                  *
     * This block is the T17/T16 hook surface. `ListKind::Arena` slots   *
     * keep their object pointer in the slot's own 8-byte frame cell;    *
     * the helpers below move descriptors and tagged element cells       *
     * between that cell, the vstack, and the RW bump arena (S1).        *
     * Scratch discipline: sc1/sc2 (the mode-mapped dedicated pair) and  *
     * the mode-mapped a0/a1 pair only — never a pooled value register   *
     * unless explicitly passed as dst/src.                              *
     * ================================================================ */

    /// `dst` := the arena object descriptor held by list slot `s`.
    pub(crate) fn ctr_load_obj(&mut self, s: usize, dst: u8) {
        let cell = self.slot_offset(s as u8);
        self.asm.ld(dst, S0, cell);
    }

    /// `dst` := the live element count of the list object in `obj`
    /// (header word 0 — the word every alias's mutation lands on).
    #[allow(dead_code)] // T17 hook (TAKEFIRST/TAKELAST/MERGE length faces)
    pub(crate) fn ctr_len_of(&mut self, dst: u8, obj: u8) {
        self.asm.ld(dst, obj, 0);
    }

    /// `dst` := `obj` + CTR_LIST_HDR + `idx` * CTR_ELEM_BYTES — the
    /// address of element cell `idx`. All four registers must be pairwise
    /// distinct (ImcRemap maps a1/sc2 to the same physical x31).
    pub(crate) fn ctr_elem_addr(&mut self, dst: u8, obj: u8, idx: u8, tmp: u8) {
        debug_assert!(dst != obj && dst != idx && dst != tmp);
        debug_assert!(obj != idx && obj != tmp && idx != tmp);
        self.asm.slli_(dst, idx, 3); // idx*8
        self.asm.slli_(tmp, idx, 4); // idx*16
        self.asm.add(dst, dst, tmp); // idx*24
        self.asm.addi(dst, dst, CTR_LIST_HDR as i32);
        self.asm.add(dst, obj, dst);
    }

    /// Store the uint element (width `ew`) in `lo` (and `hi` at the u128
    /// width) into the element cell at `addr` as a tagged cell
    /// (tag = the Value type id of the width). The tag temporary is the
    /// mode-mapped scratch a0 — never a pool register and never `addr`
    /// (the creation path passes sc1 as the address).
    pub(crate) fn ctr_store_elem(&mut self, addr: u8, ew: u32, lo: u8, hi: Option<u8>) {
        let tmp = self.a0;
        let tag = ctr_uint_type_tag(ew) as i32;
        self.asm.addi(tmp, ZERO, tag);
        self.asm.sd(tmp, addr, 0);
        self.asm.sd(lo, addr, 8);
        match hi {
            Some(h) => self.asm.sd(h, addr, 16),
            None => self.asm.sd(ZERO, addr, 16),
        }
    }

    /// Load the uint element payload (width `ew`) from the element cell at
    /// `addr` into `dst` (and `dst_hi` at the u128 width). The tag word is
    /// not re-verified: the P2 admission stores one fixed uint width per
    /// slot (the plan's width replay guarantees it), so the payload is the
    /// whole cell; T17's mixed-element relaxation adds its tag dispatch
    /// here.
    pub(crate) fn ctr_load_elem(&mut self, dst: u8, dst_hi: Option<u8>, addr: u8) {
        self.asm.ld(dst, addr, 8);
        if let Some(h) = dst_hi {
            self.asm.ld(h, addr, 16);
        }
    }

    /// Arena twin of `emit_list_bounds_guards` for a variable index: the
    /// 2³² extract_u32 face (72), then `idx ≥ len → 85` with the length
    /// read THROUGH the descriptor (the shared header word — the live
    /// state every alias's mutation lands on).
    pub(crate) fn emit_list_bounds_guards_arena(&mut self, idx_reg: u8, s: usize) -> Ret<()> {
        let l72 = self.nil_thunk(72);
        let sc1 = self.asm.sc1;
        self.asm.li(sc1, 1u64 << 32);
        self.asm.bgeu(idx_reg, sc1, l72);
        let l85 = self.nil_thunk(85);
        let cell = self.slot_offset(s as u8);
        self.asm.ld(sc1, S0, cell);
        self.asm.ld(sc1, sc1, 0);
        self.asm.bgeu(idx_reg, sc1, l85);
        Ok(())
    }

    /// Arena twin of `emit_list_load`: element `idx_reg` of the slot's
    /// object into `dst` (and `dst_hi` at the u128 width). `idx_reg` is
    /// only read (it may be a bank Ref home).
    pub(crate) fn emit_list_load_arena(
        &mut self,
        dst: u8,
        dst_hi: Option<u8>,
        idx_reg: u8,
        s: usize,
    ) -> Ret<()> {
        let sc1 = self.asm.sc1;
        let sc2 = self.asm.sc2;
        self.asm.ld(sc2, S0, self.slot_offset(s as u8)); // the descriptor
        self.ctr_elem_addr(sc1, sc2, idx_reg, self.a0);
        self.ctr_load_elem(dst, dst_hi, sc1);
        Ok(())
    }

    /// Allocate a list object of `cap` element cells with initial length
    /// `len`: S1-bump `16 + 24·cap` bytes (the arena-exhaustion check is
    /// the bytes-CAT budget face — fault_cast), write the header, `dst` :=
    /// the object base. The element cells are NOT zeroed — every access
    /// is bounded by the live `len` word.
    pub(crate) fn ctr_list_alloc(&mut self, dst: u8, cap: u64, len: u64) {
        let a0 = self.a0;
        let sc1 = self.asm.sc1;
        let total = CTR_LIST_HDR + CTR_ELEM_BYTES * cap;
        // emit_arena_check clobbers a0/a1 (a1 becomes S1+total), so hold
        // the byte total in sc1 — otherwise the bump below double-adds the
        // arena base and overflows after the second allocation
        self.asm.li(sc1, total);
        self.emit_arena_check(sc1);
        self.asm.mv(dst, S1);
        self.asm.add(S1, S1, sc1);
        self.asm.li(a0, cap);
        self.asm.sd(a0, dst, 8);
        self.asm.li(a0, len);
        self.asm.sd(a0, dst, 0);
    }

    /// `dst` := a one-level deep copy of the list object in `src` — the
    /// interpreter's `CompoItem::copy_with_stats`: a fresh object of the
    /// same capacity, header and every live element cell copied; a nested
    /// container descriptor inside an element cell would be copied BY
    /// VALUE (shared — exactly the interpreter's `Value::clone` on a
    /// `Value::Compo`). Clobbers a0/sc1/sc2 and three value-pool scratch
    /// registers (pushed and popped internally).
    pub(crate) fn ctr_clone_list(&mut self, dst: u8, src: u8) {
        let a0 = self.a0;
        let sc1 = self.asm.sc1;
        let sc2 = self.asm.sc2;
        // total = 16 + 24*cap (cap ≤ CTR_LIST_CAP: the shifts cannot wrap)
        self.asm.ld(sc1, src, 8); // cap
        self.asm.slli_(sc2, sc1, 3); // cap*8
        self.asm.slli_(sc1, sc1, 4); // cap*16
        self.asm.add(sc1, sc1, sc2); // cap*24
        self.asm.addi(sc1, sc1, CTR_LIST_HDR as i32);
        self.emit_arena_check(sc1);
        self.asm.mv(dst, S1);
        self.asm.add(S1, S1, sc1);
        // header: len, cap
        self.asm.ld(sc2, src, 0);
        self.asm.sd(sc2, dst, 0);
        self.asm.ld(sc2, src, 8);
        self.asm.sd(sc2, dst, 8);
        // copy 3*len words of element cells (word loop: one load+store).
        // The walking pointers and the value come from the value pool:
        // ImcRemap maps a1 and sc2 to the same x31, so the a1/sc2 scratch
        // pair cannot hold two distinct live values here.
        let sw = self.push_vreg();
        let dw = self.push_vreg();
        let val = self.push_vreg();
        self.asm.addi(sw, src, CTR_LIST_HDR as i32);
        self.asm.addi(dw, dst, CTR_LIST_HDR as i32);
        self.asm.ld(sc1, src, 0); // len
        self.asm.slli_(sc2, sc1, 1); // 2*len
        self.asm.add(sc1, sc1, sc2); // 3*len = word count
        self.asm.addi(a0, ZERO, 0);
        let l_loop = self.asm.new_label();
        let l_done = self.asm.new_label();
        self.place(l_loop);
        self.asm.bgeu(a0, sc1, l_done);
        self.asm.ld(val, sw, 0);
        self.asm.sd(val, dw, 0);
        self.asm.addi(sw, sw, 8);
        self.asm.addi(dw, dw, 8);
        self.asm.addi(a0, a0, 1);
        self.asm.jal_(ZERO, l_loop);
        self.place(l_done);
        self.pop_vreg();
        self.pop_vreg();
        self.pop_vreg();
    }


    /// Stage `obj[i]`'s key bytes pointer/length into the key scratch cells.
    /// `idx` is a pool register (read-only).
    pub(crate) fn stage_map_entry_key(&mut self, obj: u8, idx: u8) -> Ret<()> {
        let (a0, a1, sc1) = (self.a0, self.a1, self.asm.sc1);
        self.asm.slli_(a0, idx, 5);
        self.asm.slli_(sc1, idx, 3);
        self.asm.add(a0, a0, sc1);
        self.asm.addi(a0, a0, CTR_MAP_HDR as i32);
        self.asm.add(a0, obj, a0);
        self.asm.ld(sc1, a0, 0);
        self.asm.ld(a1, a0, 8);
        self.map_stage_key(sc1, a1);
        Ok(())
    }

    /// Load `obj[i]`'s tagged value words into `va`/`vb` and its tag into
    /// `tag`. `idx` is a pool register (read-only).
    pub(crate) fn load_map_entry_value(
        &mut self,
        obj: u8,
        idx: u8,
        va: u8,
        vb: u8,
        tag: u8,
    ) -> Ret<()> {
        let (a0, sc1) = (self.a0, self.asm.sc1);
        self.asm.slli_(a0, idx, 5);
        self.asm.slli_(sc1, idx, 3);
        self.asm.add(a0, a0, sc1);
        self.asm.addi(a0, a0, CTR_MAP_HDR as i32);
        self.asm.add(a0, obj, a0);
        self.asm.ld(tag, a0, 16);
        self.asm.ld(va, a0, 24);
        self.asm.ld(vb, a0, 32);
        Ok(())
    }

    /* ---- T17: the general list-value ops on the descriptor model ---- *
     * Every helper below runs on an Arena-class receiver (plan() forces
     * the class for take/merge/insert/remove and for the EQ/CHOOSE/bind
     * forms). Register discipline: sc1/sc2 and the mode scratch a0 (plus
     * a1 only where sc2 is provably dead — ImcRemap aliases a1 and sc2);
     * every other temporary rides a fresh pool position above the live
     * operands and is popped before returning.                          */

    /// Move `count_elems` tagged cells starting at element `start` down by
    /// one cell (REMOVE/TAKEFIRST compaction). `start ≥ 1`; `count` may be
    /// zero. Preserves `obj` (sc1) and `count_elems`.
    pub(crate) fn ctr_shift_left_from(&mut self, obj: u8, start_idx: u8, count_elems: u8) {
        let src = self.push_vreg();
        let dst = self.push_vreg();
        let cnt = self.push_vreg();
        // src = obj + 16 + start*24 (element start), dst = one cell back
        self.ctr_elem_addr(src, obj, start_idx, self.a0);
        self.asm.addi(dst, src, -(CTR_ELEM_BYTES as i32));
        // word count = 3 * elements
        self.asm.slli_(cnt, count_elems, 1);
        self.asm.add(cnt, cnt, count_elems);
        let l_loop = self.asm.new_label();
        let l_done = self.asm.new_label();
        self.place(l_loop);
        self.asm.beq(cnt, ZERO, l_done);
        self.asm.ld(self.a0, src, 0);
        self.asm.sd(self.a0, dst, 0);
        self.asm.addi(src, src, 8);
        self.asm.addi(dst, dst, 8);
        self.asm.addi(cnt, cnt, -1);
        self.asm.jal_(ZERO, l_loop);
        self.place(l_done);
        self.pop_vreg();
        self.pop_vreg();
        self.pop_vreg();
    }

    /// Move `count_elems` tagged cells starting at element `idx` UP by one
    /// cell (INSERT shifting). `idx ≤ len`; `count = len - idx` may be
    /// zero. Preserves `obj` (sc1) and `len` (sc2).
    pub(crate) fn ctr_shift_right_from(&mut self, obj: u8, len: u8, count_elems: u8) {
        let a0 = self.a0;
        let src = self.push_vreg();
        let dst = self.push_vreg();
        let cnt = self.push_vreg();
        // src = obj + 16 + len*24 - 8  (last word of the LAST element),
        // dst = src + 24 (the same word one cell up)
        self.asm.slli_(src, len, 3);
        self.asm.slli_(a0, len, 4);
        self.asm.add(src, src, a0); // len*24
        self.asm.addi(src, src, CTR_LIST_HDR as i32 - 8);
        self.asm.add(src, src, obj); // obj + 16 + len*24 - 8
        self.asm.addi(dst, src, CTR_ELEM_BYTES as i32);
        // word count = 3 * elements
        self.asm.slli_(cnt, count_elems, 1);
        self.asm.add(cnt, cnt, count_elems);
        let l_loop = self.asm.new_label();
        let l_done = self.asm.new_label();
        self.place(l_loop);
        self.asm.beq(cnt, ZERO, l_done);
        self.asm.ld(a0, src, 0);
        self.asm.sd(a0, dst, 0);
        self.asm.addi(src, src, -8);
        self.asm.addi(dst, dst, -8);
        self.asm.addi(cnt, cnt, -1);
        self.asm.jal_(ZERO, l_loop);
        self.place(l_done);
        self.pop_vreg();
        self.pop_vreg();
        self.pop_vreg();
    }
}
