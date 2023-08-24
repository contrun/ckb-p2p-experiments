use crate::network::{get_bootnodes, CKBNetworkType};

use ckb_connector::{
    compress, decompress, message::build_discovery_get_nodes, SharedState, SupportProtocols,
};
use ckb_types::{packed, prelude::*};

use p2p::{
    async_trait,
    builder::MetaBuilder as P2PMetaBuilder,
    bytes::{Bytes, BytesMut},
    context::ProtocolContext as P2PProtocolContext,
    context::ProtocolContextMutRef as P2PProtocolContextMutRef,
    context::ServiceContext as P2PServiceContext,
    service::ProtocolHandle as P2PProtocolHandle,
    service::ProtocolMeta as P2PProtocolMeta,
    service::ServiceError as P2PServiceError,
    service::ServiceEvent as P2PServiceEvent,
    service::TargetProtocol as P2PTargetProtocol,
    traits::ServiceHandle as P2PServiceHandle,
    traits::ServiceProtocol as P2PServiceProtocol,
};

use std::sync::{Arc, RwLock};
use std::time::Instant;

use tokio_util::codec::{length_delimited::LengthDelimitedCodec, Encoder};

#[derive(Clone)]
pub struct P2PNode {
    network_type: CKBNetworkType,
    shared: Arc<RwLock<SharedState>>,
}

impl P2PNode {
    /// Create a P2PNode
    pub fn new(network_type: CKBNetworkType, shared: Arc<RwLock<SharedState>>) -> Self {
        Self {
            network_type,
            shared,
        }
    }

    /// Convert P2PNode into P2PProtocolMeta
    pub fn build_protocol_metas(&self) -> Vec<P2PProtocolMeta> {
        vec![
            {
                let meta_builder: P2PMetaBuilder = SupportProtocols::Identify.into();
                meta_builder
                    .service_handle(move || P2PProtocolHandle::Callback(Box::new(self.clone())))
                    .build()
            },
            {
                let meta_builder: P2PMetaBuilder = SupportProtocols::Discovery.into();
                meta_builder
                    .service_handle(move || P2PProtocolHandle::Callback(Box::new(self.clone())))
                    .build()
            },
            {
                // Necessary to communicate with CKB full node
                let meta_builder: P2PMetaBuilder = SupportProtocols::Sync.into();
                meta_builder
                    // Only Timer, Sync, Relay make compress
                    .before_send(compress)
                    .before_receive(|| Some(Box::new(decompress)))
                    .service_handle(move || P2PProtocolHandle::Callback(Box::new(self.clone())))
                    .build()
            },
        ]
    }

    async fn received_identify(&mut self, context: P2PProtocolContextMutRef<'_>, data: Bytes) {
        match packed::IdentifyMessage::from_compatible_slice(data.as_ref()) {
            Ok(message) => {
                match packed::Identify::from_compatible_slice(
                    message.identify().raw_data().as_ref(),
                ) {
                    Ok(identify_payload) => {
                        let client_version_vec: Vec<u8> =
                            identify_payload.client_version().unpack();
                        let _client_version =
                            String::from_utf8_lossy(&client_version_vec).to_string();

                        let _client_name_vec: Vec<u8> = identify_payload.name().unpack();

                        let client_flag: u64 = identify_payload.flag().unpack();

                        // protocol is private mod in ckb, use the bitflag map directly
                        // since a light node can't provide LIGHT_CLIENT serv but full node can, use this as a workaround
                        let _is_full_node = (client_flag & 0b10000) == 0b10000;

                        log::info!(
                            "P2PNode received IdentifyMessage, address: {}, time: {:?}",
                            context.session.address,
                            Instant::now()
                        );
                    }
                    Err(err) => {
                        log::error!(
                            "P2PNode received invalid Identify Payload, address: {}, error: {:?}",
                            context.session.address,
                            err
                        );
                    }
                }
            }
            Err(err) => {
                log::error!(
                    "P2PNode received invalid IdentifyMessage, address: {}, error: {:?}",
                    context.session.address,
                    err
                );
            }
        }
    }

    #[allow(clippy::only_used_in_recursion)]
    async fn received_discovery(&mut self, context: P2PProtocolContextMutRef<'_>, data: Bytes) {
        match packed::DiscoveryMessage::from_compatible_slice(data.as_ref()) {
            Ok(message) => {
                match message.payload().to_enum() {
                    packed::DiscoveryPayloadUnion::Nodes(discovery_nodes) => {
                        log::debug!(
                            "P2PNode received DiscoveryMessages Nodes, address: {}, nodes.len: {}",
                            context.session.address,
                            discovery_nodes.items().len(),
                        );
                    }
                    packed::DiscoveryPayloadUnion::GetNodes(_discovery_get_nodes) => {
                        // discard
                    }
                }
            }
            Err(err) => {
                log::error!(
                    "P2PNode received invalid DiscoveryMessage, address: {}, error: {:?}",
                    context.session.address,
                    err
                );
            }
        }
    }

    async fn connected_discovery(
        &mut self,
        context: P2PProtocolContextMutRef<'_>,
        protocol_version: &str,
    ) {
        let discovery_get_node_message = build_discovery_get_nodes(None, 1000u32, 1u32);
        if protocol_version == "0.0.1" {
            let mut codec = LengthDelimitedCodec::new();
            let mut bytes = BytesMut::new();
            codec
                .encode(discovery_get_node_message.as_bytes(), &mut bytes)
                .expect("encode must be success");
            let message_bytes = bytes.freeze();
            context.send_message(message_bytes).await.unwrap();
        } else {
            let message_bytes = discovery_get_node_message.as_bytes();
            context.send_message(message_bytes).await.unwrap();
        }
    }
}

#[async_trait]
impl P2PServiceProtocol for P2PNode {
    async fn init(&mut self, context: &mut P2PProtocolContext) {
        let bootnodes = get_bootnodes(self.network_type);
        for node in bootnodes {
            log::debug!("Trying to dial {}", &node);
            let dial_res = context.dial(node.clone(), P2PTargetProtocol::All).await;
            log::debug!("Dial {} result: {:?}", &node, &dial_res);
        }
    }

    async fn notify(&mut self, _context: &mut P2PProtocolContext, _token: u64) {}

    async fn connected(&mut self, context: P2PProtocolContextMutRef<'_>, protocol_version: &str) {
        log::debug!(
            "P2PNode open protocol, protocol_name: {} address: {}",
            context
                .protocols()
                .get(&context.proto_id())
                .map(|p| p.name.as_str())
                .unwrap_or_default(),
            context.session.address
        );
        if let Ok(mut shared) = self.shared.write() {
            shared.add_protocol(context.session, context.proto_id);
        }

        if context.proto_id() == SupportProtocols::Discovery.protocol_id() {
            self.connected_discovery(context, protocol_version).await
        }
    }

    async fn disconnected(&mut self, context: P2PProtocolContextMutRef<'_>) {
        log::debug!(
            "P2PNode close protocol, protocol_name: {}, address: {:?}",
            context
                .protocols()
                .get(&context.proto_id())
                .map(|p| p.name.as_str())
                .unwrap_or_default(),
            context.session.address
        );
        if let Ok(mut shared) = self.shared.write() {
            shared.remove_protocol(&context.session.id, &context.proto_id());
        }
    }

    async fn received(&mut self, context: P2PProtocolContextMutRef<'_>, data: Bytes) {
        if context.proto_id == SupportProtocols::Discovery.protocol_id() {
            self.received_discovery(context, data).await
        } else if context.proto_id == SupportProtocols::Identify.protocol_id() {
            self.received_identify(context, data).await
        }
    }
}

#[async_trait]
impl P2PServiceHandle for P2PNode {
    async fn handle_error(&mut self, _context: &mut P2PServiceContext, error: P2PServiceError) {
        match &error {
            P2PServiceError::DialerError { address, error } => {
                log::error!("dialing to {} failed: {}", address, error)
            }
            P2PServiceError::ProtocolSelectError { proto_name, .. } => {
                log::error!("selecting protocol {:?} failed", proto_name)
            }
            _ => {
                log::error!("P2PNode detect service error, error: {:?}", error);
            }
        }
    }

    /// Handling session establishment and disconnection events
    async fn handle_event(&mut self, context: &mut P2PServiceContext, event: P2PServiceEvent) {
        log::debug!("P2PNode handle event: {:?}", event);
        match event {
            P2PServiceEvent::SessionOpen {
                session_context: session,
            } => {
                log::debug!("P2PNode open session: {:?}", session);
                // Reject passive connection
                if session.ty.is_inbound() {
                    let _ = context.disconnect(session.id);
                    return;
                }

                let _add = self
                    .shared
                    .write()
                    .map(|mut shared| shared.add_session(session.as_ref().to_owned()));
            }
            P2PServiceEvent::SessionClose {
                session_context: session,
            } => {
                log::debug!(
                    "P2PNode close session: {:?}, addr: {:?}",
                    session,
                    session.address
                );
                let _removed = self
                    .shared
                    .write()
                    .map(|mut shared| shared.remove_session(&session.id));
            }
            _ => {}
        }
    }
}
