//! A set of functions used to construct network messages.
use ckb_types::{
    core::{Cycle, TransactionView},
    packed,
    prelude::*,
};
use p2p::multiaddr::Multiaddr;

pub fn build_identify_message(
    network_identifier: &str,
    client_version: &str,
    listening_addresses: Vec<Multiaddr>,
    observed_address: Multiaddr,
) -> packed::IdentifyMessage {
    let identify_self_defined_payload = packed::Identify::new_builder()
        .name(network_identifier.pack())
        .client_version(client_version.pack())
        .flag({
            // https://github.com/nervosnetwork/ckb/blob/3f89ae6dd2e0fd86b899b0c37dbe11864dc16544/network/src/protocols/identify/mod.rs#L604
            const FLAG_FULL_NODE: u64 = 1;
            FLAG_FULL_NODE.pack()
        })
        .build();
    packed::IdentifyMessage::new_builder()
        .identify({
            packed::Bytes::new_builder()
                .set(
                    identify_self_defined_payload
                        .as_bytes()
                        .iter()
                        .copied()
                        .map(Into::into)
                        .collect(),
                )
                .build()
        })
        .listen_addrs({
            let to_vec = listening_addresses
                .into_iter()
                .map(|addr| packed::Address::from_slice(&addr.to_vec()).unwrap())
                .collect::<Vec<_>>();
            packed::AddressVec::new_builder().set(to_vec).build()
        })
        .observed_addr({
            let byte_vec = observed_address
                .to_vec()
                .into_iter()
                .map(Into::into)
                .collect();
            let bytes = packed::Bytes::new_builder().set(byte_vec).build();
            packed::Address::new_builder().bytes(bytes).build()
        })
        .build()
}

pub fn build_relay_transaction(
    transaction: &TransactionView,
    cycles: Cycle,
) -> packed::RelayMessage {
    let relay_tx = packed::RelayTransaction::new_builder()
        .transaction(transaction.data())
        .cycles(cycles.pack())
        .build();
    let relay_tx_vec = packed::RelayTransactionVec::new_builder()
        .push(relay_tx)
        .build();
    let relay_txs = packed::RelayTransactions::new_builder()
        .transactions(relay_tx_vec)
        .build();
    packed::RelayMessage::new_builder().set(relay_txs).build()
}

pub fn build_relay_transaction_hashes(hashes: Vec<packed::Byte32>) -> packed::RelayMessage {
    let hashes = packed::Byte32Vec::new_builder().set(hashes).build();
    let relay_txs_hashes = packed::RelayTransactionHashes::new_builder()
        .tx_hashes(hashes)
        .build();
    packed::RelayMessage::new_builder()
        .set(relay_txs_hashes)
        .build()
}

pub fn build_discovery_get_nodes(
    listening_port: Option<u16>,
    max_nodes: u32,
    self_defined_flag: u32,
) -> packed::DiscoveryMessage {
    let discovery_payload = packed::DiscoveryPayload::new_builder()
        .set(
            packed::GetNodes::new_builder()
                .listen_port({
                    match listening_port {
                        None => packed::PortOpt::default(),
                        Some(port) => packed::PortOpt::new_builder()
                            .set(Some({
                                packed::Uint16::from_slice(&port.to_le_bytes()).unwrap()
                            }))
                            .build(),
                    }
                })
                .count(max_nodes.pack())
                .version(self_defined_flag.pack())
                .build(),
        )
        .build();
    packed::DiscoveryMessage::new_builder()
        .payload(discovery_payload)
        .build()
}

pub fn build_discovery_nodes(
    active_push: bool,
    addresses: Vec<Multiaddr>,
) -> packed::DiscoveryMessage {
    let nodes = addresses
        .into_iter()
        .map(|address| {
            let bytes = packed::Bytes::new_builder()
                .set(address.to_vec().into_iter().map(Into::into).collect())
                .build();
            packed::Node::new_builder()
                .addresses(vec![bytes].pack())
                .build()
        })
        .collect::<Vec<_>>();
    let discovery_payload = packed::DiscoveryPayload::new_builder()
        .set(
            packed::Nodes::new_builder()
                .announce(active_push.pack())
                .items(packed::NodeVec::new_builder().set(nodes).build())
                .build(),
        )
        .build();
    packed::DiscoveryMessage::new_builder()
        .payload(discovery_payload)
        .build()
}
