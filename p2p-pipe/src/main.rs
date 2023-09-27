use futures::{future::Either, prelude::*};
use libp2p::{
    core::{muxing::StreamMuxerBox, transport::OrTransport, upgrade},
    dns, gossipsub, identify, identity,
    kad::{
        self, record::store::MemoryStore, GetProvidersOk, Kademlia, KademliaEvent, Mode,
        QueryResult,
    },
    mdns, noise, quic,
    swarm::NetworkBehaviour,
    swarm::{SwarmBuilder, SwarmEvent},
    tcp, yamux, Multiaddr, PeerId, Transport,
};
use log::{debug, info};
use std::collections::hash_map::DefaultHasher;
use std::error::Error;
use std::hash::{Hash, Hasher};
use std::str::FromStr;
use std::time::Duration;
use tokio::{io, time};
use tokio_util::codec::{FramedRead, LinesCodec};

// We create a custom network behaviour that combines Gossipsub and Mdns.
#[derive(NetworkBehaviour)]
struct MyBehaviour {
    identify: identify::Behaviour,
    gossipsub: gossipsub::Behaviour,
    mdns: mdns::tokio::Behaviour,
    kademlia: Kademlia<MemoryStore>,
}

const GOSSIPSUB_TOPIC_NAME: &str = "test-gossipsub-topic";
const KADEMLIA_PROVIDE_NAME: &str = "test-kademlia-provides";
const BOOTNODES: [(&str, &str); 6] = [
    (
        "/dnsaddr/bootstrap.libp2p.io",
        "QmNnooDu7bfjPFoTZYxMNLWUQJyrVwtbZg5gBMjTezGAJN",
    ),
    (
        "/dnsaddr/bootstrap.libp2p.io",
        "QmQCU2EcMqAqQPR2i9bChDtGNJchTbq5TbXJJ16u19uLTa",
    ),
    (
        "/dnsaddr/bootstrap.libp2p.io",
        "QmbLHAnMoJPWSCR5Zhtx6BHJX9KiKNN6tpvbUcqanj75Nb",
    ),
    (
        "/dnsaddr/bootstrap.libp2p.io",
        "QmcZf59bWwK5XFi76CZX8cbJ4BhTzzA3gU1ZjYZcYW3dwt",
    ),
    (
        "/ip4/104.131.131.82/tcp/4001",
        "QmaCpDMGvV2BGHeYERUEnRQAwe3N8SzbUtfsmvsqQLuvuJ",
    ), // mars.i.ipfs.io
    (
        "/ip4/127.0.0.1/tcp/44001",
        "12D3KooWA8BKExWnva7sQnUHcs2zJ8QK7vMK9dCUfz5oW2A7Qbbh",
    ),
];

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let _ = env_logger::try_init();
    let id_keys = if let Ok(privkey) = std::env::var("PRIVKEY") {
        let bytes = hex::decode(privkey).unwrap();
        identity::Keypair::from_protobuf_encoding(&bytes).unwrap()
    } else {
        // Create a random PeerId
        identity::Keypair::generate_ed25519()
    };
    dbg!(hex::encode(&id_keys.to_protobuf_encoding().unwrap()));
    let pub_key = id_keys.public();
    let local_peer_id = PeerId::from(pub_key.clone());
    println!("Local peer id: {local_peer_id}");

    let transport = {
        let tcp_transport =
            tcp::tokio::Transport::new(tcp::Config::new().port_reuse(true).nodelay(true))
                .upgrade(upgrade::Version::V1)
                .authenticate(noise::Config::new(&id_keys)?)
                .multiplex(yamux::Config::default())
                .timeout(Duration::from_secs(20));

        let quic_transport = {
            let mut config = quic::Config::new(&id_keys);
            config.support_draft_29 = true;
            quic::tokio::Transport::new(config)
        };

        dns::TokioDnsConfig::system(OrTransport::new(quic_transport, tcp_transport))?
            .map(|either_output, _| match either_output {
                Either::Left((peer_id, muxer)) => (peer_id, StreamMuxerBox::new(muxer)),
                Either::Right((peer_id, muxer)) => (peer_id, StreamMuxerBox::new(muxer)),
            })
            .map_err(|err| io::Error::new(io::ErrorKind::Other, err))
            .boxed()
    };

    // To content-address message, we can take the hash of message and use it as an ID.
    let message_id_fn = |message: &gossipsub::Message| {
        let mut s = DefaultHasher::new();
        message.data.hash(&mut s);
        gossipsub::MessageId::from(s.finish().to_string())
    };

    // Set a custom gossipsub configuration
    let gossipsub_config = gossipsub::ConfigBuilder::default()
        .heartbeat_interval(Duration::from_secs(10)) // This is set to aid debugging by not cluttering the log space
        .validation_mode(gossipsub::ValidationMode::Strict) // This sets the kind of message validation. The default is Strict (enforce message signing)
        .message_id_fn(message_id_fn) // content-address messages. No two messages of the same content will be propagated.
        .build()
        .expect("Valid config");

    // build a gossipsub network behaviour
    let mut gossipsub = gossipsub::Behaviour::new(
        gossipsub::MessageAuthenticity::Signed(id_keys),
        gossipsub_config,
    )
    .expect("Correct configuration");
    // Create a Gossipsub topic
    let topic = gossipsub::IdentTopic::new(GOSSIPSUB_TOPIC_NAME);
    // subscribes to our topic
    gossipsub.subscribe(&topic)?;

    // Create a Swarm to manage peers and events
    let mut swarm = {
        let identify = identify::Behaviour::new(
            identify::Config::new("ipfs/0.1.0".to_string(), pub_key)
                .with_agent_version(format!("rust-libp2p-server/{}", env!("CARGO_PKG_VERSION"))),
        );
        let mdns = mdns::tokio::Behaviour::new(mdns::Config::default(), local_peer_id)?;
        let mut kademlia = Kademlia::new(local_peer_id, MemoryStore::new(local_peer_id));
        for (addr, id) in &BOOTNODES {
            kademlia.add_address(
                &PeerId::from_str(id).unwrap(),
                Multiaddr::from_str(addr).unwrap(),
            );
        }
        kademlia.bootstrap().unwrap();
        kademlia
            .start_providing(KADEMLIA_PROVIDE_NAME.as_bytes().to_vec().into())
            .unwrap();
        kademlia.set_mode(Some(Mode::Server));
        let behaviour = MyBehaviour {
            identify,
            gossipsub,
            mdns,
            kademlia,
        };
        SwarmBuilder::with_tokio_executor(transport, behaviour, local_peer_id).build()
    };

    let mut interval = time::interval(time::Duration::from_secs(30));
    // Read full lines from stdin
    let mut stdin = FramedRead::new(io::stdin(), LinesCodec::new()).fuse();

    // Listen on all interfaces and whatever port the OS assigns
    swarm.listen_on("/ip4/0.0.0.0/udp/0/quic-v1".parse()?)?;
    swarm.listen_on("/ip4/0.0.0.0/tcp/0".parse()?)?;

    println!("Enter messages via STDIN and they will be sent to connected peers using Gossipsub");
    // Kick it off
    loop {
        tokio::select! {
            _ = interval.tick() => {
                println!("ticked");
                let _query_id = swarm
                    .behaviour_mut()
                    .kademlia
                    .get_providers(KADEMLIA_PROVIDE_NAME.as_bytes().to_vec().into());
            },
            line = stdin.select_next_some() => {
                if let Err(e) = swarm
                    .behaviour_mut().gossipsub
                    .publish(topic.clone(), line.expect("Stdin not to close").as_bytes()) {
                    println!("Publish error: {e:?}");
                }
            },
            event = swarm.select_next_some() => {
                // dbg!(&event);
                match event {
                    SwarmEvent::Behaviour(MyBehaviourEvent::Identify(identify::Event::Received {
                            peer_id,
                            info:
                                identify::Info {
                                    listen_addrs,
                                    protocols,
                                    ..
                                },
                        })) => {
                            if protocols.iter().any(|p| *p == kad::PROTOCOL_NAME) {
                                for addr in listen_addrs {
                                    swarm
                                        .behaviour_mut()
                                        .kademlia
                                        .add_address(&peer_id, addr);
                                }
                            }
                    },
                    SwarmEvent::Behaviour(MyBehaviourEvent::Mdns(mdns::Event::Discovered(list))) => {
                        for (peer_id, _multiaddr) in list {
                            println!("mDNS discovered a new peer: {peer_id}");
                            swarm.behaviour_mut().gossipsub.add_explicit_peer(&peer_id);
                        }
                    },
                    SwarmEvent::Behaviour(MyBehaviourEvent::Mdns(mdns::Event::Expired(list))) => {
                        for (peer_id, _multiaddr) in list {
                            println!("mDNS discover peer has expired: {peer_id}");
                            swarm.behaviour_mut().gossipsub.remove_explicit_peer(&peer_id);
                        }
                    },
                    SwarmEvent::Behaviour(MyBehaviourEvent::Gossipsub(gossipsub::Event::Message {
                        propagation_source: peer_id,
                        message_id: id,
                        message,
                    })) => println!(
                            "Got message: '{}' with id: {id} from peer: {peer_id}",
                            String::from_utf8_lossy(&message.data),
                        ),
                    SwarmEvent::NewListenAddr { address, .. } => {
                        println!("Local node is listening on {address}");
                    },

                    SwarmEvent::Behaviour(MyBehaviourEvent::Kademlia(
                        KademliaEvent::OutboundQueryProgressed {
                            id: _,
                            result:
                                QueryResult::GetProviders(Ok(GetProvidersOk::FoundProviders {
                                    providers, ..
                                })),
                            ..
                        },
                    )) => {
                        println!("Obatained list of providers: {providers:?}");
                    },
                    SwarmEvent::Behaviour(MyBehaviourEvent::Kademlia(_)) => {
                        // dbg!(&event);
                    },
                        _ => {}
                    }
            }
        }
    }
}
