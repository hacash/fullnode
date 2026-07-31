//! fullnodedev-compatible `stable.nodes` cache helpers.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

const STABLE_EXPIRE_SECS: u64 = 24 * 60 * 60;

fn stable_path(data_dir: &str) -> PathBuf {
    Path::new(data_dir).join("stable.nodes")
}

fn expired(path: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    let Ok(modified) = meta.modified() else {
        return false;
    };
    let Ok(elapsed) = SystemTime::now().duration_since(modified) else {
        return false;
    };
    elapsed.as_secs() >= STABLE_EXPIRE_SECS
}

pub fn read_stable_file(data_dir: &str, max: usize) -> Vec<SocketAddr> {
    if max == 0 || data_dir.is_empty() {
        return vec![];
    }
    let path = stable_path(data_dir);
    if expired(&path) {
        let _ = std::fs::remove_file(&path);
        return vec![];
    }
    let Ok(content) = std::fs::read_to_string(&path) else {
        return vec![];
    };
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(addr) = line.parse::<SocketAddr>() else {
            continue;
        };
        if addr.ip().is_loopback() {
            continue;
        }
        if seen.insert(addr) {
            out.push(addr);
            if out.len() >= max {
                break;
            }
        }
    }
    out
}

pub fn write_stable_file(data_dir: &str, addrs: &[SocketAddr]) {
    if data_dir.is_empty() {
        return;
    }
    let path = stable_path(data_dir);
    let mut out = String::new();
    for a in addrs {
        out.push_str(&format!("{}\n", a));
    }
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let mut tmp = path.clone();
    tmp.set_extension("nodes.tmp");
    if std::fs::write(&tmp, out).is_ok() {
        let _ = std::fs::rename(&tmp, &path);
    } else {
        let _ = std::fs::remove_file(&tmp);
    }
}
