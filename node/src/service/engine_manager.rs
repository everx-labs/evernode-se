use crate::config::ForkConfig;
use crate::data::{ArangoHelper, ExternalAccountsProvider, FSStorage, ForkProvider};
use crate::engine::engine::TonNodeEngine;
use crate::error::NodeResult;
use parking_lot::RwLock;
use ever_block::ShardIdent;
use std::path::PathBuf;
use std::sync::Arc;
use std::thread::JoinHandle;

use super::TonNodeServiceConfig;

pub struct TonNodeEngineManager {
    original_config: TonNodeServiceConfig,
    node: RwLock<(Arc<TonNodeEngine>, Option<JoinHandle<()>>)>,
}

impl TonNodeEngineManager {
    pub fn new(config: TonNodeServiceConfig) -> NodeResult<Self> {
        Ok(Self {
            node: RwLock::new((Self::create_engine(&config)?, None)),
            original_config: config,
        })
    }

    pub fn node(&self) -> Arc<TonNodeEngine> {
        self.node.read().0.clone()
    }

    fn create_engine(config: &TonNodeServiceConfig) -> NodeResult<Arc<TonNodeEngine>> {
        let storage = Arc::new(FSStorage::new(PathBuf::from("./"))?);
        let ((blockchain_config, global_id), account_provider) =
            if let Some(fork) = &config.node.fork {
                let provider = ForkProvider::new(fork.clone());
                (
                    provider.get_blockchain_config()?,
                    Some(Arc::new(provider) as Arc<(dyn ExternalAccountsProvider)>),
                )
            } else {
                ((config.blockchain.clone(), config.node.global_id), None)
            };
        Ok(Arc::new(TonNodeEngine::with_params(
            global_id,
            config.node.shard_id_config().shard_ident(),
            Arc::new(blockchain_config),
            Arc::new(ArangoHelper::from_config(
                &config.node.document_db_config(),
            )?),
            storage,
            account_provider,
        )?))
    }

    pub fn start(&self) {
        let mut lock = self.node.write();
        let node = lock.0.clone();
        lock.1 = Some(std::thread::spawn(move || node.run()));
    }

    fn stop(&self) {
        let mut lock = self.node.write();
        lock.0.message_queue.stop();
        lock.1.take().map(|thread| thread.join());
    }

    fn clear_db(&self) -> NodeResult<()> {
        let arango = ArangoHelper::from_config(
            &self.original_config.node.document_db_config(),
        )?;
        arango.clear_db()?;

        let fs_storage = FSStorage::new(PathBuf::from("./"))?;
        fs_storage.clear_db(self.original_config.node.shard_id_config().shard_ident(),)?;
        fs_storage.clear_db(ShardIdent::masterchain())?;

        Ok(())
    }

    pub fn reset(&self) -> NodeResult<()> {
        self.stop();
        self.clear_db()?;
        *self.node.write() = (Self::create_engine(&self.original_config)?, None);
        self.start();
        Ok(())
    }

    pub fn fork(&self, endpoint: String, auth: Option<String>, reset: bool) -> NodeResult<()> {
        self.stop();
        if reset {
            self.clear_db()?;
        }
        let mut config = self.original_config.clone();
        config.node.fork = Some(ForkConfig {
            endpoint,
            auth,
        });
        *self.node.write() = (Self::create_engine(&config)?, None);
        self.start();
        Ok(())
    }

    pub fn unfork(&self, reset: bool) -> NodeResult<()> {
        self.stop();
        if reset {
            self.clear_db()?;
        }
        *self.node.write() = (Self::create_engine(&self.original_config)?, None);
        self.start();
        Ok(())
    }
}
