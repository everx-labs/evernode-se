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

#[cfg(test)]
use crate::config::NodeConfig;
use crate::data::DocumentsDb;
use crate::engine::masterchain::Masterchain;
use crate::engine::shardchain::Shardchain;
use crate::engine::InMessagesQueue;
use crate::error::NodeResult;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::{
    sync::{
        atomic::{AtomicU32, Ordering},
        Arc,
    },
    thread,
};
use ton_block::ShardIdent;
use ton_executor::BlockchainConfig;

/// It is top level struct provided node functionality related to transactions processing.
/// Initialises instances of: all messages receivers, InMessagesQueue, MessagesProcessor.
pub struct TonNodeEngine {
    time_delta: AtomicU32,
    seq_mode: AtomicBool,
    pub message_queue: Arc<InMessagesQueue>,
    pub masterchain: Masterchain,
    pub workchain: Shardchain,
}

impl TonNodeEngine {
    pub fn increase_time(&self, delta: u32) {
        let new_time_delta = self.time_delta() + delta;
        self.time_delta.store(new_time_delta, Ordering::Relaxed);
        log::info!(target: "node", "SE time delta set to {}", new_time_delta);
    }

    pub fn reset_time(&self) {
        self.time_delta.store(0, Ordering::Relaxed);
        log::info!(target: "node", "SE time delta set to 0");
    }

    pub fn time_delta(&self) -> u32 {
        self.time_delta.load(Ordering::Relaxed)
    }

    pub fn seq_mode(&self) -> bool {
        self.seq_mode.load(Ordering::Relaxed)
    }

    pub fn set_seq_mode(&self, mode: bool) {
        self.seq_mode.store(mode, Ordering::Relaxed);
        log::info!(target: "node", "SE seq mode to {}", mode);
    }

    pub fn start(self: Arc<Self>) -> NodeResult<()> {
        if self.workchain.finality_was_loaded {
            self.masterchain.restore_state()?;
        }

        thread::spawn(move || loop {
            if let Err(err) = self.generate_blocks(true) {
                log::error!(target: "node", "failed block generation: {}", err);
            }
        });
        Ok(())
    }

    /// Construct new engine for selected shard
    /// with given time to generate block candidate
    pub fn with_params(
        global_id: i32,
        workchain_shard: ShardIdent,
        blockchain_config: Arc<BlockchainConfig>,
        documents_db: Arc<dyn DocumentsDb>,
        storage_path: PathBuf,
    ) -> NodeResult<Self> {
        let message_queue = Arc::new(InMessagesQueue::new(10000));
        Ok(TonNodeEngine {
            time_delta: AtomicU32::new(0),
            seq_mode: AtomicBool::new(false),
            message_queue: message_queue.clone(),
            masterchain: Masterchain::with_params(
                global_id,
                blockchain_config.clone(),
                message_queue.clone(),
                documents_db.clone(),
                storage_path.clone(),
            )?,
            workchain: Shardchain::with_params(
                workchain_shard,
                global_id,
                blockchain_config.clone(),
                message_queue.clone(),
                documents_db.clone(),
                storage_path.clone(),
            )?,
        })
    }

    pub(crate) fn generate_blocks(&self, debug: bool) -> NodeResult<()> {
        self.process_messages(debug)?;
        self.message_queue.wait_new_message();
        Ok(())
    }

    pub(crate) fn process_messages(&self, debug: bool) -> NodeResult<()> {
        let mut continue_processing = true;
        while continue_processing {
            continue_processing = false;
            let time_delta = self.time_delta();
            while let Some(block) = self.workchain.generate_block(time_delta, debug)? {
                self.masterchain.register_new_shard_block(&block)?;
                continue_processing = true;
            }
            if self
                .masterchain
                .generate_block(time_delta, debug)?
                .is_some()
            {
                continue_processing = true;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
pub fn get_config_params(json: &str) -> (NodeConfig, Vec<ed25519_dalek::PublicKey>) {
    match NodeConfig::parse(json) {
        Ok(config) => match config.import_keys() {
            Ok(keys) => (config, keys),
            Err(err) => {
                log::warn!(target: "node", "{}", err);
                panic!("{} / {}", err, json)
            }
        },
        Err(err) => {
            log::warn!(target: "node", "Error parsing configuration file. {}", err);
            panic!("Error parsing configuration file. {}", err)
        }
    }
}
