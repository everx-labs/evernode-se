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
use crate::block::{BlockBuilder, BlockFinality, NewBlockApplier, OrdinaryBlockFinality};
#[cfg(test)]
use crate::config::NodeConfig;
use crate::data::{DocumentsDb, DocumentsDbMock, FileBasedStorage};
use crate::engine::{
    InMessagesQueue, LiveControl, LiveControlReceiver, QueuedMessage,
    DEPRECATED_GIVER_ABI2_DEPLOY_MSG, GIVER_ABI1_DEPLOY_MSG, GIVER_ABI2_DEPLOY_MSG, GIVER_BALANCE,
    MULTISIG_BALANCE, MULTISIG_DEPLOY_MSG,
};
use crate::error::{NodeError, NodeResult};
use crate::MessagesReceiver;
use parking_lot::Mutex;
use std::path::PathBuf;
use std::{
    io::ErrorKind,
    sync::{
        atomic::{AtomicU32, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};
use ton_block::{
    Block, CommonMsgInfo, CurrencyCollection, Deserializable, Grams, InternalMessageHeader,
    Message, MsgAddressInt, ShardIdent, ShardStateUnsplit, UnixTime32,
};
use ton_executor::BlockchainConfig;
use ton_types::{AccountId, HashmapType};

#[cfg(test)]
#[path = "../../../../tonos-se-tests/unit/test_ton_node_engine.rs"]
mod tests;

type Storage = FileBasedStorage;
type ArcBlockFinality = Arc<Mutex<OrdinaryBlockFinality<Storage, Storage, Storage, Storage>>>;
type BlockApplier =
    Mutex<NewBlockApplier<OrdinaryBlockFinality<Storage, Storage, Storage, Storage>>>;

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
    shard_ident: ShardIdent,
    live_properties: Arc<EngineLiveProperties>,
    receivers: Vec<Mutex<Box<dyn MessagesReceiver>>>,
    live_control_receiver: Option<Box<dyn LiveControlReceiver>>,
    pub blockchain_config: BlockchainConfig,
    pub finalizer: ArcBlockFinality,
    pub block_applier: BlockApplier,
    pub message_queue: Arc<InMessagesQueue>,
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

        if self.finalizer.lock().get_last_seq_no() == 1 {
            let workchain_id = self.current_shard_id().workchain_id() as i8;
            self.deploy_contracts(workchain_id)?;
        }
        thread::spawn(move || loop {
            if let Err(err) = self.prepare_block(true) {
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
        shard: ShardIdent,
        receivers: Vec<Box<dyn MessagesReceiver>>,
        live_control_receiver: Option<Box<dyn LiveControlReceiver>>,
        blockchain_config: BlockchainConfig,
        documents_db: Option<Arc<dyn DocumentsDb>>,
        storage_path: PathBuf,
    ) -> NodeResult<Self> {
        let documents_db = documents_db.unwrap_or_else(|| Arc::new(DocumentsDbMock));
        let message_queue = Arc::new(InMessagesQueue::with_db(10000, documents_db.clone()));

        let storage = Arc::new(Storage::with_path(shard.clone(), storage_path.clone())?);
        let block_finality = Arc::new(Mutex::new(OrdinaryBlockFinality::with_params(
            shard.clone(),
            storage_path,
            storage.clone(),
            storage.clone(),
            storage.clone(),
            storage.clone(),
            Some(documents_db.clone()),
            Vec::new(),
        )));
        match block_finality.lock().load() {
            Ok(_) => {
                log::info!(target: "node", "load block finality successfully");
            }
            Err(NodeError::Io(err)) => {
                if err.kind() != ErrorKind::NotFound {
                    return Err(NodeError::Io(err));
                }
            }
            Err(err) => {
                return Err(err);
            }
        }

        let live_properties = Arc::new(EngineLiveProperties::new());

        let receivers = receivers
            .into_iter()
            .map(|r| Mutex::new(r))
            .collect::<Vec<_>>();

        Ok(TonNodeEngine {
            shard_ident: shard.clone(),
            receivers,
            live_properties,
            live_control_receiver,
            blockchain_config,
            finalizer: block_finality.clone(),
            block_applier: Mutex::new(NewBlockApplier::with_params(block_finality, documents_db)),
            message_queue,
        })
    }

    /// Getter for current shard identifier
    pub fn current_shard_id(&self) -> &ShardIdent {
        &self.shard_ident
    }

    fn print_block_info(block: &Block) {
        let extra = block.read_extra().unwrap();
        log::info!(target: "node",
            "block: gen time = {}, in msg count = {}, out msg count = {}, account_blocks = {}",
            block.read_info().unwrap().gen_utime(),
            extra.read_in_msg_descr().unwrap().len().unwrap(),
            extra.read_out_msg_descr().unwrap().len().unwrap(),
            extra.read_account_blocks().unwrap().len().unwrap());
    }

    ///
    /// Generate new block if possible
    ///
    pub fn prepare_block(&self, debug: bool) -> NodeResult<()> {
        let shard_state = self.finalizer.lock().get_last_shard_state();
        let out_msg_queue_info = shard_state.read_out_msg_queue_info()?;
        if out_msg_queue_info.out_queue().is_empty() && self.message_queue.is_empty() {
            self.message_queue.wait_new_message();
        }

        let timestamp = UnixTime32::now().as_u32() + self.live_properties.get_time_delta();
        let blk_prev_info = self.finalizer.lock().get_last_block_info()?;
        log::debug!(target: "node", "PARENT block: {:?}", blk_prev_info);

        let collator = BlockBuilder::with_params(shard_state, blk_prev_info, timestamp)?;
        match collator.build_block(&self.message_queue, self.blockchain_config.clone(), debug)? {
            Some((block, new_shard_state)) => {
                log::trace!(target: "node", "block generated successfully");
                // TODO remove debug print
                Self::print_block_info(&block);
                self.finality_and_apply_block(&block, new_shard_state)?;
            }
            None => {
                log::trace!(target: "node", "empty block was not generated");
            }
        }
        Ok(())
    }

    /// finality and apply block
    fn finality_and_apply_block(
        &self,
        block: &Block,
        applied_shard: ShardStateUnsplit,
    ) -> NodeResult<(Arc<ShardStateUnsplit>, Vec<u128>)> {
        let mut time = Vec::new();
        let now = Instant::now();
        let new_state = self.block_applier.lock().apply(block, applied_shard)?;
        time.push(now.elapsed().as_micros());
        Ok((new_state, time))
    }
}

impl TonNodeEngine {
    fn deploy_contracts(&self, workchain_id: i8) -> NodeResult<()> {
        self.deploy_contract(workchain_id, GIVER_ABI1_DEPLOY_MSG, GIVER_BALANCE, 1)?;
        self.deploy_contract(workchain_id, GIVER_ABI2_DEPLOY_MSG, GIVER_BALANCE, 3)?;
        self.deploy_contract(workchain_id, MULTISIG_DEPLOY_MSG, MULTISIG_BALANCE, 5)?;
        self.deploy_contract(
            workchain_id,
            DEPRECATED_GIVER_ABI2_DEPLOY_MSG,
            GIVER_BALANCE,
            7,
        )?;

        Ok(())
    }

    fn deploy_contract(
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
        let mut message = QueuedMessage::with_message(message)?;
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
