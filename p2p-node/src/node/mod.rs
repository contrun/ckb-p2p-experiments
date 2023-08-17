use crate::network::{bootnodes, CKBNetworkType};

use ckb_testkit::connector::message::build_discovery_get_nodes;
use ckb_testkit::{
    ckb_types::{packed, prelude::*},
    compress,
    connector::SharedState,
    decompress, Node, SupportProtocols,
};
use p2p::error::{DialerErrorKind, SendErrorKind};
use p2p::{
    builder::MetaBuilder as P2PMetaBuilder,
    bytes::{Bytes, BytesMut},
    context::ProtocolContext as P2PProtocolContext,
    context::ProtocolContextMutRef as P2PProtocolContextMutRef,
    context::ServiceContext as P2PServiceContext,
    context::SessionContext,
    multiaddr::Multiaddr,
    service::ProtocolHandle as P2PProtocolHandle,
    service::ProtocolMeta as P2PProtocolMeta,
    service::ServiceError as P2PServiceError,
    service::ServiceEvent as P2PServiceEvent,
    service::TargetProtocol as P2PTargetProtocol,
    traits::ServiceHandle as P2PServiceHandle,
    traits::ServiceProtocol as P2PServiceProtocol,
};
use std::collections::{HashMap, HashSet};
use std::convert::TryFrom;
use std::error::Error;
use std::ops::Mul;
use std::str::FromStr;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};
use tokio::runtime::Handle;
use tokio_util::codec::{length_delimited::LengthDelimitedCodec, Decoder, Encoder};

// TODO Adjust the parameters
const DIAL_ONLINE_ADDRESSES_INTERVAL: Duration = Duration::from_secs(1);
const PRUNE_OFFLINE_ADDRESSES_INTERVAL: Duration = Duration::from_secs(60 * 60 * 24);

const DISCONNECT_TIMEOUT_SESSION_INTERVAL: Duration = Duration::from_secs(10);
const POSTGRES_ONLINE_ADDRESS_INTERVAL: Duration = Duration::from_secs(60);
const DIAL_ONLINE_ADDRESSES_TOKEN: u64 = 1;
const PRUNE_OFFLINE_ADDRESSES_TOKEN: u64 = 2;
const DISCONNECT_TIMEOUT_SESSION_TOKEN: u64 = 3;
const POSTGRES_ONLINE_ADDRESSES_TOKEN: u64 = 4;

const ADDRESS_TIMEOUT: Duration = Duration::from_secs(30);

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

    fn received_identify(&mut self, context: P2PProtocolContextMutRef, data: Bytes) {
        match packed::IdentifyMessage::from_compatible_slice(data.as_ref()) {
            Ok(message) => {
                match packed::Identify::from_compatible_slice(
                    message.identify().raw_data().as_ref(),
                ) {
                    Ok(identify_payload) => {
                        let client_version_vec: Vec<u8> =
                            identify_payload.client_version().unpack();
                        let client_version =
                            String::from_utf8_lossy(&client_version_vec).to_string();

                        let client_name_vec: Vec<u8> = identify_payload.name().unpack();

                        let client_flag: u64 = identify_payload.flag().unpack();

                        // protocol is private mod in ckb, use the bitflag map directly
                        // since a light node can't provide LIGHT_CLIENT serv but full node can, use this as a workaround
                        let is_full_node = (client_flag & 0b10000) == 0b10000;

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

    fn received_discovery(&mut self, context: P2PProtocolContextMutRef, data: Bytes) {
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
                // ckb2019, before hard fork
                let mut data = BytesMut::from(data.as_ref());
                let mut codec = LengthDelimitedCodec::new();
                match codec.decode(&mut data) {
                    Ok(Some(frame)) => self.received_discovery(context, frame.freeze()),
                    _ => {
                        log::error!(
                            "P2PNode received invalid DiscoveryMessage, address: {}, error: {:?}",
                            context.session.address,
                            err
                        );
                    }
                }
            }
        }
    }

    fn connected_discovery(&mut self, context: P2PProtocolContextMutRef, protocol_version: &str) {
        let discovery_get_node_message = build_discovery_get_nodes(None, 1000u32, 1u32);
        if protocol_version == "0.0.1" {
            let mut codec = LengthDelimitedCodec::new();
            let mut bytes = BytesMut::new();
            codec
                .encode(discovery_get_node_message.as_bytes(), &mut bytes)
                .expect("encode must be success");
            let message_bytes = bytes.freeze();
            context.send_message(message_bytes).unwrap();
        } else {
            let message_bytes = discovery_get_node_message.as_bytes();
            context.send_message(message_bytes).unwrap();
        }
    }
}

impl P2PServiceProtocol for P2PNode {
    fn init(&mut self, context: &mut P2PProtocolContext) {}

    fn notify(&mut self, context: &mut P2PProtocolContext, token: u64) {}

    fn connected(&mut self, context: P2PProtocolContextMutRef, protocol_version: &str) {
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
            self.connected_discovery(context, protocol_version)
        }
    }

    fn disconnected(&mut self, context: P2PProtocolContextMutRef) {
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

    fn received(&mut self, context: P2PProtocolContextMutRef, data: Bytes) {
        if context.proto_id == SupportProtocols::Discovery.protocol_id() {
            self.received_discovery(context, data)
        } else if context.proto_id == SupportProtocols::Identify.protocol_id() {
            self.received_identify(context, data)
        }
    }
}

impl P2PServiceHandle for P2PNode {
    fn handle_error(&mut self, _context: &mut P2PServiceContext, error: P2PServiceError) {
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
    fn handle_event(&mut self, context: &mut P2PServiceContext, event: P2PServiceEvent) {
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
            _ => {
                unimplemented!()
            }
        }
    }
}
