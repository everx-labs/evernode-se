#![cfg_attr(feature = "ci_run", deny(warnings))]

extern crate clap;
extern crate ton_node;
#[macro_use]
extern crate log;
extern crate ed25519_dalek;
extern crate http;
extern crate iron;
extern crate log4rs;
extern crate parking_lot;
extern crate reqwest;
extern crate serde;
extern crate ton_block;
extern crate ton_types;
extern crate ton_vm as tvm;
#[macro_use]
extern crate serde_json;
extern crate base64;
extern crate router;
extern crate serde_derive;
extern crate ton_block_json;
extern crate ton_executor;

mod message_api;

use arango::ArangoHelper;
use clap::{App, Arg, ArgMatches};
use control_api::ControlApi;
use ed25519_dalek::{Keypair, PublicKey};
use iron::Iron;
use message_api::MessageReceiverApi;
use router::Router;
use serde_json::Value;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use ton_executor::BlockchainConfig;
use ton_node::error::{NodeError, NodeResult};
use ton_node::node_engine::config::NodeConfig;
use ton_node::node_engine::ton_node_engine::TonNodeEngine;
use ton_node::node_engine::ton_node_handlers::init_ton_node_handlers;
use ton_node::node_engine::{DocumentsDb, MessagesReceiver};

mod arango;
mod control_api;
#[cfg(test)]
#[path = "../../tonos-se-tests/unit/test_node_se.rs"]
mod tests;

fn main() {
    run().expect("Error run node");
}

fn read_str(path: &str) -> NodeResult<String> {
    Ok(fs::read_to_string(Path::new(path))
        .map_err(|err| NodeError::PathError(format!("Failed to read {}: {}", path, err)))?)
}

pub struct StartNodeConfig {
    node: NodeConfig,
    public_keys: Vec<PublicKey>,
    blockchain: BlockchainConfig,
}

impl StartNodeConfig {
    fn from_args(args: ArgMatches) -> NodeResult<Self> {
        if args.is_present("workdir") {
            if let Some(ref workdir) = args.value_of("workdir") {
                env::set_current_dir(Path::new(workdir)).unwrap()
            }
        }

        let config_json = read_str(args.value_of("config").unwrap_or_default())?;
        let (node, public_keys) = parse_config(&config_json);

        let blockchain_config_json =
            read_str(args.value_of("blockchain-config").unwrap_or_default())?;
        let blockchain = blockchain_config_from_json(&blockchain_config_json)?;

        Ok(Self {
            node,
            public_keys,
            blockchain,
        })
    }
}

fn run() -> NodeResult<()> {
    println!("TON Startup Edition Prototype {}\nCOMMIT_ID: {}\nBUILD_DATE: {}\nCOMMIT_DATE: {}\nGIT_BRANCH: {}",
        env!("CARGO_PKG_VERSION"),
        env!("BUILD_GIT_COMMIT"),
        env!("BUILD_TIME"),
        env!("BUILD_GIT_DATE"),
        env!("BUILD_GIT_BRANCH"));

    let app = App::new(env!("CARGO_PKG_NAME"))
        .version(env!("CARGO_PKG_VERSION"))
        .arg(
            Arg::with_name("workdir")
                .help("Path to working directory")
                .long("workdir")
                .takes_value(true)
                .max_values(1),
        )
        .arg(
            Arg::with_name("config")
                .help("configuration file name")
                .long("config")
                .required(true)
                .takes_value(true)
                .max_values(1),
        )
        .arg(
            Arg::with_name("blockchain-config")
                .help("blockchain configuration file name")
                .long("blockchain-config")
                .required(true)
                .takes_value(true)
                .max_values(1),
        )
        .arg(
            Arg::with_name("automsg")
                .help("Auto generate message timeout")
                .long("automsg")
                .takes_value(true)
                .max_values(1),
        );
    // let app = app.arg(
    //     Arg::with_name("localhost")
    //         .help("Localhost connectivity only")
    //         .long("localhost"),
    // );
    let config = StartNodeConfig::from_args(app.get_matches())?;

    log4rs::init_file(config.node.log_path.clone(), Default::default()).expect(&format!(
        "Error initialize logging configuration. config: {}",
        config.node.log_path
    ));

    info!(target: "node", "TON Node Startup Edition {}\nCOMMIT_ID: {}\nBUILD_DATE: {}\nCOMMIT_DATE: {}\nGIT_BRANCH: {}",
        env!("CARGO_PKG_VERSION"),
        env!("BUILD_GIT_COMMIT"),
        env!("BUILD_TIME") ,
        env!("BUILD_GIT_DATE"),
        env!("BUILD_GIT_BRANCH"));

    let err = start_node(config);
    log::error!(target: "node", "{:?}", err);

    Ok(())
}

fn start_node(config: StartNodeConfig) -> NodeResult<()> {
    let keypair = fs::read(Path::new(&config.node.private_key)).expect(&format!(
        "Error reading key file {}",
        config.node.private_key
    ));
    let private_key = Keypair::from_bytes(&keypair).unwrap();

    let db: Box<dyn DocumentsDb> = Box::new(ArangoHelper::from_config(
        &config.node.document_db_config(),
    )?);

    let router = Arc::new(Mutex::new(Some(Router::new())));

    let receivers: Vec<Box<dyn MessagesReceiver>> = vec![Box::new(MessageReceiverApi::new(
        config.node.api.messages.clone(),
        router.clone(),
    ))];

    let control_api = Box::new(ControlApi::new(
        config.node.api.live_control.clone(),
        router.clone(),
    )?);

    let ton = TonNodeEngine::with_params(
        config.node.shard_id_config().shard_ident(),
        true,
        config.node.port,
        config.node.node_index,
        0,
        0,
        private_key,
        config.public_keys,
        config.node.boot,
        receivers,
        control_api,
        config.blockchain,
        Some(db),
        PathBuf::from("./"),
    )?;

    init_ton_node_handlers(&ton);
    let ton = Arc::new(ton);
    TonNodeEngine::start(ton.clone())?;
    let addr = format!("{}:{}", config.node.api.address, config.node.api.port);

    if let Some(router) = router.lock().unwrap().take() {
        thread::spawn(move || {
            Iron::new(router).http(addr).expect("error starting api");
        });
    }

    loop {
        thread::sleep(Duration::from_secs(1));
    }
}

pub fn parse_config(json: &str) -> (NodeConfig, Vec<PublicKey>) {
    match NodeConfig::parse(json) {
        Ok(config) => match config.import_keys() {
            Ok(keys) => (config, keys),
            Err(err) => {
                log::error!(target: "node", "{}", err);
                panic!("{}", err)
            }
        },
        Err(err) => {
            log::error!(target: "node", "Error parsing configuration file. {}", err);
            panic!("Error parsing configuration file. {}", err)
        }
    }
}

pub fn blockchain_config_from_json(json: &str) -> ton_types::Result<BlockchainConfig> {
    let map = serde_json::from_str::<serde_json::Map<String, Value>>(&json)?;
    let config_params = ton_block_json::parse_config(&map)?;
    BlockchainConfig::with_config(config_params)
}
