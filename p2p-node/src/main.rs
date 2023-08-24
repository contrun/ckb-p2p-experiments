use ckb_connector::{ConnectorBuilder, SharedState};

use clap::{crate_version, Arg, Command};
use p2p::multiaddr::Multiaddr;
use std::env;

use std::sync::{Arc, RwLock};

use p2p_node::network::CKBNetworkType;
use p2p_node::node::P2PNode;
use p2p_node::Error;

#[tokio::main]
async fn main() -> Result<(), Error> {
    let _logger_guard = init_logger();
    log::info!("ckb-p2p-node starting");

    let matches = clap_app().get_matches();
    let address = matches.get_one::<String>("address").unwrap();
    let listening_address = Multiaddr::try_from(address.as_str()).unwrap();
    let network = matches.get_one::<String>("network").unwrap();
    let network_type = CKBNetworkType::try_from(network.as_str()).unwrap();

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
