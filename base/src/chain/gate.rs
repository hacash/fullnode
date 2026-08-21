//! Read sessions for state queries: a session pins the tree root captured with
//! its state tip, so the subtree survives root rolls without holding any lock.

use std::sync::Arc;

use field::Hash;

use crate::state::StateRead;
use crate::{Env, ExecutionServices, StateChunkRef, TxRef};

/// Root-pinned session for miner packing: the pin keeps the tip's weak parent chain
/// alive across root rolls; callers still validate `epoch()`. Fields are private.
pub struct StateReadSession {
    view: StateChunkRef,
    _root_pin: StateChunkRef,
    head_hash: Hash,
    head_height: u64,
    epoch: u64,
    _hold: sys::HoldGuard,
}

impl StateReadSession {
    /// Engine-private constructor; external callers obtain a session via
    /// `ChainView::state_canonical` only — they cannot detach the tip from its pin.
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
    pub fn begin_execution<'s>(&'s self) -> StateExecSession<'s> {
        StateExecSession {
            root: StateChunkRef::block_draft_on(&self.view, self.head_height.saturating_add(1)),
            _session: self,
        }
    }
}

impl std::fmt::Debug for StateReadSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StateReadSession")
            .field("head_height", &self.head_height)
            .finish_non_exhaustive()
    }
}

/// Cumulative execution layer bounded by a state session: candidates commit into
/// `root` only after successful execution; failed candidates are discarded.
pub struct StateExecSession<'s> {
    root: StateChunkRef,
    _session: &'s StateReadSession,
}

impl StateExecSession<'_> {
    /// Execution entry: only exists in full builds (codec-only builds have no
    /// callable `TransactionExecute` surface).
    #[cfg(feature = "execute")]
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

/// Optimistic session at a specific branch tip for external consumers (indexers);
/// callers must validate tip membership after the read. Fields are private.
pub struct StateSnapSession<'a> {
    view: StateChunkRef,
    _root_pin: StateChunkRef,
    tip_hash: Hash,
    tip_height: u64,
    _hold: Option<sys::HoldGuard>,
    _marker: std::marker::PhantomData<&'a ()>,
}

impl<'a> StateSnapSession<'a> {
    /// Engine-private constructor; external callers obtain a session via
    /// `ChainView::state_at_session` only (§5.3).
    pub fn new(
        view: StateChunkRef,
        root_pin: StateChunkRef,
        tip_hash: Hash,
        tip_height: u64,
        hold: Option<sys::HoldGuard>,
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

/// Optimistic chain-state snapshot under only the short Tree lock; a root pin keeps
/// its weak parent chain alive. May be discarded if a writer commits during the window (accepted cost of not blocking writers); readers validate via `ChainView::validate_optimistic`.
pub struct OptimisticState {
    view: StateChunkRef,
    pub head_hash: Hash,
    pub head_height: u64,
    pub epoch: u64,
    _root_pin: StateChunkRef,
    _hold: Option<sys::HoldGuard>,
}

impl OptimisticState {
    pub fn new(
        view: StateChunkRef,
        root_pin: StateChunkRef,
        head_hash: Hash,
        head_height: u64,
        epoch: u64,
        hold: Option<sys::HoldGuard>,
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
