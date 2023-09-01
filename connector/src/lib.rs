mod compress;
mod extension;
pub mod logger;
pub mod message;
mod shared;
mod simple_protocol_handler;
mod simple_service_handler;
mod support_protocols;

pub use compress::{compress, decompress};
use p2p::secio::PeerId;
use p2p::SessionId;
pub use shared::SharedState;
pub use simple_protocol_handler::SimpleProtocolHandler;
pub use simple_service_handler::SimpleServiceHandler;
pub use support_protocols::SupportProtocols;

use ckb_stop_handler::{SignalSender, StopHandler};
use p2p::{
    builder::ServiceBuilder, bytes::Bytes, context::SessionContext, multiaddr::Multiaddr,
    secio::SecioKeyPair, service::ProtocolMeta as P2PProtocolMeta, service::Service as P2PService,
    service::ServiceAsyncControl as P2PServiceAsyncControl,
    service::TargetProtocol as P2PTargetProtocol, traits::ServiceHandle as P2PServiceHandle,
    yamux::Config as YamuxConfig, ProtocolId,
};
use std::collections::HashSet;
use std::sync::{Arc, RwLock};
use std::thread::sleep;
use std::time::{Duration, Instant};

/// There are server ways to identify a session (a connection to remote peer).
/// `SessionId` is a obscure integer generated and given out by tentacle.
/// This number is not useful by itself as it contains no information about the remote peer.
/// `MultiAddr` is the transport address that the actual network connection uses.
/// `PeerId` is the id of the remote peer.
/// We should normally use `PeerId` to identify the network session that we want to use,
/// as there may exist many network session that corresponds to connection to the same peer.
/// Some are relayed connections, while some are direct connections.
/// In some case where the remote peer's id is not known before-hand, we can use,
/// `Multiaddr` or `SessionId` to identify the network session.
#[derive(Debug, Copy, Clone)]
pub enum IdentifySessionBy<'a> {
    SessionId(SessionId),
    Multiaddr(&'a Multiaddr),
    PeerId(&'a PeerId),
}

/// Connector Builder
pub struct ConnectorBuilder {
    key_pair: SecioKeyPair,
    // listening addresses
    listening_addresses: Vec<Multiaddr>,
    // protocol metas
    protocol_metas: Vec<P2PProtocolMeta>,
    // [`SessionConfig::yamux_config`](tentacle::service::config::SessionConfig::yamux_config)
    yamux_config: YamuxConfig,
    // [`SessionConfig::send_buffer_size`](tentacle::service::config::SessionConfig::send_buffer_size)
    send_buffer_size: usize,
    // [`SessionConfig::recv_buffer_size`](tentacle::service::config::SessionConfig::recv_buffer_size)
    recv_buffer_size: usize,
}

/// Connector is a fake node
pub struct Connector {
    #[allow(dead_code)]
    key_pair: SecioKeyPair,
    shared: Arc<RwLock<SharedState>>,
    p2p_service_controller: P2PServiceAsyncControl,
    _stop_handler: StopHandler<tokio::sync::oneshot::Sender<()>>,
}

impl Default for ConnectorBuilder {
    fn default() -> Self {
        Self {
            key_pair: SecioKeyPair::secp256k1_generated(),
            listening_addresses: Vec::new(),
            protocol_metas: Vec::new(),
            yamux_config: Default::default(),
            send_buffer_size: 24 * 1024 * 1024, // 24mb
            recv_buffer_size: 24 * 1024 * 1024, // 24mb
        }
    }
}

impl ConnectorBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn key_pair(mut self, key_pair: SecioKeyPair) -> Self {
        self.key_pair = key_pair;
        self
    }

    pub fn protocol_meta(mut self, protocol_meta: P2PProtocolMeta) -> Self {
        self.protocol_metas.push(protocol_meta);
        self
    }

    pub fn protocol_metas(mut self, protocol_metas: Vec<P2PProtocolMeta>) -> Self {
        self.protocol_metas.extend(protocol_metas);
        self
    }

    pub fn yamux_config(mut self, yamux_config: YamuxConfig) -> Self {
        self.yamux_config = yamux_config;
        self
    }

    pub fn send_buffer_size(mut self, send_buffer_size: usize) -> Self {
        self.send_buffer_size = send_buffer_size;
        self
    }

    pub fn recv_buffer_size(mut self, recv_buffer_size: usize) -> Self {
        self.recv_buffer_size = recv_buffer_size;
        self
    }

    pub fn listening_addresses(mut self, listening_addresses: Vec<Multiaddr>) -> Self {
        self.listening_addresses = listening_addresses;
        self
    }

    pub fn build<T>(self, service_handle: T, shared: Arc<RwLock<SharedState>>) -> Connector
    where
        T: P2PServiceHandle + Unpin + Send + 'static,
    {
        assert_eq!(
            self.protocol_metas.len(),
            self.protocol_metas
                .iter()
                .map(|protocol| protocol.name())
                .collect::<HashSet<_>>()
                .len(),
            "Duplicate protocols detected"
        );
        // Read more from https://github.com/nervosnetwork/ckb/blob/a25112f1032ac6796dc68fcf3922d316ae74db65/network/src/services/protocol_type_checker.rs#L1-L10
        assert!(
            self.protocol_metas.iter().any(|protocol| protocol.id() == SupportProtocols::Sync.protocol_id() ),
            "Sync protocol is the most underlying protocol to establish connection and must be contained in protocols",
        );
        let listening_addresses = self.listening_addresses.clone();
        let key_pair = self.key_pair.clone();

        // Start P2P Service and maintain the controller
        let mut p2p_service = self.build_p2p_service(service_handle);

        let p2p_service_controller = p2p_service.control().to_owned();
        let (stopped_signal_sender, mut stopped_signal_receiver) = tokio::sync::oneshot::channel();
        ::std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async move {
                if !listening_addresses.is_empty() {
                    for listening_address in listening_addresses {
                        let actual_listening_address =
                            p2p_service.listen(listening_address.clone()).await.unwrap();
                        assert_eq!(listening_address, actual_listening_address);
                    }
                }

                let p2p_service_controller = p2p_service.control().to_owned();
                loop {
                    tokio::select! {
                        _ = p2p_service.run() => {},
                        _ = &mut stopped_signal_receiver => {
                            let _ = p2p_service_controller.shutdown();
                            break;
                        }
                    }
                }
            });
        });

        Connector {
            key_pair,
            shared,
            p2p_service_controller,
            _stop_handler: StopHandler::new(
                SignalSender::Tokio(stopped_signal_sender),
                None,
                "connector".to_string(),
            ),
        }
    }

    // Create a p2p service instance
    fn build_p2p_service<T>(self, service_handle: T) -> P2PService<T>
    where
        T: P2PServiceHandle + Unpin,
    {
        let mut p2p_service_builder = ServiceBuilder::new();
        for protocol_meta in self.protocol_metas.into_iter() {
            p2p_service_builder = p2p_service_builder.insert_protocol(protocol_meta);
        }

        // Set SO_REUSEPORT and SO_REUSEADDR when this is supported on the running platform and
        // when there is only only listening address.
        #[cfg(any(unix, windows))]
        {
            use p2p::service::TcpSocket;
            use p2p::utils::multiaddr_to_socketaddr;

            #[cfg(unix)]
            use std::os::unix::io::{FromRawFd, IntoRawFd};

            #[cfg(windows)]
            use std::os::windows::io::{FromRawSocket, IntoRawSocket};

            if self.listening_addresses.len() == 1 {
                if let Some(addr) = multiaddr_to_socketaddr(&self.listening_addresses[0]) {
                    p2p_service_builder =
                        p2p_service_builder.tcp_config(move |socket: TcpSocket| {
                            let socket = unsafe {
                                #[cfg(unix)]
                                let socket = socket2::Socket::from_raw_fd(socket.into_raw_fd());
                                #[cfg(windows)]
                                let socket =
                                    socket2::Socket::from_raw_socket(socket.into_raw_socket());
                                socket
                            };
                            #[cfg(all(
                                unix,
                                not(target_os = "solaris"),
                                not(target_os = "illumos")
                            ))]
                            socket.set_reuse_port(true)?;
                            socket.set_reuse_address(true)?;
                            match socket.domain() {
                                Ok(domain) if domain == socket2::Domain::for_address(addr) => {
                                    socket.bind(&addr.into()).expect("Successfully rebind addr");
                                }
                                _ => {}
                            }
                            let socket = unsafe {
                                #[cfg(unix)]
                                let socket = TcpSocket::from_raw_fd(socket.into_raw_fd());
                                #[cfg(windows)]
                                let socket = TcpSocket::from_raw_socket(socket.into_raw_socket());
                                socket
                            };
                            Ok(socket)
                        });
                }
            }
        }

        p2p_service_builder
            .forever(true)
            .key_pair(self.key_pair.clone())
            .yamux_config(self.yamux_config)
            .set_send_buffer_size(self.send_buffer_size)
            .set_recv_buffer_size(self.recv_buffer_size)
            .build(service_handle)
    }
}

impl Connector {
    /// Try to establish connection with `node`. This function blocks until all protocols opened.
    pub async fn connect(&mut self, node_addr: &Multiaddr) -> Result<(), String> {
        crate::info!(
            "Connector try to make session establishment and open protocols to node \"{}\", protocols: {:?}",
            node_addr, self.p2p_service_controller.protocols(),
        );
        self.p2p_service_controller
            .dial(node_addr.clone(), P2PTargetProtocol::All)
            .await
            .map_err(|err| format!("Connector dial error: {:?}", err))?;

        // Wait for all protocols connections establishment
        let start_time = Instant::now();
        let mut last_logging_time = Instant::now();
        while start_time.elapsed() <= Duration::from_secs(5) {
            if let Some(opened_protocol_ids) =
                self.get_opened_protocol_ids(IdentifySessionBy::Multiaddr(node_addr))
            {
                let expected_opened = self
                    .p2p_service_controller
                    .protocols()
                    .iter()
                    .filter(|(protocol_id, _)| {
                        // TODO Filter out short-running protocols. Which protocols are short-running?
                        protocol_id != &&SupportProtocols::Identify.protocol_id()
                            && protocol_id != &&SupportProtocols::DisconnectMessage.protocol_id()
                    })
                    .count();
                if opened_protocol_ids.len() >= expected_opened {
                    return Ok(());
                }

                if last_logging_time.elapsed() > Duration::from_secs(1) {
                    last_logging_time = Instant::now();
                    crate::debug!(
                        "Connector is waiting protocols establishment to node \"{}\", trying protocols: {:?}, opened protocols: {:?}",
                        node_addr, self.p2p_service_controller.protocols(), opened_protocol_ids,
                    );
                }
                sleep(Duration::from_millis(100));
            } else {
                if last_logging_time.elapsed() > Duration::from_secs(1) {
                    last_logging_time = Instant::now();
                    crate::debug!(
                        "Connector is waiting session establishment to node \"{}\"",
                        node_addr
                    );
                }
                sleep(Duration::from_millis(100));
            }
        }

        Err(format!(
            "Connector is timeout when connecting to {}",
            node_addr
        ))
    }

    /// Send `data` through the protocol of the session
    pub async fn send(
        &self,
        session: IdentifySessionBy<'_>,
        protocol: SupportProtocols,
        data: Bytes,
    ) -> Result<(), String> {
        let session = self
            .get_session(session)
            .ok_or_else(|| format!("The connection to {:?} not found", session))?;
        self.p2p_service_controller
            .send_message_to(session.id, protocol.protocol_id(), data)
            .await
            .map_err(|err| {
                format!(
                    "Connector send message under protocol \"{}\" to {:?}, error: {:?}",
                    protocol.name(),
                    session,
                    err
                )
            })
    }

    /// Return the session corresponding to the `node` if connected.
    pub fn get_session(&self, session: IdentifySessionBy<'_>) -> Option<SessionContext> {
        if let Ok(shared) = self.shared.read() {
            return shared.get_session(session);
        }
        unreachable!()
    }

    /// Return the opened protocols of the session corresponding to the `node` if connected
    pub fn get_opened_protocol_ids(
        &self,
        session: IdentifySessionBy<'_>,
    ) -> Option<Vec<ProtocolId>> {
        if let Ok(shared) = self.shared.read() {
            return shared.get_opened_protocol_ids(session);
        }
        unreachable!()
    }

    /// Return the shared state
    pub fn shared(&self) -> &Arc<RwLock<SharedState>> {
        &self.shared
    }

    /// Return the P2P service controller
    pub fn p2p_service_controller(&self) -> &P2PServiceAsyncControl {
        &self.p2p_service_controller
    }

    pub fn key_pair(&self) -> &SecioKeyPair {
        &self.key_pair
    }
}
