use super::compress::{compress, decompress};
use super::SharedState;
use super::SupportProtocols;
use p2p::{
    async_trait,
    builder::MetaBuilder as P2PMetaBuilder,
    bytes,
    context::{ProtocolContext, ProtocolContextMutRef},
    service::{ProtocolHandle as P2PProtocolHandle, ProtocolMeta as P2PProtocolMeta},
    traits::ServiceProtocol as P2PServiceProtocol,
};
use std::sync::{Arc, RwLock};

/// Simple protocol handler which implements tentacle's
/// [`P2PServiceProtocol`](https://github.com/nervosnetwork/tentacle/blob/master/tentacle/src/traits.rs#L57-L77)
pub struct SimpleProtocolHandler {
    shared: Arc<RwLock<SharedState>>,
    protocol: SupportProtocols,
}

impl SimpleProtocolHandler {
    pub fn new(shared: Arc<RwLock<SharedState>>, protocol: SupportProtocols) -> Self {
        Self { shared, protocol }
    }

    pub fn build(self, be_compressed: bool) -> P2PProtocolMeta {
        let meta_builder: P2PMetaBuilder = self.protocol.clone().into();
        if be_compressed {
            meta_builder
                .before_send(compress)
                .before_receive(|| Some(Box::new(decompress)))
                .service_handle(move || P2PProtocolHandle::Callback(Box::new(self)))
                .build()
        } else {
            meta_builder
                .service_handle(move || P2PProtocolHandle::Callback(Box::new(self)))
                .build()
        }
    }
}

#[async_trait]
impl P2PServiceProtocol for SimpleProtocolHandler {
    async fn init(&mut self, _context: &mut ProtocolContext) {}

    async fn connected(&mut self, context: ProtocolContextMutRef<'_>, _protocol_version: &str) {
        crate::debug!(
            "SimpleProtocolHandler connected, protocol: {}, session: {:?}",
            self.protocol.name(),
            context.session
        );
        if let Ok(mut shared) = self.shared.write() {
            shared.add_protocol(context.session, context.proto_id);
        }
    }

    async fn disconnected(&mut self, context: ProtocolContextMutRef<'_>) {
        crate::debug!(
            "SimpleProtocolHandler disconnected, protocol: {}, session: {:?}",
            self.protocol.name(),
            context.session
        );
        if let Ok(mut shared) = self.shared.write() {
            shared.remove_protocol(&context.session.id, &context.proto_id());
        }
    }

    async fn received(&mut self, context: ProtocolContextMutRef<'_>, data: bytes::Bytes) {
        crate::debug!(
            "SimpleProtocolHandler received, protocol: {}, session: {:?}",
            self.protocol.name(),
            context.session
        );
        if let Ok(shared) = self.shared.write() {
            let sender = shared
                .get_protocol_sender(&context.session.id, &context.proto_id())
                .unwrap_or_else(|| {
                    panic!("received message but shared.get_protocol_sender returns None")
                });
            let _ = sender.send(data);
        }
    }
}
