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

use crate::block::finality::MINTER_ADDRESS;
#[cfg(test)]
use crate::config::NodeConfig;
use crate::data::DocumentsDb;
use crate::engine::masterchain::Masterchain;
use crate::engine::shardchain::Shardchain;
use crate::engine::{
    InMessagesQueue, LiveControl, LiveControlReceiver, DEPRECATED_GIVER_ABI2_DEPLOY_MSG,
    GIVER_ABI1_DEPLOY_MSG, GIVER_ABI2_DEPLOY_MSG, GIVER_BALANCE, MULTISIG_BALANCE,
    MULTISIG_DEPLOY_MSG,
};
use crate::error::NodeResult;
use crate::MessagesReceiver;
use parking_lot::Mutex;
use std::path::PathBuf;
use std::{
    sync::{
        atomic::{AtomicU32, Ordering},
        Arc,
    },
    thread,
    time::Duration,
};
use ton_block::{
    CommonMsgInfo, CurrencyCollection, Deserializable, Grams, InternalMessageHeader, Message,
    MsgAddressInt, ShardIdent, UnixTime32,
};
use ton_executor::BlockchainConfig;
use ton_types::AccountId;

#[cfg(test)]
#[path = "../../../../tonos-se-tests/unit/test_ton_node_engine.rs"]
mod tests;

pub struct EngineLiveProperties {
    pub time_delta: Arc<AtomicU32>,
}

impl EngineLiveProperties {
    fn new() -> Self {
        Self {
            time_delta: Arc::new(AtomicU32::new(0)),
        }
    }

    fn get_time_delta(&self) -> u32 {
        self.time_delta.load(Ordering::Relaxed)
    }

    fn set_time_delta(&self, value: u32) {
        self.time_delta.store(value, Ordering::Relaxed)
    }

    fn increment_time(&self, delta: u32) {
        self.set_time_delta(self.get_time_delta() + delta)
    }
}

struct EngineLiveControl {
    properties: Arc<EngineLiveProperties>,
}

impl EngineLiveControl {
    fn new(properties: Arc<EngineLiveProperties>) -> Self {
        Self { properties }
    }
}

impl LiveControl for EngineLiveControl {
    fn increase_time(&self, delta: u32) -> NodeResult<()> {
        self.properties.increment_time(delta);
        log::info!(target: "node", "SE time delta set to {}", self.properties.get_time_delta());
        Ok(())
    }

    fn reset_time(&self) -> NodeResult<()> {
        self.properties.set_time_delta(0);
        log::info!(target: "node", "SE time delta set to 0");
        Ok(())
    }

    fn time_delta(&self) -> NodeResult<u32> {
        Ok(self.properties.get_time_delta())
    }
}

/// It is top level struct provided node functionality related to transactions processing.
/// Initialises instances of: all messages receivers, InMessagesQueue, MessagesProcessor.
pub struct TonNodeEngine {
    live_properties: Arc<EngineLiveProperties>,
    receivers: Vec<Mutex<Box<dyn MessagesReceiver>>>,
    live_control_receiver: Option<Box<dyn LiveControlReceiver>>,
    pub message_queue: Arc<InMessagesQueue>,
    pub masterchain: Masterchain,
    pub workchain: Shardchain,
}

impl TonNodeEngine {
    pub fn start(self: Arc<Self>) -> NodeResult<()> {
        for recv in self.receivers.iter() {
            recv.lock().run(Arc::clone(&self.message_queue))?;
        }

        let live_control = EngineLiveControl::new(self.live_properties.clone());
        if let Some(ref control_receiver) = self.live_control_receiver {
            control_receiver.run(Box::new(live_control))?;
        }

        if !self.workchain.finality_was_loaded {
            self.initialize_blockchain()?;
        }
        thread::spawn(move || loop {
            if let Err(err) = self.generate_blocks(true) {
                log::error!(target: "node", "failed block generation: {}", err);
            }
        });
        Ok(())
    }

    #[cfg(test)]
    pub fn stop(self: Arc<Self>) -> NodeResult<()> {
        log::info!(target: "node","TONNodeEngine stopped.");
        Ok(())
    }

    /// Construct new engine for selected shard
    /// with given time to generate block candidate
    pub fn with_params(
        global_id: i32,
        workchain_shard: ShardIdent,
        receivers: Vec<Box<dyn MessagesReceiver>>,
        live_control_receiver: Option<Box<dyn LiveControlReceiver>>,
        blockchain_config: Arc<BlockchainConfig>,
        documents_db: Arc<dyn DocumentsDb>,
        storage_path: PathBuf,
    ) -> NodeResult<Self> {
        let message_queue = Arc::new(InMessagesQueue::with_db(10000, documents_db.clone()));
        let live_properties = Arc::new(EngineLiveProperties::new());

        let receivers = receivers
            .into_iter()
            .map(|r| Mutex::new(r))
            .collect::<Vec<_>>();
        Ok(TonNodeEngine {
            message_queue: message_queue.clone(),
            receivers,
            live_properties,
            live_control_receiver,
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

    fn generate_blocks(&self, debug: bool) -> NodeResult<()> {
        let mut continue_generating = true;
        while continue_generating {
            continue_generating = false;
            let time_delta = self.live_properties.get_time_delta();
            while let Some(block) = self.workchain.generate_block(time_delta, debug)? {
                self.masterchain.register_new_shard_block(&block)?;
                continue_generating = true;
            }
            if self
                .masterchain
                .generate_block(time_delta, debug)?
                .is_some()
            {
                continue_generating = true;
            }
        }
        self.message_queue.wait_new_message();
        Ok(())
    }
}

impl TonNodeEngine {
    fn initialize_blockchain(&self) -> NodeResult<()> {
        log::info!(target: "node", "Initialize blockchain");
        let workchain_id = self.workchain.shard_ident.workchain_id() as i8;
        self.enqueue_deploy_message(workchain_id, GIVER_ABI1_DEPLOY_MSG, GIVER_BALANCE, 1)?;
        self.enqueue_deploy_message(workchain_id, GIVER_ABI2_DEPLOY_MSG, GIVER_BALANCE, 3)?;
        self.enqueue_deploy_message(workchain_id, MULTISIG_DEPLOY_MSG, MULTISIG_BALANCE, 5)?;
        self.enqueue_deploy_message(
            workchain_id,
            DEPRECATED_GIVER_ABI2_DEPLOY_MSG,
            GIVER_BALANCE,
            7,
        )?;

        Ok(())
    }

    fn enqueue_deploy_message(
        &self,
        workchain_id: i8,
        deploy_msg_boc: &[u8],
        initial_balance: u128,
        transfer_lt: u64,
    ) -> NodeResult<AccountId> {
        let (deploy_msg, deploy_addr) =
            Self::create_contract_deploy_message(workchain_id, deploy_msg_boc);
        let transfer_msg = Self::create_transfer_message(
            workchain_id,
            MINTER_ADDRESS.address(),
            deploy_addr.clone(),
            initial_balance,
            transfer_lt,
        );
        self.queue_with_retry(transfer_msg)?;
        self.queue_with_retry(deploy_msg)?;

        Ok(deploy_addr)
    }

    fn queue_with_retry(&self, message: Message) -> NodeResult<()> {
        let mut message = message;
        while let Err(msg) = self.message_queue.queue(message) {
            message = msg;
            thread::sleep(Duration::from_micros(100));
        }

        Ok(())
    }

    fn create_contract_deploy_message(workchain_id: i8, msg_boc: &[u8]) -> (Message, AccountId) {
        let mut msg = Message::construct_from_bytes(msg_boc).unwrap();
        if let CommonMsgInfo::ExtInMsgInfo(ref mut header) = msg.header_mut() {
            match header.dst {
                MsgAddressInt::AddrStd(ref mut addr) => addr.workchain_id = workchain_id,
                _ => panic!("Contract deploy message has invalid destination address"),
            }
        }

        let address = msg.int_dst_account_id().unwrap();

        (msg, address)
    }

    // create transfer funds message for initialize balance
    pub fn create_transfer_message(
        workchain_id: i8,
        src: AccountId,
        dst: AccountId,
        value: u128,
        lt: u64,
    ) -> Message {
        let hdr = Self::create_transfer_int_header(workchain_id, src, dst, value);
        let mut msg = Message::with_int_header(hdr);

        msg.set_at_and_lt(UnixTime32::now().as_u32(), lt);
        msg
    }

    pub fn create_transfer_int_header(
        workchain_id: i8,
        src: AccountId,
        dst: AccountId,
        value: u128,
    ) -> InternalMessageHeader {
        InternalMessageHeader::with_addresses_and_bounce(
            MsgAddressInt::with_standart(None, workchain_id, src).unwrap(),
            MsgAddressInt::with_standart(None, workchain_id, dst).unwrap(),
            CurrencyCollection::from_grams(Grams::new(value).unwrap()),
            false,
        )
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
