//! TCP listener, dialer, session loop, and P2P maintenance.
//!
//! Dual-protocol: accepts both v1 (legacy mainnet) and v2 (new) peers.
//! Dialing tries v2 first, then reconnects with v1 when needed.
//! Each session reads frames in the negotiated version and dispatches
//! application messages to `handle_message` (shared, version-agnostic).

use std::collections::HashSet;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::{Notify, mpsc};

use sys::{Rerr, ToHex, Waiter};

use crate::P2PNode;
use crate::msgqueue::InboundMsg;
use crate::p2p::codec::{
    accept_read_magic, dial_magic_exchange, read_transport_msg as v2_read_msg, write_v2_magic,
};
use crate::p2p::handshake::{
    HANDSHAKE_TIMEOUT, PeerIdentity, VersionMessage, exchange_handshake, peer_id_from_key,
};
use crate::p2p::legacy::{
    self, build_node_info, node_info_key_name, parse_answer_peer, parse_report_peer,
};
use crate::p2p::msg::{
    MSG_BLOCK_DISCOVER, MSG_TX_SUBMIT, P2P_MAGIC_V1, V1_MSG_ANSWER_PEER, V1_MSG_CLOSE,
    V1_MSG_CUSTOMER, V1_MSG_PING, V1_MSG_PONG, V1_MSG_REMIND_ME_IS_PUBLIC, V1_MSG_REPORT_PEER,
    V1_MSG_REQUEST_NEAREST_PUBLIC_NODES, V1_MSG_REQUEST_NODE_KEY_FOR_PUBLIC_CHECK, services,
    v2 as v2msg,
};
use crate::p2p::peer::{
    CTRL_WRITER_CAPACITY, PeerWriteCmd, ProtocolVersion, RemotePeer, TX_WRITER_CAPACITY,
    spawn_writer,
};
use crate::publiccheck::{
    maybe_mark_public_from_report, maybe_mark_public_from_version, public_from_answer,
    serve_check_public_v2,
};

const P2P_STATUS_PRINT_INTERVAL_SECS: u64 = 60 * 97;

fn format_p2p_status(
    public_count: usize,
    subnet_count: usize,
    node_key: &[u8; 16],
    names: &[String],
) -> String {
    format!(
        "[P2P] {} public {} subnet nodes connected, key({}) => {}.\n",
        public_count,
        subnet_count,
        node_key[..2].to_hex(),
        names.join(", ")
    )
}

impl P2PNode {
    pub fn start_p2p_on(self: Arc<Self>, handle: &tokio::runtime::Handle, waiter: Waiter) -> Rerr {
        // Always start inbound queue (API submit waits on it even if P2P is idle).
        self.ensure_msg_handler(waiter.clone());
        self.start_held_replay_worker(waiter.clone());
        if self.config.listen_port == 0
            && self.config.boot_nodes.is_empty()
            && !self.config.use_stable_nodes
        {
            println!("[P2P] no listen/boots/stable; P2P idle");
            return Ok(());
        }
        let listener = self.bind_p2p_listener_on(handle)?;
        let this = self;
        handle.spawn(async move {
            if let Err(e) = this.run_p2p_with_listener(waiter, listener).await {
                eprintln!("[P2P] runtime exited: {}", e);
            }
        });
        Ok(())
    }

    pub fn start_p2p(self: Arc<Self>, waiter: Waiter) -> Rerr {
        let this = self.clone();
        std::thread::Builder::new()
            .name("hacash-p2p".into())
            .spawn(move || {
                if let Err(e) = this.run_p2p(waiter) {
                    eprintln!("[P2P] runtime exited: {}", e);
                }
            })
            .map_err(|e| sys::Error::fault(format!("spawn p2p thread failed: {}", e)))?;
        Ok(())
    }

    pub fn run_p2p(self: Arc<Self>, waiter: Waiter) -> Rerr {
        self.ensure_msg_handler(waiter.clone());
        self.start_held_replay_worker(waiter.clone());
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .map_err(|e| sys::Error::fault(format!("tokio runtime build failed: {}", e)))?;
        rt.block_on(async move { self.run_p2p_async(waiter).await })
    }

    pub fn run_listener(self: Arc<Self>, waiter: Waiter) -> Rerr {
        self.run_p2p(waiter)
    }

    /// Dial v2 first, then always retry with v1 after a failed v2 attempt.
    pub async fn connect_addr(self: &Arc<Self>, addr: SocketAddr) -> Rerr {
        match self.connect_addr_v2(addr).await {
            Ok(()) => return Ok(()),
            Err(_) => {
                // Fall through to a fresh v1 TCP connection. A v1 peer may
                // already have closed the v2-magic probe connection.
            }
        }
        self.connect_addr_v1(addr).await
    }

    /// v2 dial: connect, exchange v2 magic, run v2 handshake + session.
    async fn connect_addr_v2(self: &Arc<Self>, addr: SocketAddr) -> Rerr {
        let stream =
            tokio::time::timeout(Duration::from_secs(6), tokio::net::TcpStream::connect(addr))
                .await
                .map_err(|_| sys::Error::fault(format!("v2 connect timeout {}", addr)))?
                .map_err(|e| sys::Error::fault(format!("v2 connect {}: {}", addr, e)))?;
        let mut stream = stream;
        // Lead with v2 magic; inspect peer reply.
        let is_v2 = tokio::time::timeout(HANDSHAKE_TIMEOUT, dial_magic_exchange(&mut stream))
            .await
            .map_err(|_| sys::Error::fault("v2 magic handshake timeout".to_owned()))??;
        if !is_v2 {
            // Peer is v1 - caller (connect_addr) will reconnect with v1.
            return sys::errf!("peer {} is v1, fallback needed", addr);
        }
        self.clone().run_v2_session(stream, addr, true).await
    }

    /// v1 dial: connect, exchange v1 magic, run v1 handshake + session.
    async fn connect_addr_v1(self: &Arc<Self>, addr: SocketAddr) -> Rerr {
        let stream =
            tokio::time::timeout(Duration::from_secs(6), tokio::net::TcpStream::connect(addr))
                .await
                .map_err(|_| sys::Error::fault(format!("v1 connect timeout {}", addr)))?
                .map_err(|e| sys::Error::fault(format!("v1 connect {}: {}", addr, e)))?;
        handle_conn_v1(self.clone(), stream, addr, true).await
    }

    pub async fn run_p2p_async(self: Arc<Self>, waiter: Waiter) -> Rerr {
        let listener = self.bind_p2p_listener_async().await?;
        self.run_p2p_with_listener(waiter, listener).await
    }

    fn p2p_socket_addr(&self) -> sys::Ret<Option<SocketAddr>> {
        if !self.config.accept_nodes || self.config.listen_port == 0 {
            return Ok(None);
        }
        Ok(Some(SocketAddr::new(
            self.config.listen_ip,
            self.config.listen_port,
        )))
    }

    fn bind_p2p_listener_on(
        &self,
        handle: &tokio::runtime::Handle,
    ) -> sys::Ret<Option<tokio::net::TcpListener>> {
        let Some(addr) = self.p2p_socket_addr()? else {
            return Ok(None);
        };
        let listener = std::net::TcpListener::bind(addr)
            .map_err(|e| sys::Error::fault(format!("p2p bind {} failed: {}", addr, e)))?;
        listener.set_nonblocking(true).map_err(|e| {
            sys::Error::fault(format!("p2p set nonblocking {} failed: {}", addr, e))
        })?;
        let _runtime = handle.enter();
        let listener = tokio::net::TcpListener::from_std(listener)
            .map_err(|e| sys::Error::fault(format!("p2p adopt listener {} failed: {}", addr, e)))?;
        self.report_p2p_listener(addr);
        Ok(Some(listener))
    }

    async fn bind_p2p_listener_async(&self) -> sys::Ret<Option<tokio::net::TcpListener>> {
        let Some(addr) = self.p2p_socket_addr()? else {
            return Ok(None);
        };
        let listener = tokio::net::TcpListener::bind(addr)
            .await
            .map_err(|e| sys::Error::fault(format!("p2p bind {} failed: {}", addr, e)))?;
        self.report_p2p_listener(addr);
        Ok(Some(listener))
    }

    fn report_p2p_listener(&self, addr: SocketAddr) {
        println!("[P2P] listening on {} (v1+v2)", addr);
    }

    async fn run_p2p_with_listener(
        self: Arc<Self>,
        waiter: Waiter,
        listen_socket: Option<tokio::net::TcpListener>,
    ) -> Rerr {
        let mut seed_addrs = parse_boot_addrs(&self.config.boot_nodes);
        if self.config.use_stable_nodes {
            self.addrbook.load_from_stable().await;
        }
        for a in &seed_addrs {
            let _ = self
                .addrbook
                .insert(None, *a, crate::addrbook::AddrSource::Boot)
                .await;
        }
        // Startup dial set: boots + nearest book candidates (capped).
        let book_seeds = self
            .addrbook
            .dial_candidates(
                &seed_addrs,
                &[],
                self.config
                    .addrbook_dial_max
                    .max(self.config.backbone_peers),
            )
            .await;
        for a in book_seeds {
            if !seed_addrs.contains(&a) {
                seed_addrs.push(a);
            }
        }

        // Startup: brief delay then seed dial (mainnet ~0.25s).
        tokio::time::sleep(Duration::from_millis(250)).await;
        self.dial_seeds(&seed_addrs).await;

        let mut check_tick = tokio::time::interval(Duration::from_secs(159));
        let mut boost_tick = tokio::time::interval(Duration::from_secs(270));
        let mut find_tick = tokio::time::interval(Duration::from_secs(52 * 60 * 4));
        let mut reconnect_tick = tokio::time::interval(Duration::from_secs(51 * 33));
        let mut print_tick =
            tokio::time::interval(Duration::from_secs(P2P_STATUS_PRINT_INTERVAL_SECS));
        // Align with mainnet low-bid replay cadence (BiddingProve::LOW_BID_LOOP_SECS = 10).
        let mut replay_tick = tokio::time::interval(Duration::from_secs(10));
        let dial_interval = Duration::from_secs(self.config.dial_interval_secs.max(10));
        let mut empty_backbone_tick = tokio::time::interval(dial_interval);
        check_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        boost_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        find_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        reconnect_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        print_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        replay_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        empty_backbone_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

        // First find_nodes after ~15s (spawn once).
        if self.config.find_nodes {
            let this = self.clone();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_secs(15)).await;
                this.find_nodes().await;
            });
        }

        // Skip immediate first ticks for long intervals.
        check_tick.tick().await;
        boost_tick.tick().await;
        find_tick.tick().await;
        reconnect_tick.tick().await;
        print_tick.tick().await;
        replay_tick.tick().await;
        empty_backbone_tick.tick().await;

        loop {
            tokio::select! {
                _ = waiter.cancelled() => break,
                _ = check_tick.tick() => {
                    crate::keepalive::check_active(&self).await;
                    crate::keepalive::ping_backbones(&self).await;
                }
                _ = boost_tick.tick() => {
                    crate::keepalive::refill_backbone(&self).await;
                    if self.peertable.backbones().await.is_empty() {
                        self.dial_seeds(&seed_addrs).await;
                    }
                }
                _ = find_tick.tick(), if self.config.find_nodes => {
                    self.find_nodes().await;
                }
                _ = reconnect_tick.tick(), if self.config.find_nodes => {
                    if self.peertable.backbones().await.len() < 2 {
                        self.dial_seeds(&seed_addrs).await;
                    }
                }
                _ = empty_backbone_tick.tick() => {
                    // `[node].dial_interval`: re-dial seeds/addrbook when backbone is empty.
                    if self.peertable.backbones().await.is_empty() {
                        self.dial_seeds(&seed_addrs).await;
                        self.refill_backbone_from_addrbook().await;
                    }
                }
                _ = replay_tick.tick() => {
                    if let Some(hold) = waiter.try_hold() {
                        let this = self.clone();
                        tokio::task::spawn_blocking(move || {
                            let _hold = hold;
                            if !this.stopping.load(std::sync::atomic::Ordering::Acquire) {
                                this.drain_deferred_blocks();
                            }
                        });
                    }
                }
                _ = print_tick.tick() => {
                    let bbs = self.peertable.backbones().await;
                    let offs = self.peertable.offshoots().await;
                    let names = bbs
                        .iter()
                        .map(|peer| {
                            format!("{}({})", peer.nick(), peer.node_key[..2].to_hex())
                        })
                        .collect::<Vec<_>>();
                    sys::flush!("{}", format_p2p_status(
                        bbs.len(),
                        offs.len(),
                        &self.config.node_key,
                        &names,
                    ));
                    if let Some(hold) = waiter.try_hold() {
                        let this = self.clone();
                        tokio::task::spawn_blocking(move || {
                            let _hold = hold;
                            if !this.stopping.load(std::sync::atomic::Ordering::Acquire) {
                                this.drain_deferred_blocks();
                            }
                        });
                    }
                }
                accept = async {
                    match &listen_socket {
                        Some(l) => l.accept().await,
                        None => std::future::pending().await,
                    }
                } => {
                    match accept {
                        Ok((stream, peer_addr)) => {
                            let this = self.clone();
                            tokio::spawn(async move {
                                if let Err(e) = handle_conn_accept(this, stream, peer_addr).await {
                                    eprintln!("[P2P] inbound {} error: {}", peer_addr, e);
                                }
                            });
                        }
                        Err(e) => eprintln!("[P2P] accept failed: {}", e),
                    }
                }
            }
        }
        // Disconnect all on shutdown.
        for p in self.peertable.all_peers().await {
            p.disconnect();
        }
        Ok(())
    }

    async fn dial_seeds(self: &Arc<Self>, seeds: &[SocketAddr]) {
        for addr in seeds {
            let addr_str = addr.to_string();
            if self.boot_already_linked(&addr_str) {
                continue;
            }
            if self.peertable.backbones().await.len() >= self.config.backbone_peers {
                break;
            }
            let this = self.clone();
            let addr = *addr;
            tokio::spawn(async move {
                if let Err(e) = this.connect_addr(addr).await {
                    eprintln!("[P2P] dial {} failed: {}", addr, e);
                }
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{P2P_STATUS_PRINT_INTERVAL_SECS, format_p2p_status};

    #[test]
    fn status_print_matches_fullnodedev_content_and_frequency() {
        assert_eq!(P2P_STATUS_PRINT_INTERVAL_SECS, 60 * 97);
        assert_eq!(
            format_p2p_status(
                2,
                7,
                &[0xab, 0xcd, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
                &[
                    "alpha<127.0.0.1:3337>(0123)".to_owned(),
                    "beta(4567)".to_owned()
                ],
            ),
            "[P2P] 2 public 7 subnet nodes connected, key(abcd) => \
             alpha<127.0.0.1:3337>(0123), beta(4567).\n"
        );
    }
}

fn parse_boot_addrs(boots: &[String]) -> Vec<SocketAddr> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for s in boots {
        let s = s.trim();
        if s.is_empty() {
            continue;
        }
        match s.parse::<SocketAddr>() {
            Ok(addr) if seen.insert(addr) => out.push(addr),
            Ok(_) => {}
            Err(e) => eprintln!("[P2P] invalid boot address '{}': {}", s, e),
        }
    }
    out
}

// ===================================================================
// Inbound (accept): read-first magic detection, dispatch to v1/v2
// ===================================================================

/// Accept-side connection handler. Reads the first 4 bytes (magic) to
/// identify new vs old peer, then routes to the version-specific handshake.
async fn handle_conn_accept(
    node: Arc<P2PNode>,
    mut stream: tokio::net::TcpStream,
    peer_addr: SocketAddr,
) -> Rerr {
    let is_v2 = tokio::time::timeout(HANDSHAKE_TIMEOUT, accept_read_magic(&mut stream))
        .await
        .map_err(|_| sys::Error::fault("p2p magic handshake timeout".to_owned()))??;
    if is_v2 {
        write_v2_magic(&mut stream).await?;
        node.run_v2_session(stream, peer_addr, false).await
    } else {
        // v1 peer (magic already consumed by accept_read_magic).
        // Reconstruct a 4-byte v1 magic prefix is impossible; instead we
        // run the v1 handshake which expects to write-then-read magic. Since
        // we already read the peer's magic, we only write ours back.
        legacy::write_all(&mut stream, &P2P_MAGIC_V1.to_be_bytes()).await?;
        handle_conn_v1_body(node, stream, peer_addr, false).await
    }
}

// ===================================================================
// v1 (legacy) session
// ===================================================================

/// v1 dial/accept entry that does the full magic handshake (write+read).
/// Used by outbound v1 dial (and v1-only accept if magic not yet exchanged).
async fn handle_conn_v1(
    node: Arc<P2PNode>,
    mut stream: tokio::net::TcpStream,
    peer_addr: SocketAddr,
    report_me: bool,
) -> Rerr {
    tokio::time::timeout(HANDSHAKE_TIMEOUT, legacy::tcp_check_handshake(&mut stream))
        .await
        .map_err(|_| sys::Error::fault("v1 magic handshake timeout".to_owned()))??;
    handle_conn_v1_body(node, stream, peer_addr, report_me).await
}

/// v1 connection body after magic exchange: REPORT/ANSWER identity, then session.
async fn handle_conn_v1_body(
    node: Arc<P2PNode>,
    mut stream: tokio::net::TcpStream,
    peer_addr: SocketAddr,
    report_me: bool,
) -> Rerr {
    let my_info = build_node_info(
        &node.config.node_key,
        &node.config.node_name,
        node.config.listen_port,
    );
    if report_me {
        legacy::write_transport_msg(&mut stream, V1_MSG_REPORT_PEER, &my_info).await?;
    }

    let (ty, body) =
        tokio::time::timeout(HANDSHAKE_TIMEOUT, legacy::read_transport_msg(&mut stream))
            .await
            .map_err(|_| sys::Error::fault("v1 identity handshake timeout".to_owned()))??;
    // One-shot discovery/probe messages (no session).
    if ty == V1_MSG_REMIND_ME_IS_PUBLIC {
        return Ok(());
    }
    if ty == V1_MSG_REQUEST_NODE_KEY_FOR_PUBLIC_CHECK {
        legacy::write_all(&mut stream, &node.config.node_key).await?;
        return Ok(());
    }
    if ty == V1_MSG_REQUEST_NEAREST_PUBLIC_NODES {
        return node.serve_nearest_public_nodes(&mut stream).await;
    }

    let (identity, is_inbound, is_public, mut session_addr) = if ty == V1_MSG_REPORT_PEER {
        let id = parse_report_peer(&body)?;
        if id.key == node.config.node_key {
            return sys::errf!("cannot connect to self");
        }
        let answer = node_info_key_name(&my_info);
        legacy::write_transport_msg(&mut stream, V1_MSG_ANSWER_PEER, &answer).await?;
        let (pub_ok, addr) =
            maybe_mark_public_from_report(peer_addr, id.listen_port, &id.key).await;
        (id, true, pub_ok, addr)
    } else if ty == V1_MSG_ANSWER_PEER {
        let id = parse_answer_peer(&body)?;
        if id.key == node.config.node_key {
            return sys::errf!("cannot connect to self");
        }
        let pub_ok = public_from_answer(peer_addr);
        (id, false, pub_ok, peer_addr)
    } else {
        return sys::errf!("unexpected v1 handshake msg type {}", ty);
    };

    // If REPORT declared a listen port and probe failed, keep TCP peer_addr.
    if identity.listen_port > 0 && is_inbound && !is_public {
        session_addr = peer_addr;
    }

    run_peer_session(
        node,
        stream,
        session_addr,
        // v1 has no service bits / relay flag: treat as full-service relay.
        // v1 is a legacy wire protocol scheduled for removal once the network
        // fully switches to v2; its `NODE_DIAMOND` reference below is kept
        // verbatim and not generalized (the whole v1 path will be deleted).
        PeerIdentity::from_legacy(
            identity,
            services::NODE_NETWORK | services::NODE_SYNC | services::NODE_DIAMOND,
            true,
        ),
        is_inbound,
        is_public,
        report_me,
        ProtocolVersion::V1,
    )
    .await
}

// ===================================================================
// v2 session
// ===================================================================

impl P2PNode {
    /// Run v2 handshake + session on a stream whose magic exchange is done.
    async fn run_v2_session(
        self: Arc<Self>,
        mut stream: tokio::net::TcpStream,
        peer_addr: SocketAddr,
        from_dial: bool,
    ) -> Rerr {
        let genesis = self.engine.consensus().genesis_block().hash();
        let latest = self.engine.latest_block();
        let my_services = self.local_services();
        let my_version = VersionMessage::build(
            &self.config.node_key,
            &self.config.node_name,
            self.config.listen_port,
            my_services,
            &genesis,
            latest.height(),
            "hacash-next/0.1",
            &self.registered_custom_message_types(),
        );

        let peer_version = if from_dial {
            exchange_handshake(&mut stream, &my_version).await?
        } else {
            // Accept: first frame may be public probe or VERSION.
            let (ty, body) = tokio::time::timeout(HANDSHAKE_TIMEOUT, v2_read_msg(&mut stream))
                .await
                .map_err(|_| sys::Error::fault("v2 accept first-frame timeout".to_owned()))??;
            if ty == v2msg::MSG_CHECK_PUBLIC {
                return serve_check_public_v2(&mut stream, &self.config.node_key).await;
            }
            if ty != v2msg::MSG_VERSION {
                return sys::errf!("unexpected v2 first msg type {}", ty);
            }
            let peer = VersionMessage::decode(&body)?;
            peer.validate_as_peer()?;
            tokio::time::timeout(HANDSHAKE_TIMEOUT, async {
                crate::p2p::handshake::send_version(&mut stream, &my_version).await?;
                crate::p2p::handshake::send_verack(&mut stream).await?;
                crate::p2p::handshake::read_verack(&mut stream).await?;
                Ok::<(), sys::Error>(())
            })
            .await
            .map_err(|_| sys::Error::fault("v2 accept handshake timeout".to_owned()))??;
            peer
        };

        if peer_version.genesis_hash != genesis.into_array() {
            return sys::errf!("v2 genesis mismatch with peer {}", peer_addr);
        }
        if peer_version.node_key == self.config.node_key {
            return sys::errf!("cannot connect to self");
        }

        let identity = PeerIdentity::from_version(&peer_version);
        let is_inbound = !from_dial;
        let (is_public, session_addr) = if is_inbound {
            maybe_mark_public_from_version(
                peer_addr,
                identity.listen_port,
                &identity.key,
                peer_version.is_public(),
            )
            .await
        } else {
            // Outbound: reachable by definition (we dialed); public iff claimed + non-loopback.
            let pub_ok = peer_version.is_public() && !peer_addr.ip().is_loopback();
            (pub_ok, peer_addr)
        };

        run_peer_session(
            self,
            stream,
            session_addr,
            identity,
            is_inbound,
            is_public,
            from_dial,
            ProtocolVersion::V2,
        )
        .await
    }

    /// Compute local services bits for VERSION advertisement.
    ///
    /// `NODE_NETWORK` / `NODE_PUBLIC` / `NODE_SYNC` are universal P2P
    /// capabilities named here. Business relay channels are named by the
    /// consensus layer via `TxPolicy::tx_pool_groups`; we aggregate
    /// their `service_bit`s verbatim and the node does not interpret them.
    fn local_services(&self) -> u64 {
        let mut s = services::NODE_NETWORK | services::NODE_SYNC;
        if self.config.accept_nodes && self.config.listen_port > 0 {
            s |= services::NODE_PUBLIC;
        }
        for spec in self.engine.tx_policy().tx_pool_groups() {
            if let Some(bit) = spec.relay_service_bit {
                s |= bit;
            }
        }
        s
    }
}

// ===================================================================
// Shared peer session (version-aware read loop)
// ===================================================================

/// Construct the `RemotePeer`, insert into peertable, then spawn the read loop.
///
/// Returns after the peer is live in the table (mainnet `insert_peer` semantics),
/// so dialers can continue without waiting for disconnect.
async fn run_peer_session(
    node: Arc<P2PNode>,
    stream: tokio::net::TcpStream,
    peer_addr: SocketAddr,
    identity: PeerIdentity,
    is_inbound: bool,
    is_public: bool,
    from_dial: bool,
    protocol_version: ProtocolVersion,
) -> Rerr {
    let peer_id = peer_id_from_key(&identity.key);
    if node.peertable.has_peer(&peer_id).await {
        return Ok(());
    }

    let (reader, writer) = stream.into_split();
    let (ctrl_tx, ctrl_rx) = mpsc::channel::<PeerWriteCmd>(CTRL_WRITER_CAPACITY);
    let (tx_tx, tx_rx) = mpsc::channel::<PeerWriteCmd>(TX_WRITER_CAPACITY);
    let close_notify = Arc::new(Notify::new());
    let closed = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let start_height = identity.start_height;
    let peer = Arc::new(RemotePeer {
        id: peer_id.clone(),
        node_key: identity.key,
        name: identity.name.clone(),
        addr: peer_addr,
        listen_port: identity.listen_port,
        is_public: std::sync::atomic::AtomicBool::new(is_public),
        is_inbound: std::sync::atomic::AtomicBool::new(is_inbound),
        last_active: std::sync::Mutex::new(Instant::now()),
        protocol_version: std::sync::atomic::AtomicU8::new(protocol_version.as_u8()),
        remote_height: std::sync::atomic::AtomicU64::new(start_height),
        // v1 has no service bits: assume the peer relays every channel we
        // know about (equivalent to an all-ones mask). v2 peers expose their
        // advertised services verbatim; the node inspects individual channel
        // bits via `relays_channel(bit)` without naming them.
        service_mask: std::sync::atomic::AtomicU64::new(
            if protocol_version == ProtocolVersion::V1 {
                u64::MAX
            } else {
                identity.services
            },
        ),
        relay: std::sync::atomic::AtomicBool::new(identity.relay),
        custom_types: identity.custom_types.clone(),
        ctrl_tx,
        tx_tx,
        close_notify: close_notify.clone(),
        closed: closed.clone(),
        knows: crate::knowledge::Knowledge::new(500),
        rate_count: std::sync::atomic::AtomicU32::new(0),
        rate_window_start: std::sync::Mutex::new(Instant::now()),
    });

    if !node.add_peer(peer.clone()).await {
        return sys::errf!("peer capacity reached");
    }
    if from_dial {
        node.note_boot_link(&peer_addr.to_string(), &peer_id);
    }
    // v2 sync is started inside add_peer (which reads peer.remote_height).
    // v1 sync is also started inside add_peer via REQ_STATUS.
    println!(
        "[P2P] peer {} ({}) {} public={} v{} from {}",
        identity.name,
        peer_id,
        if is_inbound { "inbound" } else { "outbound" },
        is_public,
        protocol_version.as_u8(),
        peer_addr
    );

    let writer_task = spawn_writer(ctrl_rx, tx_rx, writer, close_notify.clone(), closed.clone());
    let node_bg = node.clone();
    let peer_bg = peer.clone();
    let peer_id_bg = peer_id.clone();
    tokio::spawn(async move {
        let mut reader = reader;
        let read_result = match protocol_version {
            ProtocolVersion::V2 => {
                run_v2_read_loop(node_bg.clone(), peer_bg.clone(), &mut reader, &peer_id_bg).await
            }
            ProtocolVersion::V1 => {
                run_v1_read_loop(node_bg.clone(), peer_bg.clone(), &mut reader, &peer_id_bg).await
            }
        };
        peer_bg.signal_close();
        node_bg.remove_peer(&peer_id_bg).await;
        writer_task.abort();
        println!("[P2P] peer {} disconnected", peer_id_bg);
        if let Err(e) = read_result {
            eprintln!("[P2P] peer {} session error: {}", peer_id_bg, e);
        }
    });
    Ok(())
}

/// Pre-dispatch bookkeeping shared by v1/v2 read loops: touch last_active
/// and enforce per-peer inbound rate limit. Returns false if the peer should
/// be disconnected for exceeding the rate budget.
fn pre_dispatch(peer: &RemotePeer, peer_id: &str) -> bool {
    peer.touch();
    if !peer.note_inbound_msg() {
        eprintln!("[P2P] peer {} exceeded inbound rate, disconnect", peer_id);
        peer.disconnect();
        false
    } else {
        true
    }
}

/// Dispatch an application message (u16 ty) shared by v1/v2 read loops.
/// TX/BLOCK go to the inbound queue with backpressure; everything else
/// spawns a task to call `handle_message`.
async fn dispatch_app_msg(
    node: &Arc<P2PNode>,
    peer: &Arc<RemotePeer>,
    peer_id: &str,
    ty: u16,
    body: Vec<u8>,
) {
    if peer.protocol_version() == ProtocolVersion::V2 && ty > 100 {
        if !peer.supports_custom_type(ty as u8) {
            eprintln!(
                "[P2P] peer {} sent unnegotiated custom message {}",
                peer_id, ty
            );
            peer.disconnect();
            return;
        }
        let Some(handler) = node.custom_message_handler(ty as u8) else {
            eprintln!(
                "[P2P] peer {} sent unregistered custom message {}",
                peer_id, ty
            );
            peer.disconnect();
            return;
        };
        let peer_ext: Arc<dyn base::Peer> = peer.clone();
        let pid = peer_id.to_string();
        tokio::spawn(async move {
            if let Err(e) = handler.handle(peer_ext, ty as u8, body) {
                eprintln!("[P2P] custom message {} from {} failed: {}", ty, pid, e);
            }
        });
    } else if ty == MSG_TX_SUBMIT {
        let msg = InboundMsg::Tx {
            peer: Some(peer_id.to_string()),
            body,
            ack: None,
        };
        let close_wait = peer.close_notify.notified();
        if peer.closed.load(std::sync::atomic::Ordering::Acquire) {
            return;
        }
        tokio::select! {
            _ = close_wait => {}
            result = node.inbound.send(msg) => {
                if let Err(e) = result {
                    eprintln!("[P2P] enqueue tx from {} failed: {}", peer_id, e);
                }
            }
        }
    } else if ty == MSG_BLOCK_DISCOVER {
        let msg = InboundMsg::Block {
            peer: Some(peer_id.to_string()),
            body,
            ack: None,
        };
        let close_wait = peer.close_notify.notified();
        if peer.closed.load(std::sync::atomic::Ordering::Acquire) {
            return;
        }
        tokio::select! {
            _ = close_wait => {}
            result = node.inbound.send(msg) => {
                if let Err(e) = result {
                    eprintln!("[P2P] enqueue block from {} failed: {}", peer_id, e);
                }
            }
        }
    } else {
        let node = node.clone();
        let peer_ext: Arc<dyn base::Peer> = peer.clone();
        let pid = peer_id.to_string();
        tokio::spawn(async move {
            if let Err(e) = node.handle_message(Some(peer_ext), ty, body).await {
                eprintln!("[P2P] peer {} msg {} failed: {}", pid, ty, e);
            }
        });
    }
}

/// v2 read loop: decode v2 frames (u8 ty), dispatch.
async fn run_v2_read_loop(
    node: Arc<P2PNode>,
    peer: Arc<RemotePeer>,
    reader: &mut (impl tokio::io::AsyncRead + Unpin),
    peer_id: &str,
) -> Rerr {
    loop {
        let close_wait = peer.close_notify.notified();
        if peer.closed.load(std::sync::atomic::Ordering::Acquire) {
            return Ok(());
        }
        let (ty, body) = tokio::select! {
            _ = close_wait => return Ok(()),
            r = v2_read_msg(reader) => match r {
                Ok(v) => v,
                Err(_) => return Ok(()),
            },
        };
        if !pre_dispatch(&peer, peer_id) {
            return Ok(());
        }
        match ty {
            v2msg::MSG_PING => {
                if let Ok(frame) = crate::p2p::codec::create_transport_frame(v2msg::MSG_PONG, &[]) {
                    let _ = peer.send_transport(frame);
                }
            }
            v2msg::MSG_PONG => {}
            v2msg::MSG_CLOSE => return Ok(()),
            v2msg::MSG_GETADDR => {
                if let Err(e) = node.handle_getaddr(&peer) {
                    eprintln!("[P2P] GETADDR reply to {} failed: {}", peer_id, e);
                }
            }
            v2msg::MSG_ADDR => {
                let node = node.clone();
                tokio::spawn(async move {
                    node.handle_addr_message(body).await;
                });
            }
            v2msg::MSG_RESERVED => {
                eprintln!("[P2P] peer {} sent reserved v2 message type 100", peer_id);
                peer.disconnect();
                return Ok(());
            }
            _ => dispatch_app_msg(&node, &peer, peer_id, ty as u16, body).await,
        }
    }
}

/// v1 read loop: decode v1 frames (u8 ty), unwrap CUSTOMER, dispatch.
async fn run_v1_read_loop(
    node: Arc<P2PNode>,
    peer: Arc<RemotePeer>,
    reader: &mut (impl tokio::io::AsyncRead + Unpin),
    peer_id: &str,
) -> Rerr {
    loop {
        let close_wait = peer.close_notify.notified();
        if peer.closed.load(std::sync::atomic::Ordering::Acquire) {
            return Ok(());
        }
        let (ty, body) = tokio::select! {
            _ = close_wait => return Ok(()),
            r = legacy::read_transport_msg(reader) => match r {
                Ok(v) => v,
                Err(_) => return Ok(()),
            },
        };
        if !pre_dispatch(&peer, peer_id) {
            return Ok(());
        }
        match ty {
            V1_MSG_CUSTOMER => {
                let (app_ty, app_body) = legacy::decode_customer(&body)?;
                dispatch_app_msg(&node, &peer, peer_id, app_ty, app_body).await;
            }
            V1_MSG_PING => {
                if let Ok(frame) = legacy::create_transport_frame(V1_MSG_PONG, &[]) {
                    let _ = peer.send_transport(frame);
                }
            }
            V1_MSG_PONG => {}
            V1_MSG_CLOSE => return Ok(()),
            _ => {}
        }
    }
}
