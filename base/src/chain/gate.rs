//! Read sessions for state queries.
//!
//! A session bundles a state tip with the ownership needed to keep its weak
//! parent chain alive. Critical packing sessions additionally hold the root
//! movement read lock for their full lifetime.

use std::sync::Arc;
use std::sync::RwLockReadGuard;

use field::Hash;

use crate::state::StateRead;
use crate::{Env, ExecutionServices, StateChunkRef, TxRef};

/// Root-stable session for miner packing. The root-move read lock is held for
/// the session lifetime; callers still validate `epoch()` because ordinary
/// head attachment remains concurrent.
///
/// Fields are private so callers cannot detach the tip from the guard. Access
/// via `view()`, `head_hash()`, and `head_height()`.
pub struct StateReadSession<'a> {
    view: StateChunkRef,
    head_hash: Hash,
    head_height: u64,
    epoch: u64,
    _hold: sys::HoldGuard,
    _root_move: RwLockReadGuard<'a, ()>,
}

impl<'a> StateReadSession<'a> {
    /// Engine-private constructor exposed for the engine crate only.
    /// External callers obtain a session via `ChainView::state_canonical`
    /// only - they cannot construct one directly and detach the state tip from
    /// its guard. Kept `pub` because the engine lives in a
    /// sibling crate; the `state_canonical` indirection is what enforces the
    /// engine-private contract.
    pub fn new(
        view: StateChunkRef,
        head_hash: Hash,
        head_height: u64,
        epoch: u64,
        hold: sys::HoldGuard,
        root_move: RwLockReadGuard<'a, ()>,
    ) -> Self {
        Self {
            view,
            head_hash,
            head_height,
            epoch,
            _hold: hold,
            _root_move: root_move,
        }
    }

    pub fn view(&self) -> &dyn StateRead {
        self.view.as_ref()
    }
    pub fn head_hash(&self) -> Hash {
        self.head_hash
    }
    pub fn head_height(&self) -> u64 {
        self.head_height
    }
    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    /// Start cumulative transaction execution tied to this session.  The
    /// returned executor borrows `self` and never exposes the owned tip.
    pub fn begin_execution<'s>(&'s self) -> StateExecSession<'s, 'a> {
        StateExecSession {
            root: StateChunkRef::block_draft_on(&self.view, self.head_height.saturating_add(1)),
            _session: self,
        }
    }
}

impl<'a> std::fmt::Debug for StateReadSession<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StateReadSession")
            .field("head_height", &self.head_height)
            .finish_non_exhaustive()
    }
}

/// Cumulative execution layer whose lifetime is bounded by a state session.
/// Candidate transactions commit into `root` only after successful
/// execution; failed candidates are discarded with their child context.
pub struct StateExecSession<'s, 'g> {
    root: StateChunkRef,
    _session: &'s StateReadSession<'g>,
}

impl StateExecSession<'_, '_> {
    pub fn execute_tx(
        &mut self,
        services: Arc<dyn ExecutionServices>,
        env: Env,
        tx: TxRef,
    ) -> sys::Rerr {
        let child = self.root.spawn_tx_child(tx.hash())?;
        let mut ctx = services.create_context(env, child, tx.clone())?;
        tx.execute(ctx.as_mut())?;
        let child = ctx.release_chunk()?;
        let parent = child.commit_to_parent()?;
        debug_assert!(parent.ptr_eq(&self.root));
        Ok(())
    }
}

/// An optimistic session at a specific branch tip, for external consumers
/// such as indexers. The root pin preserves the weak parent chain; callers
/// must validate tip membership after finishing the read.
///
/// Fields are private so callers cannot detach the tip from its root pin.
pub struct StateSnapSession<'a> {
    view: StateChunkRef,
    _root_pin: StateChunkRef,
    tip_hash: Hash,
    tip_height: u64,
    _hold: sys::HoldGuard,
    _marker: std::marker::PhantomData<&'a ()>,
}

impl<'a> StateSnapSession<'a> {
    /// Engine-private constructor.  External callers obtain a session via
    /// `ChainView::state_at_session` only - they cannot construct one
    /// directly (§5.3).
    pub fn new(
        view: StateChunkRef,
        root_pin: StateChunkRef,
        tip_hash: Hash,
        tip_height: u64,
        hold: sys::HoldGuard,
    ) -> Self {
        Self {
            view,
            _root_pin: root_pin,
            tip_hash,
            tip_height,
            _hold: hold,
            _marker: std::marker::PhantomData,
        }
    }

    pub fn view(&self) -> &dyn StateRead {
        &self.view
    }

    pub fn tip_hash(&self) -> Hash {
        self.tip_hash
    }

    pub fn tip_height(&self) -> u64 {
        self.tip_height
    }
}

impl<'a> std::fmt::Debug for StateSnapSession<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StateSnapSession")
            .field("tip_height", &self.tip_height)
            .finish_non_exhaustive()
    }
}

/// Optimistic snapshot of chain state, captured under only the short Tree
/// lock. A root pin keeps its weak parent chain alive after capture.
///
/// Optimistic readers (§6) capture head + view + start epoch in one step,
/// do one bounded computation, then validate via `ChainView::validate_optimistic`.
///
/// Because no lock is held, the result may be discarded if a writer committed
/// during the window.  This is the accepted cost for not blocking writers.
pub struct OptimisticState {
    view: StateChunkRef,
    pub head_hash: Hash,
    pub head_height: u64,
    pub epoch: u64,
    _root_pin: StateChunkRef,
    _hold: sys::HoldGuard,
}

impl OptimisticState {
    pub fn new(
        view: StateChunkRef,
        root_pin: StateChunkRef,
        head_hash: Hash,
        head_height: u64,
        epoch: u64,
        hold: sys::HoldGuard,
    ) -> Self {
        Self {
            view,
            _root_pin: root_pin,
            head_hash,
            head_height,
            epoch,
            _hold: hold,
        }
    }

    pub fn view(&self) -> &dyn StateRead {
        &self.view
    }

    /// Start best-effort transaction execution. The returned chunk must not
    /// outlive this snapshot, which owns its parent tip and root pin.
    pub fn begin_tx(&self, tx_hash: Hash) -> StateChunkRef {
        StateChunkRef::tx_on(&self.view, tx_hash)
    }

    /// Start cumulative draft execution tied to this pinned snapshot.
    pub fn begin_block_draft(&self, height: u64) -> StateChunkRef {
        StateChunkRef::block_draft_on(&self.view, height)
    }
}

impl std::fmt::Debug for OptimisticState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OptimisticState")
            .field("head_height", &self.head_height)
            .field("epoch", &self.epoch)
            .finish_non_exhaustive()
    }
}
