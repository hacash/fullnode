//! Mainnet miner upgrade check. Without the `upgrade-guard` feature, [`register`]
//! adds nothing.

use sys::Rerr;

use crate::fullnode::Fullnode;

pub(super) fn register(node: &mut Fullnode) -> Rerr {
    #[cfg(feature = "upgrade-guard")]
    {
        node.add_app(Box::new(enabled::GuardApp::new(
            node.miner_config().enable,
            node.chain_id(),
        )))?;
    }
    #[cfg(not(feature = "upgrade-guard"))]
    {
        let _ = node;
    }
    Ok(())
}

#[cfg(feature = "upgrade-guard")]
mod enabled {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::thread::{self, JoinHandle};
    use std::time::Duration;

    use base::{ApiResponse, BlockSubmitGate, ChainId, ChainListener, PkgOrigin};
    use sys::{Rerr, Waiter};

    use crate::fullnode::{AppHost, NodeApp};

    pub(super) struct GuardApp {
        miner_enabled: bool,
        chain_id: ChainId,
        worker: Option<JoinHandle<()>>,
    }

    impl GuardApp {
        pub(super) fn new(miner_enabled: bool, chain_id: ChainId) -> Self {
            Self {
                miner_enabled,
                chain_id,
                worker: None,
            }
        }
    }

    impl NodeApp for GuardApp {
        fn name(&self) -> &'static str {
            "upgrade_guard"
        }

        fn attach(&mut self, host: &mut AppHost) -> Rerr {
            self.worker = install(self.miner_enabled, self.chain_id, host.waiter(), host);
            Ok(())
        }

        fn join(&mut self) -> Rerr {
            if self
                .worker
                .take()
                .is_some_and(|worker| worker.join().is_err())
            {
                return sys::errf!("upgrade guard worker panicked");
            }
            Ok(())
        }
    }

    const POLL_SECS: u64 = 2 * 60 * 60;
    const RELEASES_LATEST: &str = "https://api.github.com/repos/hacash/fullnode/releases/latest";
    const UPGRADE_NOTICE: &str =
        "Software version is too low. Please upgrade: https://github.com/hacash/fullnode/releases";
    const STALE_SECS: u64 = 35 * 60;

    fn install(
        miner_enabled: bool,
        chain_id: ChainId,
        waiter: Waiter,
        host: &mut AppHost,
    ) -> Option<JoinHandle<()>> {
        if !miner_enabled || !chain_id.is_mainnet() {
            return None;
        }
        let state = Arc::new(GuardState {
            version_low: AtomicBool::new(false),
            arrived: AtomicU64::new(0),
        });
        if let Err(e) = host.add_chain_listener(Arc::new(ArrivalListener {
            state: Arc::clone(&state),
        })) {
            eprintln!("[upgrade] register arrival listener failed: {e}");
            return None;
        }
        host.add_block_submit_gate(upgrade_gate(Arc::clone(&state)));
        Some(thread::spawn(move || {
            loop {
                refresh_version_flag(&state);
                if waiter.sleep_or_quit(Duration::from_secs(POLL_SECS)) {
                    break;
                }
            }
        }))
    }

    struct GuardState {
        version_low: AtomicBool,
        arrived: AtomicU64,
    }

    struct ArrivalListener {
        state: Arc<GuardState>,
    }

    impl ChainListener for ArrivalListener {
        fn on_block_accepted(&self, _height: u64, origin: PkgOrigin) -> Rerr {
            if matches!(origin, PkgOrigin::Rebuild | PkgOrigin::Replay) {
                return Ok(());
            }
            self.state.arrived.store(sys::curtimes(), Ordering::Relaxed);
            Ok(())
        }
    }

    fn upgrade_gate(state: Arc<GuardState>) -> BlockSubmitGate {
        Arc::new(move |ctx| {
            if !submit_is_limited(
                ctx.engine.consensus().chain_id().is_mainnet(),
                state.version_low.load(Ordering::Relaxed),
                state.arrived.load(Ordering::Relaxed),
                sys::curtimes(),
            ) {
                return None;
            }
            print_upgrade_notice();
            Some(ApiResponse::json(format!(
                "{{\"ret\":1,\"err\":{}}}",
                field::json_escape(UPGRADE_NOTICE)
            )))
        })
    }

    fn print_upgrade_notice() {
        let rule = "-".repeat(UPGRADE_NOTICE.len());
        print!("\n\n{rule}\n{UPGRADE_NOTICE}\n{rule}\n\n");
    }

    fn submit_is_limited(is_mainnet: bool, version_low: bool, arrived: u64, now: u64) -> bool {
        if !is_mainnet || !version_low || arrived == 0 {
            return false;
        }
        now.saturating_sub(arrived) < STALE_SECS
    }

    fn refresh_version_flag(state: &GuardState) {
        let tag = match fetch_latest_tag() {
            Ok(tag) => tag,
            Err(e) => {
                eprintln!("[upgrade] github release check failed: {e}");
                return;
            }
        };
        match version_is_behind(crate::HACASH_NODE_VERSION, &tag) {
            Some(behind) => {
                state.version_low.store(behind, Ordering::Relaxed);
                println!(
                    "[upgrade] local={} latest={} outdated={}",
                    crate::HACASH_NODE_VERSION,
                    tag,
                    behind
                );
            }
            None => eprintln!("[upgrade] ignore unparsed release tag {tag}"),
        }
    }

    fn fetch_latest_tag() -> Result<String, String> {
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(20))
            .user_agent("hacash-fullnode")
            .build()
            .map_err(|e| e.to_string())?;
        let text = client
            .get(RELEASES_LATEST)
            .header("accept", "application/vnd.github+json")
            .send()
            .map_err(|e| e.to_string())?
            .error_for_status()
            .map_err(|e| e.to_string())?
            .text()
            .map_err(|e| e.to_string())?;
        tag_name_from_release_json(&text).ok_or_else(|| "release tag missing".to_owned())
    }

    fn tag_name_from_release_json(text: &str) -> Option<String> {
        let pairs = field::json_split_object(text).ok()?;
        let raw = pairs.iter().find(|(key, _)| *key == "tag_name")?.1;
        field::json_expect_quoted_decoded(raw).ok()
    }

    /// `Some(true)` when the local major is lower, or the majors match and the
    /// local minor is lower. A higher local major is not behind, even if its minor
    /// is smaller. `None` when either side is not `major.minor`.
    fn version_is_behind(local: &str, remote: &str) -> Option<bool> {
        let local = parse_major_minor(local)?;
        let remote = parse_major_minor(remote)?;
        Some(if local.0 != remote.0 {
            local.0 < remote.0
        } else {
            local.1 < remote.1
        })
    }

    fn parse_major_minor(raw: &str) -> Option<(u64, u64)> {
        let text = raw.trim().trim_start_matches(['v', 'V']);
        let (major, rest) = split_component(text)?;
        let rest = rest.strip_prefix('.')?;
        let (minor, _) = split_component(rest)?;
        Some((major, minor))
    }

    fn split_component(text: &str) -> Option<(u64, &str)> {
        let end = text
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(text.len());
        if end == 0 {
            return None;
        }
        Some((text[..end].parse().ok()?, &text[end..]))
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn major_is_compared_before_minor() {
            assert_eq!(version_is_behind("2.0.0", "1.9.0"), Some(false));
            assert_eq!(version_is_behind("1.2.5", "1.3.0"), Some(true));
            assert_eq!(version_is_behind("1.2.5", "1.2.9"), Some(false));
            assert_eq!(version_is_behind("1.2.5", "2.0.0"), Some(true));
            assert_eq!(version_is_behind("1.3.0", "1.2.4"), Some(false));
            assert_eq!(version_is_behind("1.2.5", "v1.4.0"), Some(true));
            assert_eq!(version_is_behind("1.2.5", "nightly"), None);
        }

        #[test]
        fn release_json_reads_tag_name() {
            let body = r#"{"tag_name":"1.3.0","name":"1.3.0"}"#;
            assert_eq!(tag_name_from_release_json(body).as_deref(), Some("1.3.0"));
            assert_eq!(tag_name_from_release_json("{}"), None);
        }

        #[test]
        fn submit_window_uses_arrival_age() {
            let fresh = 1_000 + STALE_SECS - 1;
            let stale = 1_000 + STALE_SECS;
            assert!(!submit_is_limited(false, true, 1_000, fresh));
            assert!(!submit_is_limited(true, false, 1_000, fresh));
            assert!(!submit_is_limited(true, true, 0, fresh));
            assert!(submit_is_limited(true, true, 1_000, fresh));
            assert!(!submit_is_limited(true, true, 1_000, stale));
        }
    }
}
