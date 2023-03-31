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

use crate::data::DocumentsDb;
use crate::data::NodeStorage;
use crate::engine::masterchain::Masterchain;
use crate::engine::messages::InMessagesQueue;
use crate::engine::shardchain::Shardchain;
use crate::engine::time::BlockTime;
use crate::error::NodeResult;
use parking_lot::RwLock;
use std::{sync::Arc, thread};
use ton_block::ShardIdent;
use ton_executor::BlockchainConfig;

/// It is top level struct provided node functionality related to transactions processing.
/// Initialises instances of: all messages receivers, InMessagesQueue, MessagesProcessor.
pub struct TonNodeEngine {
    pub time: RwLock<BlockTime>,
    pub message_queue: Arc<InMessagesQueue>,
    pub masterchain: Masterchain,
    pub workchain: Shardchain,
}

impl TonNodeEngine {
    /// Construct new engine for selected shard
    /// with given time to generate block candidate
    pub fn with_params(
        global_id: i32,
        workchain_shard: ShardIdent,
        blockchain_config: Arc<BlockchainConfig>,
        documents_db: Arc<dyn DocumentsDb>,
        storage: Arc<dyn NodeStorage>,
    ) -> NodeResult<Self> {
        let message_queue = Arc::new(InMessagesQueue::new(10000));
        let masterchain = Masterchain::with_params(
            global_id,
            blockchain_config.clone(),
            message_queue.clone(),
            documents_db.clone(),
            &*storage,
        )?;
        let workchain = Shardchain::with_params(
            workchain_shard,
            global_id,
            blockchain_config.clone(),
            message_queue.clone(),
            documents_db.clone(),
            &*storage,
        )?;
        Ok(TonNodeEngine {
            time: RwLock::new(BlockTime::new()),
            message_queue: message_queue.clone(),
            masterchain,
            workchain,
        })
    }

    pub fn start(self: Arc<Self>) -> NodeResult<()> {
        if self.workchain.finality_was_loaded {
            self.masterchain.restore_state()?;
        }
        thread::spawn(move || loop {
            if let Err(err) = self.process_messages(true) {
                log::error!(target: "node", "failed block generation: {}", err);
            }
            self.message_queue.wait_new_message();
        });
        Ok(())
    }

    pub(crate) fn process_messages(&self, debug: bool) -> NodeResult<()> {
        let mut continue_processing = true;
        let mut gen_time = self.get_next_time();
        while continue_processing {
            continue_processing = false;
            while let Some(block) = self.workchain.generate_block(gen_time, debug)? {
                self.set_last_time(gen_time);
                self.masterchain.register_new_shard_block(&block)?;
                continue_processing = true;
                gen_time = self.get_next_time();
            }
            if self.masterchain.generate_block(gen_time, debug)?.is_some() {
                self.set_last_time(gen_time);
                continue_processing = true;
            }
        }
        Ok(())
    }

    pub fn get_next_time(&self) -> u32 {
        self.time.read().get_next()
    }

    fn set_last_time(&self, time: u32) {
        self.time.write().set_last(time)
    }
}
