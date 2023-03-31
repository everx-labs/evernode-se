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
use crate::engine::BlockTimeMode;
use crate::error::{NodeError, NodeResult};
use parking_lot::RwLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::sleep;
use std::time::Duration;
use std::{sync::Arc, thread};
use ton_block::{Message, ShardIdent};
use ton_executor::BlockchainConfig;

/// It is top level struct provided node functionality related to transactions processing.
/// Initialises instances of: all messages receivers, InMessagesQueue, MessagesProcessor.
pub struct TonNodeEngine {
    time: RwLock<BlockTime>,
    is_running_in_background: AtomicBool,
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
        debug_mode: bool,
    ) -> NodeResult<Self> {
        let message_queue = Arc::new(InMessagesQueue::new(10000));
        let masterchain = Masterchain::with_params(
            global_id,
            blockchain_config.clone(),
            message_queue.clone(),
            documents_db.clone(),
            &*storage,
            debug_mode,
        )?;
        let workchain = Shardchain::with_params(
            workchain_shard,
            global_id,
            blockchain_config.clone(),
            message_queue.clone(),
            documents_db.clone(),
            &*storage,
            debug_mode,
        )?;
        if workchain.finality_was_loaded {
            masterchain.restore_state()?;
        }
        let node = TonNodeEngine {
            time: RwLock::new(BlockTime::new()),
            message_queue: message_queue.clone(),
            masterchain,
            workchain,
            is_running_in_background: AtomicBool::new(false),
        };
        node.execute_queued_messages()?;
        Ok(node)
    }

    pub fn start(self: Arc<Self>) -> NodeResult<()> {
        self.is_running_in_background.store(true, Ordering::Relaxed);
        thread::spawn(move || loop {
            if let Err(err) = self.execute_queued_messages() {
                log::error!(target: "node", "failed block generation: {}", err);
            }
            if self.message_queues_are_empty() {
                self.message_queue.wait_new_message();
            }
        });
        Ok(())
    }

    fn message_queues_are_empty(&self) -> bool {
        self.workchain.out_message_queue_is_empty()
            && self.masterchain.out_message_queue_is_empty()
            && self.message_queue.is_empty()
    }

    fn execute_queued_messages(&self) -> NodeResult<()> {
        let mut continue_processing = true;
        let mut gen_time = self.get_next_time();
        while continue_processing {
            continue_processing = false;
            while let Some(block) = self.workchain.generate_block(gen_time, self.time_mode())? {
                self.set_last_time(gen_time);
                self.masterchain.register_new_shard_block(&block)?;
                continue_processing = true;
                gen_time = self.get_next_time();
            }
            if self
                .masterchain
                .generate_block(gen_time, self.time_mode())?
                .is_some()
            {
                self.set_last_time(gen_time);
                continue_processing = true;
            }
        }
        Ok(())
    }

    pub fn process_messages(&self) -> NodeResult<()> {
        if self.is_running_in_background.load(Ordering::Relaxed) {
            while !self.message_queues_are_empty() {
                sleep(Duration::from_millis(100));
            }
            Ok(())
        } else {
            self.execute_queued_messages()
        }
    }

    pub fn enqueue_message(&self, msg: Message) -> NodeResult<()> {
        self.message_queue
            .queue(msg)
            .map_err(|_| NodeError::QueueFull)
    }

    pub fn process_message(&self, msg: Message) -> NodeResult<()> {
        self.enqueue_message(msg)?;
        self.process_messages()
    }

    pub fn set_time_mode(&self, mode: BlockTimeMode) {
        self.time.write().set_mode(mode);
    }

    pub fn increase_time_delta(&self, delta: u32) {
        self.time.write().increase_delta(delta);
    }

    pub fn reset_time_delta(&self) {
        self.time.write().reset_delta();
    }

    pub fn time_mode(&self) -> BlockTimeMode {
        self.time.read().mode
    }

    pub fn time_delta(&self) -> u32 {
        self.time.read().delta
    }

    pub fn get_next_time(&self) -> u32 {
        self.time.read().get_next()
    }

    fn set_last_time(&self, time: u32) {
        self.time.write().set_last(time)
    }
}
