/*
* Copyright 2018-2022 TON DEV SOLUTIONS LTD.
*
* Licensed under the SOFTWARE EVALUATION License (the "License"); you may not use
* this file except in compliance with the License.  You may obtain a copy of the
* License at:
*
* https://www.ton.dev/licenses
*
* Unless required by applicable law or agreed to in writing, software
* distributed under the License is distributed on an "AS IS" BASIS,
* WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
* See the License for the specific TON DEV software governing permissions and limitations
* under the License.
*/

use crate::api::{ControlApi, MessageReceiverApi};
use crate::config::NodeConfig;
use crate::data::ArangoHelper;
use crate::engine::engine::TonNodeEngine;
use crate::engine::MessagesReceiver;
use crate::error::{NodeError, NodeResult};
use clap::{Arg, ArgMatches, Command};
use iron::Iron;
use parking_lot::Mutex;
use router::Router;
use serde_json::Value;
use std::{
    env, fs,
    path::{Path, PathBuf},
    sync::Arc,
    thread,
    time::Duration,
};
use ton_executor::BlockchainConfig;

pub mod error;

mod api;
mod block;
mod config;
mod data;
mod engine;

#[cfg(test)]
#[path = "../../../tonos-se-tests/unit/test_node_se.rs"]
mod tests;

fn main() {
    run().expect("Error run node");
}

fn read_str(path: &str) -> NodeResult<String> {
    Ok(fs::read_to_string(Path::new(path))
        .map_err(|err| NodeError::PathError(format!("Failed to read {}: {}", path, err)))?)
}

struct StartNodeConfig {
    node: NodeConfig,
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
        let node = parse_config(&config_json);

        let blockchain_config_json =
            read_str(args.value_of("blockchain-config").unwrap_or_default())?;
        let blockchain = blockchain_config_from_json(&blockchain_config_json)?;

        Ok(Self { node, blockchain })
    }
}

fn run() -> NodeResult<()> {
    println!(
        "Evernode Simple Emulator {}\n\
            RUST_VERSION: {}\n\
            COMMIT_ID: {}\n\
            BUILD_DATE: {}\n\
            COMMIT_DATE: {}\n\
            GIT_BRANCH: {}\n",
        env!("CARGO_PKG_VERSION"),
        env!("BUILD_RUST_VERSION"),
        env!("BUILD_GIT_COMMIT"),
        env!("BUILD_TIME"),
        env!("BUILD_GIT_DATE"),
        env!("BUILD_GIT_BRANCH")
    );

    let app = Command::new(env!("CARGO_PKG_NAME"))
        .version(env!("CARGO_PKG_VERSION"))
        .arg(
            Arg::new("workdir")
                .help("Path to working directory")
                .long("workdir")
                .takes_value(true)
                .max_values(1),
        )
        .arg(
            Arg::new("config")
                .help("configuration file name")
                .long("config")
                .required(true)
                .takes_value(true)
                .max_values(1),
        )
        .arg(
            Arg::new("blockchain-config")
                .help("blockchain configuration file name")
                .long("blockchain-config")
                .required(true)
                .takes_value(true)
                .max_values(1),
        )
        .arg(
            Arg::new("automsg")
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

    log::info!(target: "node", "Evernode Simple Emulator {}\nCOMMIT_ID: {}\nBUILD_DATE: {}\nCOMMIT_DATE: {}\nGIT_BRANCH: {}",
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
    let db = Arc::new(ArangoHelper::from_config(
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
        config.node.global_id,
        config.node.shard_id_config().shard_ident(),
        receivers,
        Some(control_api),
        Arc::new(config.blockchain),
        db,
        PathBuf::from("./"),
    )?;

    let ton = Arc::new(ton);
    TonNodeEngine::start(ton.clone())?;
    let addr = format!("{}:{}", config.node.api.address, config.node.api.port);

    if let Some(router) = router.lock().take() {
        thread::spawn(move || {
            Iron::new(router).http(addr).expect("error starting api");
        });
    }

    loop {
        thread::sleep(Duration::from_secs(1));
    }
}

fn parse_config(json: &str) -> NodeConfig {
    match NodeConfig::parse(json) {
        Ok(config) => config,
        Err(err) => {
            log::error!(target: "node", "Error parsing configuration file. {}", err);
            panic!("Error parsing configuration file. {}", err)
        }
    }
}

fn blockchain_config_from_json(json: &str) -> ton_types::Result<BlockchainConfig> {
    let map = serde_json::from_str::<serde_json::Map<String, Value>>(&json)?;
    let config_params = ton_block_json::parse_config(&map)?;
    BlockchainConfig::with_config(config_params)
}
