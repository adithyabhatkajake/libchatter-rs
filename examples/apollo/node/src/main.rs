// use futures::prelude::*;
use clap::{load_yaml, App};
use config::Node;
use types::apollo::{ClientMsg, ProtocolMsg, Transaction};
use std::error::Error;

fn main() -> Result<(), Box<dyn Error>> {
    let yaml = load_yaml!("cli.yml");
    let m = App::from_yaml(yaml).get_matches();

    let conf_str = m.value_of("config")
        .expect("unable to convert config file into a string");
    let conf_file = std::path::Path::new(conf_str);
    let str = String::from(conf_str);
    let mut config = match conf_file
        .extension()
        .expect("Unable to get file extension")
        .to_str()
        .expect("Failed to convert the extension into ascii string") 
    {
        "json" => Node::from_json(str),
        "dat" => Node::from_bin(str),
        "toml" => Node::from_toml(str),
        "yaml" => Node::from_yaml(str),
        _ => panic!("Invalid config file extension"),
    };
    if let Some(v) = m.value_of("delta") {
        config.delta = v.parse().expect("unexpected delta value provided");
    }

    if let Some(v) = m.value_of("sleep") {
        unsafe {
            config::SLEEP_TIME = v.parse().expect("unexpected sleep time");
        }
    } else {
        unsafe {
            config::SLEEP_TIME = (5 + config.num_nodes) as u64;
        }
    }

    config
        .validate()
        .expect("The decoded config is not valid");
    if let Some(f) = m.value_of("ip") {
        config.update_config(util::io::file_to_ips(f.to_string()));
    }
    let config = config;
    let mut is_client_apollo_enabled = false;
    if m.is_present("special_client") {
        is_client_apollo_enabled = true;
    } 

    simple_logger::SimpleLogger::new().init().unwrap();
    let x = m.occurrences_of("debug");
    match x {
        0 => log::set_max_level(log::LevelFilter::Info),
        1 => log::set_max_level(log::LevelFilter::Debug),
        2 | _ => log::set_max_level(log::LevelFilter::Trace),
    }

    log::info!("Successfully decoded the config file");

    let cli_net_rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    
    // Setup client network
    let cli_network = net::futures_manager::Protocol::<Transaction, ClientMsg>::new(config.id, config.num_nodes, config.root_cert.clone(), config.my_cert.clone(), config.my_cert_key.clone());
    let (cli_send, cli_recv) = 
    cli_net_rt.block_on(
        cli_network.client_setup(
            config.client_ip(),
            util::codec::EnCodec::new(),
            util::codec::Decodec::new(),
        )
    );

    let prot_net_rt = tokio::runtime::Builder::new_multi_thread()
    .enable_all()
    .build()
    .unwrap();

    // Setup networking
    let protocol_network = net::futures_manager::Protocol::<ProtocolMsg, ProtocolMsg>::new(config.id, config.num_nodes, config.root_cert.clone(), config.my_cert.clone(), config.my_cert_key.clone());

    // Setup the protocol network
    let (net_send, net_recv) = 
    prot_net_rt.block_on(
        protocol_network.server_setup(
            config.net_map.clone(), 
            util::codec::EnCodec::new(), 
            util::codec::Decodec::new()
        )
    );

    let core_rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .worker_threads(2)
        .build()
        .unwrap();
    
    log::info!("Using special apollo client: {}", is_client_apollo_enabled);
    // Start the Apollo consensus protocol
    core_rt.block_on(
        apollo::node::reactor(
            &config,
            is_client_apollo_enabled,
            net_send,
            net_recv,
            cli_send,
            cli_recv
        )
    );
    Ok(())
}

