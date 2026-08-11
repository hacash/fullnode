//! Side hash list: the peripheral persistence hint for side branch bodies.
//!
//! Side bodies are stored content-addressed in the block DB; this file only
//! records their hashes (fixed 32-byte records) so boot can rebuild the
//! volatile side tree. It is not a canonical authority: appends are
//! best-effort and asynchronous, the file is read only at boot, and the boot
//! side replay treats the whole file as discardable.
//!
//! A single writer thread owns append / compaction / clear ordering. A
//! compaction rewrites the file through a temporary file + atomic rename,
//! merging hashes that arrived meanwhile (the channel is drained first), so
//! no recovery hint is lost while the file is replaced.

use std::collections::HashSet;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};

use field::Hash;
use sys::Ret;

/// One compaction per this many appended hashes. Compaction removes records
/// that are already canonical or below the durable root so the file cannot
/// grow without bound and boot reads stay cheap.
const COMPACT_INTERVAL: usize = 2048;

/// Builds the per-compaction keep predicate. Returned closure answers "should
/// this side hash stay in the list?" — it drops canonical hashes, hashes
/// whose body is gone or undecodable, and history below the durable root.
pub type SideKeepCtx = Arc<dyn Fn() -> Box<dyn Fn(&Hash) -> bool> + Send + Sync>;

pub(crate) enum SideMsg {
    Append(Vec<Hash>),
}

/// Best-effort appender handle held by the engine.
pub struct SideListWriter {
    tx: SyncSender<SideMsg>,
}

impl SideListWriter {
    /// Create the appender without starting the writer thread. The receiver
    /// is handed to `spawn`; engine boot owns the timing, so a boot failure
    /// cannot leak the writer thread and boot's direct file reads stay
    /// race-free.
    pub fn new() -> (Arc<Self>, Receiver<SideMsg>) {
        let (tx, rx) = sync_channel::<SideMsg>(64);
        (Arc::new(Self { tx }), rx)
    }

    /// Start the single writer thread. `path = None` disables persistence (no
    /// data dir); the writer thread still runs so appends are dropped silently
    /// instead of erroring.
    pub fn spawn(
        self: &Arc<Self>,
        path: Option<PathBuf>,
        keep_ctx: SideKeepCtx,
        cancel: Arc<AtomicBool>,
        rx: Receiver<SideMsg>,
    ) -> Ret<std::thread::JoinHandle<()>> {
        std::thread::Builder::new()
            .name("side-hash-list".to_owned())
            .spawn(move || writer_loop(path, rx, keep_ctx, cancel))
            .map_err(|e| sys::Error::fault(format!("cannot spawn side hash list writer: {}", e)))
    }

    /// Best-effort append. A saturated writer only warns: the recovery hint
    /// is peripheral and may be lost without affecting canonical safety.
    pub fn append(&self, hash: Hash) {
        if self.tx.try_send(SideMsg::Append(vec![hash])).is_err() {
            eprintln!(
                "[Engine] side hash list writer is saturated; a side recovery hint was dropped"
            );
        }
    }

    pub fn append_many(&self, hashes: Vec<Hash>) {
        if hashes.is_empty() {
            return;
        }
        if self.tx.try_send(SideMsg::Append(hashes)).is_err() {
            eprintln!(
                "[Engine] side hash list writer is saturated; side recovery hints were dropped"
            );
        }
    }
}

fn writer_loop(
    path: Option<PathBuf>,
    rx: Receiver<SideMsg>,
    keep_ctx: SideKeepCtx,
    cancel: Arc<AtomicBool>,
) {
    let mut appended = 0usize;
    loop {
        // Normal operation blocks briefly for the next append; once cancelled
        // the channel is drained so a normal shutdown keeps every recorded
        // hint, while an extreme crash may lose the last window.
        let msg = if cancel.load(Ordering::Acquire) {
            match rx.try_recv() {
                Ok(msg) => Some(msg),
                Err(std::sync::mpsc::TryRecvError::Empty)
                | Err(std::sync::mpsc::TryRecvError::Disconnected) => break,
            }
        } else {
            match rx.recv_timeout(std::time::Duration::from_millis(200)) {
                Ok(msg) => Some(msg),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => None,
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            }
        };
        let Some(SideMsg::Append(hashes)) = msg else {
            continue;
        };
        let Some(path) = path.as_deref() else {
            continue;
        };
        if let Err(e) = append_hashes(path, &hashes) {
            eprintln!("[Engine] side hash list append failed: {}", e);
        }
        appended += hashes.len();
        if appended >= COMPACT_INTERVAL {
            appended = 0;
            compact(path, &keep_ctx);
        }
    }
}

fn append_hashes(path: &Path, hashes: &[Hash]) -> std::io::Result<()> {
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    for hash in hashes {
        file.write_all(&hash.0)?;
    }
    file.flush()
}

fn compact(path: &Path, keep_ctx: &SideKeepCtx) {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) => {
            eprintln!(
                "[Engine] side hash list unreadable during compaction: {}",
                e
            );
            return;
        }
    };
    if bytes.len() % 32 != 0 {
        eprintln!("[Engine] side hash list has partial records; clearing it");
        let _ = std::fs::write(path, &[] as &[u8]);
        return;
    }
    let keep = (keep_ctx)();
    let mut kept = Vec::with_capacity(bytes.len());
    let mut seen = HashSet::new();
    for chunk in bytes.chunks_exact(32) {
        let mut buf = [0u8; 32];
        buf.copy_from_slice(chunk);
        let hash = Hash::from(buf);
        if seen.insert(hash) && keep(&hash) {
            kept.extend_from_slice(chunk);
        }
    }
    // Atomic replacement in the same directory; pending appends were drained
    // before the rewrite and are appended to the new file afterwards.
    let tmp = path.with_extension("tmp");
    if let Err(e) = std::fs::write(&tmp, &kept) {
        eprintln!("[Engine] side hash list compaction write failed: {}", e);
        let _ = std::fs::remove_file(&tmp);
        return;
    }
    if let Err(e) = std::fs::rename(&tmp, path) {
        eprintln!("[Engine] side hash list compaction rename failed: {}", e);
        let _ = std::fs::remove_file(&tmp);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU64;

    fn hash(value: u8) -> Hash {
        Hash::from([value; 32])
    }

    static DIR_SEQ: AtomicU64 = AtomicU64::new(0);

    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> (Self, PathBuf) {
            let dir = std::env::temp_dir().join(format!(
                "side_list_test_{}_{}_{}",
                std::process::id(),
                DIR_SEQ.fetch_add(1, Ordering::Relaxed),
                sys::curtimes()
            ));
            std::fs::create_dir_all(&dir).unwrap();
            (TempDir(dir.clone()), dir.join("side_hash_list"))
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn compaction_drops_canonical_and_below_root_hashes() {
        let (_dir, path) = TempDir::new();
        let hashes: Vec<Hash> = (1..=6).map(hash).collect();
        append_hashes(&path, &hashes).unwrap();

        // Keep only heights above root 2 and not canonical: hash(4), hash(5),
        // hash(6).
        let keep: SideKeepCtx = Arc::new(move || {
            let canonical: HashSet<Hash> = [hash(1), hash(2), hash(3)].into_iter().collect();
            let root_height = 2u64;
            let body = move |h: &Hash| {
                if canonical.contains(h) {
                    return false;
                }
                h.0[0] > root_height as u8
            };
            Box::new(body)
        });
        compact(&path, &keep);
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(bytes.len(), 96);
        assert_eq!(bytes[0], 4);
        assert_eq!(bytes[32], 5);
        assert_eq!(bytes[64], 6);
    }

    #[test]
    fn partial_records_are_cleared_not_preserved() {
        let (_dir, path) = TempDir::new();
        std::fs::write(&path, [7u8; 40]).unwrap();
        let keep: SideKeepCtx = Arc::new(|| Box::new(|_| true));
        compact(&path, &keep);
        assert_eq!(std::fs::read(&path).unwrap().len(), 0);
    }

    #[test]
    fn writer_drains_queued_appends_on_cancel() {
        let (_dir, path) = TempDir::new();
        let cancel = Arc::new(AtomicBool::new(false));
        let (writer, rx) = SideListWriter::new();
        let handle = writer
            .spawn(
                Some(path.clone()),
                Arc::new(|| Box::new(|_| true)),
                cancel.clone(),
                rx,
            )
            .unwrap();
        writer.append(hash(1));
        writer.append(hash(2));
        cancel.store(true, Ordering::Release);
        handle.join().unwrap();
        assert_eq!(std::fs::read(&path).unwrap().len(), 64);
    }
}
