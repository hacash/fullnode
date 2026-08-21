//! TCP listener, dialer, session loop, and P2P maintenance.

use std::collections::HashSet;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::{Notify, mpsc};

use sys::{Rerr, ToHex, Waiter};

use crate::P2PNode;
use crate::msgqueue::InboundMsg;
use crate::p2p::codec::{accept_read_magic, dial_magic_exchange, read_transport_msg, write_magic};
use crate::p2p::handshake::{HANDSHAKE_TIMEOUT, PeerIdentity, VersionMessage, exchange_handshake};
use crate::p2p::msg::{
    MSG_ADDR, MSG_BLOCK_DISCOVER, MSG_CHECK_PUBLIC, MSG_CLOSE, MSG_GETADDR, MSG_PING, MSG_PONG,
    MSG_RESERVED, MSG_TX_SUBMIT, MSG_VERSION, services,
};
use crate::p2p::peer::{
    PEER_WRITER_CAPACITY, PeerWriteCmd, RemotePeer, next_peer_id, spawn_writer,
};
use crate::publiccheck::{maybe_mark_public_from_version, serve_check_public};

const P2P_STATUS_PRINT_INTERVAL_SECS: u64 = 60 * 97;
const MAGIC_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);

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

    /// Dial a peer and run the handshake.
    pub async fn connect_addr(self: &Arc<Self>, addr: SocketAddr) -> Rerr {
        let stream =
            tokio::time::timeout(Duration::from_secs(6), tokio::net::TcpStream::connect(addr))
                .await
                .map_err(|_| sys::Error::fault(format!("connect timeout {}", addr)))?
                .map_err(|e| sys::Error::fault(format!("connect {}: {}", addr, e)))?;
        let mut stream = stream;
        tokio::time::timeout(HANDSHAKE_TIMEOUT, dial_magic_exchange(&mut stream))
            .await
            .map_err(|_| sys::Error::fault("magic handshake timeout".to_owned()))??;
        self.clone().run_session(stream, addr, true).await
    }

    pub async fn run_p2p_async(self: Arc<Self>, waiter: Waiter) -> Rerr {
        let listener = self.bind_p2p_listener_async().await?;
        self.run_p2p_with_listener(waiter, listener).await
    }

    fn p2p_socket_addr(&self) -> sys::Ret<Option<SocketAddr>> {
        if self.config.listen_port == 0 {
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
        println!("[P2P] listening on {}", addr);
    }

    async fn run_p2p_with_listener(
        self: Arc<Self>,
        waiter: Waiter,
        listen_socket: Option<tokio::net::TcpListener>,
    ) -> Rerr {
        let seed_addrs = parse_boot_addrs(&self.config.boot_nodes);

        let startup_node = self.clone();
        let startup_seeds = seed_addrs.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(250)).await;
            startup_node.connect_stable_then_boot(&startup_seeds).await;
        });

        let mut check_tick = tokio::time::interval(Duration::from_secs(159));
        let mut boost_tick = tokio::time::interval(Duration::from_secs(270));
        let mut find_tick = tokio::time::interval(Duration::from_secs(52 * 60 * 4));
        let mut reconnect_tick = tokio::time::interval(Duration::from_secs(51 * 33));
        let mut print_tick =
            tokio::time::interval(Duration::from_secs(P2P_STATUS_PRINT_INTERVAL_SECS));
        // Align with mainnet low-bid replay cadence (BiddingProve::LOW_BID_LOOP_SECS = 10).
        let mut replay_tick = tokio::time::interval(Duration::from_secs(10));
        check_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        boost_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        find_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        reconnect_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        print_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        replay_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

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

        loop {
            tokio::select! {
                _ = waiter.cancelled() => break,
                _ = check_tick.tick() => {
                    crate::keepalive::check_active(&self).await;
                    crate::keepalive::ping_backbones(&self).await;
                }
                _ = boost_tick.tick() => {
                    crate::keepalive::boost_public(&self).await;
                    if self.peertable.backbones().await.is_empty() {
                        self.connect_stable_then_boot(&seed_addrs).await;
                    }
                }
                _ = find_tick.tick(), if self.config.find_nodes => {
                    self.find_nodes().await;
                }
                _ = reconnect_tick.tick(), if self.config.find_nodes => {
                    if self.peertable.backbones().await.len() < 2 {
                        self.connect_stable_then_boot(&seed_addrs).await;
                    }
                }
                _ = replay_tick.tick() => {
                    if let Some(hold) = waiter.try_hold() {
                        let this = self.clone();
                        tokio::task::spawn_blocking(move || {
                            let _hold = hold;
                            if !this.stopping.load(std::sync::atomic::Ordering::Acquire) {
                                let _ = this.drain_deferred_blocks();
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
                                let _ = this.drain_deferred_blocks();
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
                            if !self.config.accept_nodes {
                                continue;
                            }
                            let this = self.clone();
                            tokio::spawn(async move {
                                // fullnodedev silently drops connections that fail before
                                // becoming peers. Public ports routinely receive non-P2P probes.
                                let _ = handle_conn_accept(this, stream, peer_addr).await;
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

    async fn connect_stable_then_boot(self: &Arc<Self>, boots: &[SocketAddr]) {
        if self.config.use_stable_nodes {
            let stable = self.load_stable_nodes(boots).await;
            self.dial_addrs(&stable, "stable").await;
            if self.peertable.backbones().await.len() < self.config.backbone_peers {
                self.dial_addrs(boots, "boot").await;
            }
        } else {
            self.dial_addrs(boots, "boot").await;
        }
    }

    async fn load_stable_nodes(&self, boots: &[SocketAddr]) -> Vec<SocketAddr> {
        let mut seen = boots.iter().copied().collect::<HashSet<_>>();
        for peer in self.peertable.all_peers().await {
            seen.insert(peer.addr);
        }
        crate::stable_nodes::read_stable_file(&self.config.data_dir, self.config.backbone_peers)
            .into_iter()
            .filter(|addr| seen.insert(*addr))
            .collect()
    }

    async fn dial_addrs(self: &Arc<Self>, addrs: &[SocketAddr], source: &str) {
        for addr in addrs {
            if let Err(e) = self.connect_addr(*addr).await {
                eprintln!("[P2P] connect {} node {} failed: {}", source, addr, e);
            }
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
// Inbound (accept): magic and handshake
// ===================================================================

/// Accept-side connection handler.
async fn handle_conn_accept(
    node: Arc<P2PNode>,
    mut stream: tokio::net::TcpStream,
    peer_addr: SocketAddr,
) -> Rerr {
    tokio::time::timeout(MAGIC_HANDSHAKE_TIMEOUT, accept_read_magic(&mut stream))
        .await
        .map_err(|_| sys::Error::fault("p2p magic handshake timeout".to_owned()))??;
    write_magic(&mut stream).await?;
    node.run_session(stream, peer_addr, false).await
}

// ===================================================================
// Session
// ===================================================================

impl P2PNode {
    /// Run handshake + session on a stream whose magic exchange is done.
    async fn run_session(
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
            let (ty, body) =
                tokio::time::timeout(HANDSHAKE_TIMEOUT, read_transport_msg(&mut stream))
                    .await
                    .map_err(|_| sys::Error::fault("accept first-frame timeout".to_owned()))??;
            if ty == MSG_CHECK_PUBLIC {
                return serve_check_public(&mut stream, &self.config.node_key).await;
            }
            if ty == MSG_GETADDR {
                return self.serve_getaddr(&mut stream).await;
            }
            if ty != MSG_VERSION {
                return sys::errf!("unexpected first message type {}", ty);
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
            .map_err(|_| sys::Error::fault("accept handshake timeout".to_owned()))??;
            peer
        };

        if peer_version.node_key == self.config.node_key {
            return sys::errf!("cannot connect to self");
        }

        let identity = PeerIdentity::from_version(&peer_version);
        let is_inbound = !from_dial;
        let (is_public, session_addr) = if is_inbound {
            maybe_mark_public_from_version(peer_addr, identity.listen_port, &identity.key).await
        } else {
            // A successfully dialed non-loopback peer is public; capability
            // bits do not change table placement.
            let pub_ok = !peer_addr.ip().is_loopback();
            (pub_ok, peer_addr)
        };

        run_peer_session(self, stream, session_addr, identity, is_inbound, is_public).await
    }

    /// Local services bits for VERSION: `NODE_NETWORK`/`NODE_PUBLIC`/`NODE_SYNC` are
    /// universal; business relay bits come verbatim from `TxPolicy::tx_pool_groups`.
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
// Shared peer session
// ===================================================================

/// Construct the `RemotePeer`, insert into peertable, spawn the read loop; returns
/// once the peer is live in the table so dialers continue without waiting.
async fn run_peer_session(
    node: Arc<P2PNode>,
    stream: tokio::net::TcpStream,
    peer_addr: SocketAddr,
    identity: PeerIdentity,
    is_inbound: bool,
    is_public: bool,
) -> Rerr {
    let peer_id = next_peer_id();
    let (reader, writer) = stream.into_split();
    let (writer_tx, writer_rx) = mpsc::channel::<PeerWriteCmd>(PEER_WRITER_CAPACITY);
    let close_notify = Arc::new(Notify::new());
    let closed = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let peer = Arc::new(RemotePeer {
        id: peer_id.clone(),
        node_key: identity.key,
        name: identity.name.clone(),
        addr: peer_addr,
        listen_port: identity.listen_port,
        is_public: std::sync::atomic::AtomicBool::new(is_public),
        is_inbound: std::sync::atomic::AtomicBool::new(is_inbound),
        last_active: std::sync::Mutex::new(Instant::now()),
        service_mask: std::sync::atomic::AtomicU64::new(identity.services),
        relay: std::sync::atomic::AtomicBool::new(identity.relay),
        custom_types: identity.custom_types.clone(),
        writer_tx,
        close_notify: close_notify.clone(),
        closed: closed.clone(),
        knows: crate::knowledge::Knowledge::new(500),
    });

    let _writer_task = spawn_writer(writer_rx, writer, close_notify.clone(), closed.clone());
    node.add_peer(peer.clone()).await;
    println!(
        "[P2P] peer {} ({}) {} public={} from {}",
        identity.name,
        peer_id,
        if is_inbound { "inbound" } else { "outbound" },
        is_public,
        peer_addr
    );

    let node_bg = node.clone();
    let peer_bg = peer.clone();
    let peer_id_bg = peer_id.clone();
    tokio::spawn(async move {
        let mut reader = reader;
        let read_result =
            run_read_loop(node_bg.clone(), peer_bg.clone(), &mut reader, &peer_id_bg).await;
        peer_bg.disconnect();
        node_bg.remove_peer(&peer_bg).await;
        let disconnect_node = node_bg.clone();
        let disconnect_peer = peer_bg.clone();
        tokio::spawn(async move {
            disconnect_node.on_peer_disconnect(disconnect_peer);
        });
        println!("[P2P] peer {} disconnected", peer_id_bg);
        if let Err(e) = read_result {
            eprintln!("[P2P] peer {} session error: {}", peer_id_bg, e);
        }
    });
    let connect_node = node.clone();
    let connect_peer = peer.clone();
    tokio::spawn(async move {
        connect_node.on_peer_connect(connect_peer);
    });
    Ok(())
}

fn pre_dispatch(peer: &RemotePeer) {
    peer.touch();
}

/// Dispatch an application message (u16 ty): TX/BLOCK go to the inbound queue
/// with backpressure; everything else spawns a task to call `handle_message`.
async fn dispatch_app_msg(
    node: &Arc<P2PNode>,
    peer: &Arc<RemotePeer>,
    peer_id: &str,
    ty: u16,
    body: Vec<u8>,
) {
    if ty > 100 {
        if !peer.supports_custom_type(ty as u8) {
            eprintln!(
                "[P2P] peer {} sent unnegotiated custom message {}",
                peer_id, ty
            );
            return;
        }
        let Some(handler) = node.custom_message_handler(ty as u8) else {
            eprintln!(
                "[P2P] peer {} sent unregistered custom message {}",
                peer_id, ty
            );
            return;
        };
        let peer_ext: Arc<dyn base::Peer> = peer.clone();
        let pid = peer_id.to_string();
        tokio::spawn(async move {
            if let Err(e) = handler.handle(peer_ext, ty as u8, body) {
                eprintln!("[P2P] custom message {} from {} failed: {}", ty, pid, e);
            }
        });
    } else if ty == MSG_TX_SUBMIT || ty == MSG_BLOCK_DISCOVER {
        let inbound = node.inbound.clone();
        let pid = peer_id.to_string();
        let is_tx = ty == MSG_TX_SUBMIT;
        let msg = if is_tx {
            InboundMsg::Tx {
                peer: Some(pid.clone()),
                body,
                ack: None,
            }
        } else {
            InboundMsg::Block {
                peer: Some(pid.clone()),
                body,
                ack: None,
            }
        };
        tokio::spawn(async move {
            if let Err(e) = inbound.send(msg).await {
                eprintln!(
                    "[P2P] enqueue {} from {} failed: {}",
                    if is_tx { "tx" } else { "block" },
                    pid,
                    e
                );
            }
        });
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

/// Decode frames (u8 ty) and dispatch messages.
async fn run_read_loop(
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
            r = read_transport_msg(reader) => match r {
                Ok(v) => v,
                Err(_) => return Ok(()),
            },
        };
        pre_dispatch(&peer);
        match ty {
            MSG_PING => {
                if let Ok(frame) = crate::p2p::codec::create_transport_frame(MSG_PONG, &[]) {
                    let _ = peer.send_transport(frame);
                }
            }
            MSG_PONG => {}
            MSG_CLOSE => return Ok(()),
            MSG_GETADDR => {
                if let Err(e) = node.handle_getaddr(&peer) {
                    eprintln!("[P2P] GETADDR reply to {} failed: {}", peer_id, e);
                }
            }
            // Discovery responses are accepted only on the one-shot query
            // connection created by find_nodes, as in dev.
            MSG_ADDR => {}
            MSG_RESERVED => {
                eprintln!("[P2P] peer {} sent reserved message type 100", peer_id);
            }
            _ => dispatch_app_msg(&node, &peer, peer_id, ty as u16, body).await,
        }
    }
}
