use crossbeam::channel::{unbounded, Receiver, Sender};
use p2p::{bytes::Bytes, context::SessionContext, secio::PeerId, ProtocolId, SessionId};
use std::collections::HashMap;

use crate::IdentifySessionBy;

pub type NetworkInfo = (
    SessionContext,
    HashMap<ProtocolId, (Sender<Bytes>, Receiver<Bytes>)>,
);

type SessionManager = HashMap<SessionId, NetworkInfo>;

/// Shared state between protocol handlers and service handler. As it is used across multiple
/// protocols, it should be wrapped into `Arc<RwLock<SharedState>>`.
#[derive(Default)]
pub struct SharedState {
    /// Session manager, #{ session.id => ( session, #{ protocol.id => mailbox } ) }
    session_manager: SessionManager,
    peers_to_sessions: HashMap<PeerId, Vec<SessionId>>,
}

impl SharedState {
    /// Create a shared state.
    pub fn new() -> Self {
        Default::default()
    }

    pub fn add_session(&mut self, session: SessionContext) {
        if let Some(ref key) = session.remote_pubkey {
            self.peers_to_sessions
                .entry(PeerId::from_public_key(key))
                .or_insert(vec![])
                .push(session.id);
        }
        self.session_manager
            .entry(session.id)
            .or_insert_with(|| (session.clone(), HashMap::new()));
    }

    pub fn remove_session(&mut self, session_id: &SessionId) -> Option<SessionContext> {
        let session = self
            .session_manager
            .remove(session_id)
            .map(|(session, _mailbox)| session);
        if let Some(ref s) = session {
            if let Some(key) = &s.remote_pubkey {
                let peer = PeerId::from_public_key(key);
                self.peers_to_sessions
                    .entry(peer.clone())
                    .and_modify(|v| v.retain(|&x| x != s.id));
                if let Some(v) = self.peers_to_sessions.get(&peer) {
                    if v.is_empty() {
                        self.peers_to_sessions.remove(&peer);
                    }
                }
            }
        }
        session
    }

    pub fn get_network_info(&self, id: IdentifySessionBy) -> Option<&NetworkInfo> {
        match id {
            IdentifySessionBy::Multiaddr(addr) => self
                .session_manager
                .values()
                .find(|(session, _)| session.address == *addr),
            IdentifySessionBy::SessionId(id) => self.session_manager.get(&id),
            IdentifySessionBy::PeerId(peer) => {
                let sessions = self.get_peer_sessions(peer);
                // TODO: sort the sessions for this peer, e.g. direct connection should be preferred.
                sessions
                    .first()
                    .and_then(|session| self.session_manager.get(&session.id))
            }
        }
    }

    pub fn get_session(&self, id: IdentifySessionBy) -> Option<SessionContext> {
        self.get_network_info(id)
            .map(|(session, _)| session.clone())
    }

    /// Get session by node address
    pub fn get_peer_session(&self, peer: &PeerId) -> Option<SessionContext> {
        self.get_session(IdentifySessionBy::PeerId(peer))
    }

    /// Get session by node address
    pub fn get_peer_sessions(&self, peer: &PeerId) -> Vec<SessionContext> {
        let empty_vec = vec![];
        self.peers_to_sessions
            .get(peer)
            .unwrap_or(&empty_vec)
            .iter()
            .filter_map(|id| {
                self.session_manager
                    .get(id)
                    .map(|(session, _)| session.clone())
            })
            .collect()
    }

    pub fn add_protocol(&mut self, session: &SessionContext, protocol_id: ProtocolId) {
        let (protocol_mailbox_sender, protocol_mailbox_receiver) = unbounded::<Bytes>();
        self.session_manager
            .entry(session.id)
            .or_insert_with(|| (session.clone(), HashMap::new()))
            .1
            .insert(
                protocol_id,
                (protocol_mailbox_sender, protocol_mailbox_receiver),
            );
    }

    pub fn remove_protocol(&mut self, session_id: &SessionId, protocol_id: &ProtocolId) {
        let _ = self
            .session_manager
            .get_mut(session_id)
            .map(|(_session, mailbox)| mailbox.remove(protocol_id));
    }

    pub fn get_protocol_sender(
        &self,
        session: IdentifySessionBy<'_>,
        protocol_id: &ProtocolId,
    ) -> Option<Sender<Bytes>> {
        self.get_network_info(session).and_then(|(_, mailbox)| {
            mailbox
                .get(protocol_id)
                .map(|(sender, _receiver)| sender.clone())
        })
    }

    pub fn get_protocol_receiver(
        &self,
        session: IdentifySessionBy<'_>,
        protocol_id: &ProtocolId,
    ) -> Option<Receiver<Bytes>> {
        self.get_network_info(session).and_then(|(_, mailbox)| {
            mailbox
                .get(protocol_id)
                .map(|(_sender, receiver)| receiver.clone())
        })
    }

    pub fn get_opened_protocol_ids(
        &self,
        session: IdentifySessionBy<'_>,
    ) -> Option<Vec<ProtocolId>> {
        self.get_network_info(session)
            .map(|(_session, mailbox)| mailbox.keys().cloned().collect())
    }

    /// Return all sessions
    pub fn get_sessions(&self) -> Vec<&SessionContext> {
        self.session_manager
            .values()
            .map(|(session, _)| session)
            .collect()
    }

    /// Return all session ids
    pub fn get_session_ids(&self) -> Vec<SessionId> {
        self.session_manager
            .values()
            .map(|(session, _)| session.id)
            .collect()
    }
}
