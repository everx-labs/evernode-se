mod control_api;
mod engine_manager;
mod message_api;

use crate::config::{NodeConfig, NodeApiConfig};
use crate::error::NodeResult;
use crate::service::control_api::ControlApi;
use crate::service::message_api::MessageReceiverApi;
use iron::Iron;
use router::Router;
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use ever_executor::BlockchainConfig;

use self::engine_manager::TonNodeEngineManager;

#[derive(Clone)]
pub struct TonNodeServiceConfig {
    pub node: NodeConfig,
    pub blockchain: BlockchainConfig,
}

pub struct TonNodeService {
    api_config: NodeApiConfig,
    node_manager: Arc<TonNodeEngineManager>,
}

impl TonNodeService {
    pub fn new(config: TonNodeServiceConfig) -> NodeResult<Self> {
        Ok(Self {
            api_config: config.node.api.clone(),
            node_manager: Arc::new(TonNodeEngineManager::new(config)?),
        })
    }

    pub fn run(&self) {
        let mut router = Router::new();
        MessageReceiverApi::add_route(
            &mut router,
            self.api_config.messages.clone(),
            self.node_manager.clone(),
        );
        ControlApi::add_route(
            &mut router,
            self.api_config.live_control.clone(),
            self.node_manager.clone(),
        );
        self.node_manager.clone().start();
        let addr = format!(
            "{}:{}",
            self.api_config.address, self.api_config.port
        );

        thread::spawn(move || {
            Iron::new(router).http(addr).expect("error starting api");
        });

        loop {
            thread::sleep(Duration::from_secs(1));
        }
    }
}
