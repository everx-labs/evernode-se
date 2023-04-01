mod control_api;
mod message_api;

use crate::config::NodeConfig;
use crate::data::{ArangoHelper, FSStorage};
use crate::engine::engine::TonNodeEngine;
use crate::error::NodeResult;
use crate::service::control_api::ControlApi;
use crate::service::message_api::MessageReceiverApi;
use iron::Iron;
use router::Router;
use std::path::PathBuf;
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use ton_executor::BlockchainConfig;

pub struct TonNodeServiceConfig {
    pub node: NodeConfig,
    pub blockchain: BlockchainConfig,
}

pub struct TonNodeService {
    config: TonNodeServiceConfig,
    node: Arc<TonNodeEngine>,
}

impl TonNodeService {
    pub fn new(config: TonNodeServiceConfig) -> NodeResult<Self> {
        let storage = Arc::new(FSStorage::new(PathBuf::from("./"))?);
        let node = Arc::new(TonNodeEngine::with_params(
            config.node.global_id,
            config.node.shard_id_config().shard_ident(),
            Arc::new(config.blockchain.clone()),
            Arc::new(ArangoHelper::from_config(
                &config.node.document_db_config(),
            )?),
            storage,
            false,
        )?);
        Ok(Self { config, node })
    }

    pub fn run(&self) {
        let mut router = Router::new();
        MessageReceiverApi::add_route(
            &mut router,
            self.config.node.api.messages.clone(),
            self.node.clone(),
        );
        ControlApi::add_route(
            &mut router,
            self.config.node.api.live_control.clone(),
            self.node.clone(),
        );
        self.node.clone().start().unwrap();
        let addr = format!(
            "{}:{}",
            self.config.node.api.address, self.config.node.api.port
        );

        thread::spawn(move || {
            Iron::new(router).http(addr).expect("error starting api");
        });

        loop {
            thread::sleep(Duration::from_secs(1));
        }
    }
}
