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

extern crate core;

use crate::config::NodeConfig;
use crate::error::{NodeError, NodeResult};
use crate::service::{TonNodeService, TonNodeServiceConfig};
use clap::{Arg, ArgMatches, Command};
use serde_json::Value;
use std::{env, fs, path::Path};
use ton_executor::BlockchainConfig;

pub mod error;

mod block;
mod config;
mod data;
mod engine;
mod service;

pub use data::MemDocumentsDb;
pub use data::MemStorage;
pub use engine::engine::TonNodeEngine;

#[cfg(test)]
mod tests;

fn main() {
    run().expect("Error run node");
}

fn read_str(path: &str) -> NodeResult<String> {
    Ok(fs::read_to_string(Path::new(path))
        .map_err(|err| NodeError::PathError(format!("Failed to read {}: {}", path, err)))?)
}

fn config_from_args(args: ArgMatches) -> NodeResult<TonNodeServiceConfig> {
    if args.is_present("workdir") {
        if let Some(ref workdir) = args.value_of("workdir") {
            env::set_current_dir(Path::new(workdir)).unwrap()
        }
    }

    let config_json = read_str(args.value_of("config").unwrap_or_default())?;
    let node = parse_config(&config_json);

    let blockchain_config_json = read_str(args.value_of("blockchain-config").unwrap_or_default())?;
    let blockchain = blockchain_config_from_json(&blockchain_config_json)?;

    Ok(TonNodeServiceConfig { node, blockchain })
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
    let config = config_from_args(app.get_matches())?;

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

    let service = TonNodeService::new(config)?;
    let err = service.run();
    log::error!(target: "node", "{:?}", err);

    Ok(())
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
