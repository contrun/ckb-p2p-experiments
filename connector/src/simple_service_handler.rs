use super::SharedState;
use p2p::{
    async_trait,
    context::ServiceContext as P2PServiceContext, service::ServiceError as P2PServiceError,
    service::ServiceEvent as P2PServiceEvent, traits::ServiceHandle as P2PServiceHandle,
};
use std::sync::{Arc, RwLock};

/// TestServiceHandler is an implementation of `P2PServiceHandle` which handle service-wise
/// events and errors.
#[derive(Clone)]
pub struct SimpleServiceHandler {
    shared: Arc<RwLock<SharedState>>,
}

impl SimpleServiceHandler {
    pub fn new(shared: Arc<RwLock<SharedState>>) -> Self {
        Self { shared }
    }
}

#[async_trait]
impl P2PServiceHandle for SimpleServiceHandler {
    /// Handling runtime errors
    async fn handle_error(&mut self, _control: &mut P2PServiceContext, error: P2PServiceError) {
        crate::error!("TestServiceHandler detect error: {:?}", error);
    }

    /// Handling session establishment and disconnection events
    async fn handle_event(&mut self, _control: &mut P2PServiceContext, event: P2PServiceEvent) {
        match event {
            P2PServiceEvent::SessionOpen {
                session_context: session,
            } => {
                crate::debug!("TestServiceHandler open session: {:?}", session);
                let _ = self
                    .shared
                    .write()
                    .map(|mut shared| shared.add_session(session.as_ref().to_owned()));
            }
            P2PServiceEvent::SessionClose {
                session_context: session,
            } => {
                crate::debug!("TestServiceHandler close session: {:?}", session);
                let _ = self
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
