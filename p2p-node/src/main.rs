use ckb_testkit::connector::SharedState;

use clap::{crate_version, Arg, Command};
use std::env;

use std::sync::{Arc, RwLock};

use p2p_node::network::CKBNetworkType;
use p2p_node::node::P2PNode;
use p2p_node::Error;

pub use ckb_testkit::ckb_jsonrpc_types;
pub use ckb_testkit::ckb_types;

#[tokio::main]
async fn main() -> Result<(), Error> {
    let _logger_guard = init_logger();
    log::info!("ckb-p2p-node starting");
    let matches = clap_app().get_matches();
    let networks: Vec<_> = matches.get_many::<String>("network").unwrap().collect();
    log::info!("Networks: {:?}", networks);

    let network_types = networks
        .into_iter()
        .map(|x| CKBNetworkType::from(x.as_str()))
        .collect::<Vec<CKBNetworkType>>();
    // let mut _connectors = Vec::new();

    for network in network_types.iter() {
        log::info!("Start listening {:?}", network);
        let _shared_state = Arc::new(RwLock::new(SharedState::new()));
        let _node = P2PNode::new();
        // // workaround for Rust lifetime
        // _connectors.push(
        //     ConnectorBuilder::new()
        //         .protocol_metas(node.build_protocol_metas())
        //         .listening_addresses(vec![])
        //         .build(node, shared),
        // );
    }

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
    Command::new("ckb-p2p-node").version(crate_version!()).arg(
        Arg::new("network")
            .long("network")
            .value_name("NETWORK")
            .required(false)
            .num_args(0..)
            .default_values(["test"]),
    )
}
