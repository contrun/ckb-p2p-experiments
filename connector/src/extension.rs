use super::{
    message::{
        build_discovery_get_nodes, build_discovery_nodes, build_identify_message,
        build_relay_transaction, build_relay_transaction_hashes,
    },
    Connector, SupportProtocols,
};
use crate::Node;
use ckb_types::{
    bytes::Bytes,
    core::{Cycle, TransactionView},
    packed,
    prelude::*,
};
use p2p::multiaddr::Multiaddr;
/// Util functions attached to `Connector`.
use std::time::Duration;

impl Connector {
    pub fn send_relay_transaction(
        &self,
        node: &Node,
        relay_protocol: SupportProtocols,
        transaction: &TransactionView,
        cycles: Cycle,
    ) -> Result<(), String> {
        assert!(
            relay_protocol.protocol_id() == SupportProtocols::Relay.protocol_id()
                || relay_protocol.protocol_id() == SupportProtocols::RelayV2.protocol_id()
        );
        let message = build_relay_transaction(transaction, cycles);
        self.send(&node, relay_protocol, message.as_bytes())?;
        Ok(())
    }

    pub fn send_relay_transaction_hash(
        &self,
        node: &Node,
        relay_protocol: SupportProtocols,
        hashes: Vec<packed::Byte32>,
    ) -> Result<(), String> {
        assert!(
            relay_protocol.protocol_id() == SupportProtocols::Relay.protocol_id()
                || relay_protocol.protocol_id() == SupportProtocols::RelayV2.protocol_id()
        );
        let message = build_relay_transaction_hashes(hashes);
        self.send(node, relay_protocol, message.as_bytes())?;
        Ok(())
    }

    pub fn send_identify_message(
        &self,
        node: &Node,
        network_identifier: &str,
        client_version: &str,
        listening_addresses: Vec<Multiaddr>,
        observed_address: Multiaddr,
    ) -> Result<(), String> {
        let message = build_identify_message(
            network_identifier,
            client_version,
            listening_addresses,
            observed_address,
        );
        self.send(node, SupportProtocols::Identify, message.as_bytes())?;
        Ok(())
    }

    pub fn send_discovery_get_nodes(
        &self,
        node: &Node,
        listening_port: Option<u16>,
        max_nodes: u32,
        self_defined_flag: u32,
    ) -> Result<(), String> {
        let discovery = build_discovery_get_nodes(listening_port, max_nodes, self_defined_flag);
        self.send(node, SupportProtocols::Discovery, discovery.as_bytes())?;
        Ok(())
    }

    pub fn send_discovery_nodes(
        &self,
        node: &Node,
        active_push: bool,
        addresses: Vec<Multiaddr>,
    ) -> Result<(), String> {
        let message = build_discovery_nodes(active_push, addresses);
        self.send(node, SupportProtocols::Discovery, message.as_bytes())?;
        Ok(())
    }

    pub fn recv(&self, node: &Node, protocol: &SupportProtocols) -> Result<Bytes, String> {
        let session = self.get_session(node).ok_or(format!(
            "session to {} is notfound",
            node.p2p_address_with_node_id()
        ))?;
        let receiver = {
            let shared = self.shared.read().unwrap();
            shared
                .get_protocol_receiver(&session.id, &protocol.protocol_id())
                .ok_or(format!(
                    "protocol \"{}\" to {} is notfound",
                    protocol.name(),
                    node.p2p_address_with_node_id()
                ))?
        };
        receiver.recv().map_err(|err| format!("{:?}", err))
    }

    pub fn recv_timeout(
        &self,
        timeout: Duration,
        node: &Node,
        protocol: &SupportProtocols,
    ) -> Result<Bytes, String> {
        let session = self.get_session(node).ok_or(format!(
            "session to {} is notfound",
            node.p2p_address_with_node_id()
        ))?;
        let receiver = {
            let shared = self.shared.read().unwrap();
            shared
                .get_protocol_receiver(&session.id, &protocol.protocol_id())
                .ok_or(format!(
                    "protocol \"{}\" to {} is notfound",
                    protocol.name(),
                    node.p2p_address_with_node_id()
                ))?
        };
        receiver
            .recv_timeout(timeout)
            .map_err(|err| format!("{:?}", err))
    }
}
