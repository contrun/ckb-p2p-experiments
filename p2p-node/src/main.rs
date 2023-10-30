use ckb_connector::{ConnectorBuilder, SharedState};

use clap::{crate_version, Arg, Command};
use p2p::multiaddr::Multiaddr;
use std::env;

use std::sync::{Arc, RwLock};

use p2p_node::network::CKBNetworkType;
use p2p_node::node::P2PNode;
use p2p_node::Error;

use serde::Deserialize;

#[derive(Copy, Clone, Debug, Deserialize)]
pub enum NodeBackend {
    Tentacle,
    Libp2p,
}

impl TryFrom<&str> for NodeBackend {
    type Error = String;
    fn try_from(s: &str) -> Result<Self, Self::Error> {
        match s {
            "tentacle" => Ok(NodeBackend::Tentacle),
            "libp2p" => Ok(NodeBackend::Libp2p),
            _ => Err(format!(
                "Unknown backend {}, only tentacle and libp2p are supported",
                s
            )),
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    let _logger_guard = init_logger();
    log::info!("ckb-p2p-node starting");

    let matches = clap_app().get_matches();
    let address = matches.get_one::<String>("address").unwrap();
    let listening_address = Multiaddr::try_from(address.as_str()).unwrap();
    let network = matches.get_one::<String>("network").unwrap();
    let network_type = CKBNetworkType::try_from(network.as_str()).unwrap();
    let backend = matches
        .get_one::<String>("backend")
        .and_then(|b| NodeBackend::try_from(b.as_str()).ok())
        .unwrap();

    match backend {
        NodeBackend::Tentacle => tentacle_main(network_type, listening_address).await?,
        NodeBackend::Libp2p => libp2p_main(network_type, listening_address).await?,
    }
    Ok(())
}

async fn libp2p_main(
    _network_type: CKBNetworkType,
    _listening_address: Multiaddr,
) -> Result<(), Error> {
    unimplemented!("libp2p is currently not implemented");
}

async fn tentacle_main(
    network_type: CKBNetworkType,
    listening_address: Multiaddr,
) -> Result<(), Error> {
    log::info!(
        "Start listening for network {:?} at {}",
        network_type,
        listening_address
    );

    let shared_state = Arc::new(RwLock::new(SharedState::new()));
    let node = P2PNode::new(network_type, shared_state.clone());

    let _c = ConnectorBuilder::new()
        .protocol_metas(node.build_protocol_metas())
        .listening_addresses(vec![listening_address])
        .build(node, shared_state);

    tokio::signal::ctrl_c().await?;
    log::info!("p2p node shutting down");
    Ok(())
}

fn init_logger() -> ckb_logger_service::LoggerInitGuard {
    let filter = match env::var("RUST_LOG") {
        Ok(filter) if filter.is_empty() => Some("info".to_string()),
        Ok(filter) => Some(filter),
        Err(_) => Some("info".to_string()),
    };
    let config = ckb_logger_config::Config {
        filter,
        color: false,
        log_to_file: false,
        log_to_stdout: true,
        ..Default::default()
    };
    ckb_logger_service::init(None, config)
        .unwrap_or_else(|err| panic!("failed to init the logger service, error: {}", err))
}

pub fn clap_app() -> Command {
    Command::new("ckb-p2p-node")
        .version(crate_version!())
        .args([
            Arg::new("backend")
                .long("backend")
                .value_name("BACKEND")
                .required(false)
                .default_values(["tentacle"]),
            Arg::new("network")
                .long("network")
                .value_name("NETWORK")
                .required(false)
                .num_args(0..)
                .default_values(["dev"]),
            Arg::new("address")
                .long("address")
                .value_name("ADDRESS")
                .required(false)
                .default_values(["/ip4/127.0.0.1/tcp/8113"]),
        ])
}
