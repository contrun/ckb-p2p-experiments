mod compress;
mod extension;
pub mod logger;
pub mod message;
mod shared;
mod simple_protocol_handler;
mod simple_service_handler;
mod support_protocols;

pub use compress::{compress, decompress};
pub use shared::SharedState;
pub use simple_protocol_handler::SimpleProtocolHandler;
pub use simple_service_handler::SimpleServiceHandler;
pub use support_protocols::SupportProtocols;

use ckb_stop_handler::{SignalSender, StopHandler};
use futures::prelude::*;
use p2p::{
    builder::ServiceBuilder, bytes::Bytes, context::SessionContext, multiaddr::Multiaddr,
    secio::SecioKeyPair, service::ProtocolMeta as P2PProtocolMeta, service::Service as P2PService,
    service::ServiceControl as P2PServiceControl, service::TargetProtocol as P2PTargetProtocol,
    traits::ServiceHandle as P2PServiceHandle, yamux::Config as YamuxConfig, ProtocolId,
};
use std::collections::HashSet;
use std::sync::{Arc, RwLock};
use std::thread::sleep;
use std::time::{Duration, Instant};

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
    p2p_service_controller: P2PServiceControl,
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

    /// ```rust
    /// use super::util::find_available_port;
    ///
    /// let p2p_port = find_available_port();
    /// let p2p_listening_address = format!("/ip4/127.0.0.1/tcp/{}", p2p_port).parse().unwrap();
    /// ```
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
                        Some(_) = p2p_service.next() => {},
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
    pub fn connect(&mut self, node_addr: &Multiaddr) -> Result<(), String> {
        crate::info!(
            "Connector try to make session establishment and open protocols to node \"{}\", protocols: {:?}",
            node_addr, self.p2p_service_controller.protocols(),
        );
        self.p2p_service_controller
            .dial(node_addr.clone(), P2PTargetProtocol::All)
            .map_err(|err| format!("Connector dial error: {:?}", err))?;

        // Wait for all protocols connections establishment
        let start_time = Instant::now();
        let mut last_logging_time = Instant::now();
        while start_time.elapsed() <= Duration::from_secs(5) {
            if let Some(opened_protocol_ids) = self.get_opened_protocol_ids(node_addr) {
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
    pub fn send(
        &self,
        node_addr: &Multiaddr,
        protocol: SupportProtocols,
        data: Bytes,
    ) -> Result<(), String> {
        let session = self
            .get_session(node_addr)
            .ok_or_else(|| format!("The connection was disconnected to \"{}\"", node_addr))?;
        self.p2p_service_controller
            .send_message_to(session.id, protocol.protocol_id(), data)
            .map_err(|err| {
                format!(
                    "Connector send message under protocol \"{}\" to \"{}\", error: {:?}",
                    protocol.name(),
                    node_addr,
                    err
                )
            })
    }

    /// Return the session corresponding to the `node` if connected.
    pub fn get_session(&self, node_addr: &Multiaddr) -> Option<SessionContext> {
        if let Ok(shared) = self.shared.read() {
            return shared.get_session(node_addr);
        }
        unreachable!()
    }

    /// Return the opened protocols of the session corresponding to the `node` if connected
    pub fn get_opened_protocol_ids(&self, node_addr: &Multiaddr) -> Option<Vec<ProtocolId>> {
        if let Ok(shared) = self.shared.read() {
            return shared
                .get_session(node_addr)
                .and_then(|session| shared.get_opened_protocol_ids(&session.id));
        }
        unreachable!()
    }

    /// Return the shared state
    pub fn shared(&self) -> &Arc<RwLock<SharedState>> {
        &self.shared
    }

    /// Return the P2P service controller
    pub fn p2p_service_controller(&self) -> &P2PServiceControl {
        &self.p2p_service_controller
    }

    pub fn key_pair(&self) -> &SecioKeyPair {
        &self.key_pair
    }
}
