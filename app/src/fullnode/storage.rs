//! Application-owned persistent storage layout: `db` opens supplied directories
//! only; this module owns the versioned chain-state paths.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use base::Store;

struct StorageDirs {
    block: PathBuf,
    state: PathBuf,
    log: PathBuf,
}

pub(super) fn open(data_dir: &str) -> sys::Ret<Arc<dyn Store>> {
    if data_dir.is_empty() {
        return Ok(Arc::new(db::StoreInst::new()));
    }
    let dirs = prepare(Path::new(data_dir))?;
    let store = Arc::new(db::StoreInst::open(&dirs.block, &dirs.state, &dirs.log)?);
    Ok(store)
}

fn prepare(data_dir: &Path) -> sys::Ret<StorageDirs> {
    std::fs::create_dir_all(data_dir)
        .map_err(|e| sys::Error::fault(format!("create data dir {}: {e}", data_dir.display())))?;

    // Detect a prior `state_v{M}` directory. When the compiled `DB_VERSION` differs from
    // the on-disk one, rename the old dirs aside (for ops to delete) instead of deleting them.
    let found_version = scan_state_version(data_dir)?;
    let needs_state_migration = match found_version {
        None => false, // first run: no prior state to migrate
        Some(v) => v != crate::DB_VERSION,
    };

    if needs_state_migration {
        let ts = timestamp_suffix()?;
        if let Some(v) = found_version {
            let old = data_dir.join(format!("state_v{v}"));
            if old.is_dir() {
                let new = data_dir.join(format!("state_v{v}_bnk_{ts}"));
                rename_dir(&old, &new)?;
                println!(
                    "[app] storage migration: renamed old state to {}",
                    new.display()
                );
            }
        }
        let old_log = data_dir.join("vmlog");
        if old_log.is_dir() {
            let new_log = data_dir.join(format!("vmlog_bnk_{ts}"));
            rename_dir(&old_log, &new_log)?;
            println!(
                "[app] storage migration: renamed old vmlog to {}",
                new_log.display()
            );
        }
    }

    let dirs = StorageDirs {
        block: data_dir.join("block"),
        state: data_dir.join(format!("state_v{}", crate::DB_VERSION)),
        log: data_dir.join("vmlog"),
    };
    for dir in [&dirs.block, &dirs.state, &dirs.log] {
        std::fs::create_dir_all(dir)
            .map_err(|e| sys::Error::fault(format!("create storage dir {}: {e}", dir.display())))?;
    }
    println!(
        "[app] storage block={} state={} vmlog={}",
        dirs.block.display(),
        dirs.state.display(),
        dirs.log.display()
    );
    Ok(dirs)
}

fn rename_dir(from: &Path, to: &Path) -> sys::Rerr {
    std::fs::rename(from, to).map_err(|e| {
        sys::Error::fault(format!(
            "rename {} to {}: {}",
            from.display(),
            to.display(),
            e
        ))
    })
}

/// Find the current state_v{N} directory (excluding renamed `_bnk_` dirs).
/// Returns `None` if no state dir exists (first run).
fn scan_state_version(data_dir: &Path) -> sys::Ret<Option<u32>> {
    for entry in std::fs::read_dir(data_dir)
        .map_err(|e| sys::Error::fault(format!("read data dir {}: {e}", data_dir.display())))?
        .flatten()
    {
        let name = match entry.file_name().into_string() {
            Ok(s) => s,
            Err(_) => continue,
        };
        let Some(rest) = name.strip_prefix("state_v") else {
            continue;
        };
        if rest.contains("_bnk_") {
            continue; // skip renamed backup dirs
        }
        if let Ok(v) = rest.parse::<u32>() {
            return Ok(Some(v));
        }
    }
    Ok(None)
}

/// Format the current wall clock as a UTC `YYYYMMDDHHMMSS` timestamp suffix
/// (Howard Hinnant's `civil_from_days` proleptic Gregorian conversion, <https://howardhinnant.github.io/date_algorithms.html>).
fn timestamp_suffix() -> sys::Ret<String> {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| sys::Error::fault(format!("system clock pre-epoch: {e}")))?
        .as_secs();
    let days = (secs / 86400) as i64;
    let rem = (secs % 86400) as u64;
    let hour = rem / 3600;
    let minute = (rem % 3600) / 60;
    let second = rem % 60;

    // civil_from_days: days since 1970-01-01 -> (year, month, day), UTC.
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    let year = if m <= 2 { y + 1 } else { y };

    Ok(format!(
        "{:04}{:02}{:02}{:02}{:02}{:02}",
        year, m, d, hour, minute, second
    ))
}
