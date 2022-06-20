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

use super::*;

use crate::error::NodeError;
use jsonrpc_http_server::jsonrpc_core::types::params::Params;
use jsonrpc_http_server::jsonrpc_core::types::Value;
use jsonrpc_http_server::jsonrpc_core::{Error, IoHandler};
use jsonrpc_http_server::{AccessControlAllowOrigin, DomainsValidation, Server, ServerBuilder};
use parking_lot::Mutex;
use std::collections::BTreeSet;
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering as AtomicOrdering},
    Arc,
};
use std::thread;
use std::time::{Duration, Instant};
use std::{cmp::Ordering, collections::HashSet};
use threadpool::ThreadPool;
use ton_block::{
    AddSub, BlkPrevInfo, ComputeSkipReason, Deserializable, Grams, HashUpdate, InMsg, Message,
    MsgEnvelope, OutMsg, OutMsgQueueKey, ShardAccount, ShardStateUnsplit, TrComputePhase,
    TrComputePhaseVm, TransactionDescr, TransactionDescrOrdinary,
};
use ton_executor::{
    BlockchainConfig, ExecuteParams, ExecutorError, OrdinaryTransactionExecutor,
    TransactionExecutor,
};
use ton_types::{
    serialize_toc, AccountId, BuilderData, HashmapE, HashmapRemover, IBitstring, Result, SliceData,
};
use crate::block::{AppendSerializedContext, BlockBuilder};
use crate::data::{DocumentsDb, TransactionsStorage};

#[cfg(test)]
#[path = "../../../../tonos-se-tests/unit/test_messages.rs"]
mod tests;

// TODO: I think that 'static - is a bad practice. If you know how to do it without static - please help
pub struct MessagesProcessor<T>
where
    T: TransactionsStorage + Send + Sync + 'static,
{
    tr_storage: Arc<T>,
    queue: Arc<InMessagesQueue>,
    shard_id: ShardIdent,
    blockchain_config: BlockchainConfig,
    executors: Arc<Mutex<HashMap<AccountId, Arc<Mutex<OrdinaryTransactionExecutor>>>>>,
}

impl<T> MessagesProcessor<T>
where
    T: TransactionsStorage + Send + Sync + 'static,
{
    pub fn with_params(
        queue: Arc<InMessagesQueue>,
        tr_storage: Arc<T>,
        shard_id: ShardIdent,
        blockchain_config: BlockchainConfig,
    ) -> Self {
        // make clone for changes
        //let shard_state_new = shard_state.lock().unwrap().clone();

        Self {
            tr_storage,
            queue,
            shard_id,
            blockchain_config,
            executors: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// loop-back message to InQueue or send to OutMsgQueue of shard
    fn route_out_messages(
        shard: &ShardIdent,
        queue: Arc<InMessagesQueue>,
        transaction: Arc<Transaction>,
        shard_state_new: Arc<Mutex<ShardStateUnsplit>>,
    ) -> NodeResult<()> {
        let queue = &mut queue.clone();
        transaction.iterate_out_msgs(|msg| {
            // if message destination address belongs current shard
            // put it to in queue
            // unwrap is safe, because transaction can generate only
            // internal and ExternalOutboundMessage
            if msg.is_internal() {
                if shard.contains_address(&msg.dst().unwrap())? {
                    queue
                        .priority_queue(QueuedMessage::with_message(msg)?)
                        .map_err(|_| failure::format_err!("Error priority queue message"))?;
                } else {
                    // let out_msg = OutMsg::New(
                    //     OutMsgNew::with_params(
                    //         &MsgEnvelope::with_message_and_fee(      // TODO need understand how set addresses for Envelop
                    //             &msg,
                    //             10u32.into()                    // TODO need understand where take fee value
                    //         )?,
                    //         &transaction
                    //     )?
                    // );
                    let out_msg = MsgEnvelope::with_message_and_fee(
                        // TODO need understand how set addresses for Envelop
                        &msg,
                        10u64.into(), // TODO need understand where take fee value
                    )?;
                    let address = OutMsgQueueKey::first_u64(transaction.account_id());
                    let mut shard_state_new = shard_state_new.lock();
                    let mut out_msg_queue_info = shard_state_new.read_out_msg_queue_info()?;
                    out_msg_queue_info.out_queue_mut().insert(
                        shard.workchain_id(),
                        address,
                        &out_msg,
                        msg.lt().unwrap(),
                    )?;
                    shard_state_new.write_out_msg_queue_info(&out_msg_queue_info)?;
                }
            }
            Ok(true)
        })?;
        Ok(())
    }

    fn try_prepare_transaction(
        builder: &BlockBuilder,
        executor: &OrdinaryTransactionExecutor,
        acc_root: &mut Cell,
        msg: &Message,
        acc_last_lt: u64,
        debug: bool,
    ) -> NodeResult<(Transaction, u64)> {
        let (block_at, block_lt) = builder.at_and_lt();
        let last_lt = std::cmp::max(acc_last_lt, block_lt);
        let lt = Arc::new(AtomicU64::new(last_lt + 1));
        let result = executor.execute_with_libs_and_params(
            Some(&msg),
            acc_root,
            ExecuteParams {
                state_libs: HashmapE::default(),
                block_unixtime: block_at,
                block_lt,
                last_tr_lt: Arc::clone(&lt),
                seed_block: UInt256::rand(), // we don't need strict randomization like real node
                debug,
                ..Default::default()
            },
        );
        match result {
            Ok(transaction) => Ok((transaction, lt.load(AtomicOrdering::Relaxed))),
            Err(err) => {
                let lt = last_lt + 1;
                let account = Account::construct_from_cell(acc_root.clone())?;
                let mut transaction = Transaction::with_account_and_message(&account, msg, lt)?;
                transaction.set_now(block_at);
                let mut description = TransactionDescrOrdinary::default();
                description.aborted = true;
                match err.downcast_ref::<ExecutorError>() {
                    Some(ExecutorError::NoAcceptError(error, arg)) => {
                        let mut vm_phase = TrComputePhaseVm::default();
                        vm_phase.success = false;
                        vm_phase.exit_code = *error;
                        if let Some(item) = arg {
                            vm_phase.exit_arg = match item
                                .as_integer()
                                .and_then(|value| value.into(std::i32::MIN..=std::i32::MAX))
                            {
                                Err(_) | Ok(0) => None,
                                Ok(exit_arg) => Some(exit_arg),
                            };
                        }
                        description.compute_ph = TrComputePhase::Vm(vm_phase);
                    }
                    Some(ExecutorError::NoFundsToImportMsg) => {
                        description.compute_ph = if account.is_none() {
                            TrComputePhase::skipped(ComputeSkipReason::NoState)
                        } else {
                            TrComputePhase::skipped(ComputeSkipReason::NoGas)
                        };
                    }
                    Some(ExecutorError::ExtMsgComputeSkipped(reason)) => {
                        description.compute_ph = TrComputePhase::skipped(reason.clone());
                    }
                    _ => return Err(err)?,
                }
                transaction.write_description(&TransactionDescr::Ordinary(description))?;
                let hash = acc_root.repr_hash();
                let state_update = HashUpdate::with_hashes(hash.clone(), hash);
                transaction.write_state_update(&state_update)?;
                Ok((transaction, lt))
            }
        }
    }

    fn execute_thread(
        blockchain_config: BlockchainConfig,
        shard_id: &ShardIdent,
        queue: Arc<InMessagesQueue>,
        tr_storage: Arc<T>,
        executors: Arc<Mutex<HashMap<AccountId, Arc<Mutex<OrdinaryTransactionExecutor>>>>>,
        msg: QueuedMessage,
        builder: Arc<BlockBuilder>,
        acc_id: &AccountId,
        new_shard_state: Arc<Mutex<ShardStateUnsplit>>,
        debug: bool,
    ) -> NodeResult<()> {
        let shard_acc = new_shard_state
            .lock()
            .read_accounts()?
            .account(acc_id)?
            .unwrap_or_default();
        let mut acc_root = shard_acc.account_cell();
        // TODO it is possible to make account immutable,
        // because in executor it is cloned for MerkleUpdate creation
        if !executors.lock().contains_key(acc_id) {
            let e = OrdinaryTransactionExecutor::new(blockchain_config);
            executors
                .lock()
                .insert(acc_id.clone(), Arc::new(Mutex::new(e)));
        }

        log::debug!("Executing message {:x}", msg.message_hash());
        let now = Instant::now();
        let executor = executors.lock().get(acc_id).unwrap().clone();
        let (mut transaction, max_lt) = Self::try_prepare_transaction(
            &builder,
            &executor.lock(),
            &mut acc_root,
            msg.message(),
            shard_acc.last_trans_lt(),
            debug,
        )?;
        transaction.set_prev_trans_hash(shard_acc.last_trans_hash().clone());
        transaction.set_prev_trans_lt(shard_acc.last_trans_lt());
        let transaction = Arc::new(transaction);
        log::info!(target: "profiler", "Transaction time: {} micros", now.elapsed().as_micros());
        // log::info!(target: "profiler", "Init time: {} micros", executor.lock().timing(0));
        // log::info!(target: "profiler", "Compute time: {} micros", executor.lock().timing(1));
        // log::info!(target: "profiler", "Finalization time: {} micros", executor.lock().timing(2));

        log::debug!("Transaction ID {:x}", transaction.hash()?);
        log::debug!("Transaction aborted: {}", transaction.read_description()?.is_aborted());

        let now = Instant::now();
        // update or remove shard account in new shard state
        let acc = Account::construct_from_cell(acc_root)?;
        if !acc.is_none() {
            let shard_acc =
                ShardAccount::with_params(&acc, transaction.hash()?, transaction.logical_time())?;
            new_shard_state
                .lock()
                .insert_account(&UInt256::from_slice(&acc_id.get_bytestring(0)), &shard_acc)?;
        } else {
            let mut shard_state = new_shard_state.lock();
            let mut accounts = shard_state.read_accounts()?;
            accounts.remove(acc_id.clone())?;
            shard_state.write_accounts(&accounts)?;
        }

        // loop-back for messages to current-shardchain
        Self::route_out_messages(
            shard_id,
            queue.clone(),
            transaction.clone(),
            new_shard_state.clone(),
        )?;

        if let Ok(Some(tr)) = tr_storage.find_by_lt(transaction.logical_time(), &acc_id) {
            panic!("{:?}\n{:?}", tr, transaction)
        }
        tr_storage.save_transaction(Arc::clone(&transaction))?;

        let in_message = Self::get_in_msg_from_transaction(shard_id, &transaction)?.unwrap();

        let imported_fees = in_message.get_fee()?;

        let out_messages =
            Self::get_out_msgs_from_transaction(shard_id, &transaction, &in_message)?;

        let mut exported_value = CurrencyCollection::new();
        let mut exported_fees = Grams::zero();

        let mut out_msg_vec = vec![];
        for m in out_messages.iter() {
            let out_msg_val = m.exported_value()?;
            exported_value.add(&out_msg_val)?;
            exported_value.grams.add(&out_msg_val.grams)?;
            exported_fees.add(&out_msg_val.grams)?;
            let exp_val = m.exported_value()?;

            // All out-messages there must contain message (as out msgs of transaction)
            out_msg_vec.push((m.serialize()?, exp_val));
        }

        // in-messages of transaction must contain message

        let transaction_cell = transaction.serialize()?;
        let context = AppendSerializedContext {
            in_msg: in_message.serialize()?,
            out_msgs: out_msg_vec,
            transaction,
            transaction_cell,
            max_lt,
            imported_value: Some(imported_fees.value_imported.clone()),
            exported_value,
            imported_fees,
            exported_fees,
        };

        if !builder.add_serialized_transaction(context) {
            log::warn!(target: "node", "Error append serialized transaction info to BlockBuilder");
            // TODO log error, write to transaction DB about error
        }
        log::info!(target: "profiler", "Transaction saving time: {} micros", now.elapsed().as_micros());
        Ok(())
    }

    ///
    /// Generate new block
    ///
    pub fn generate_block_multi(
        &mut self,
        shard_state: &ShardStateUnsplit,
        timeout: Duration,
        seq_no: u32,
        prev_ref: BlkPrevInfo,
        required_block_at: u32,
        debug: bool,
    ) -> NodeResult<Option<(Block, ShardStateUnsplit)>> {
        log::debug!("GENBLKMUL");
        let now = Instant::now();
        let start_time = Instant::now();
        let pool = ThreadPool::new(16);

        let new_shard_state = Arc::new(Mutex::new(shard_state.clone()));

        let builder = Arc::new(BlockBuilder::with_shard_ident(
            self.shard_id.clone(),
            seq_no,
            prev_ref,
            0,
            None,
            required_block_at,
        ));

        let mut is_empty = true;

        while start_time.elapsed() < timeout {
            if let Some(msg) = self.queue.dequeue_first_unused() {
                let acc_id = msg.message().int_dst_account_id().unwrap();

                // lock account in queue
                self.queue.lock_account(acc_id.clone());
                let shard_id = self.shard_id.clone();
                let queue = self.queue.clone();
                let storage = self.tr_storage.clone();
                let executors = self.executors.clone();
                let builder = builder.clone();
                let shard_state = new_shard_state.clone();
                let blockchain_config = self.blockchain_config.clone();
                let th = move || {
                    let res = Self::execute_thread(
                        blockchain_config,
                        &shard_id,
                        queue.clone(),
                        storage,
                        executors,
                        msg,
                        builder,
                        &acc_id,
                        shard_state,
                        debug,
                    );
                    queue.unlock_account(&acc_id);
                    if !res.is_ok() {
                        log::warn!(target: "node", "Executor execute failed. {}", res.unwrap_err());
                    }
                };

                pool.execute(th);

                is_empty = false;
            } else {
                thread::sleep(Duration::from_nanos(100));
            }
        }

        if !is_empty {
            pool.join();
            let time0 = now.elapsed().as_micros();

            log::info!(target: "node", "in messages queue len={}", self.queue.len());
            self.executors.lock().clear();
            self.queue.locks_clear();

            let new_shard_state = std::mem::take(&mut *new_shard_state.lock());
            let block = builder.finalize_block(shard_state, &new_shard_state)?;
            log::info!(target: "profiler",
                "Block time: non-final/final {} / {} micros",
                time0, now.elapsed().as_micros()
            );
            Ok(Some((block, new_shard_state)))
        } else {
            Ok(None)
        }
    }

    fn get_in_msg_from_transaction(
        _shard_id: &ShardIdent,
        transaction: &Transaction,
    ) -> NodeResult<Option<InMsg>> {
        if let Some(ref msg) = transaction.read_in_msg()? {
            let tr_cell = transaction.serialize()?;
            let msg = if msg.is_inbound_external() {
                InMsg::external_msg(transaction.in_msg_cell().unwrap_or_default(), tr_cell)
            } else {
                let fee = msg.get_fee()?.unwrap_or_default();
                let env = MsgEnvelope::with_message_and_fee(msg, fee.clone())?;
                InMsg::immediatelly_msg(env.serialize()?, tr_cell, fee)
            };
            Ok(Some(msg))
        } else {
            Ok(None)
        }
    }

    fn get_out_msgs_from_transaction(
        shard_id: &ShardIdent,
        transaction: &Transaction,
        reimport: &InMsg,
    ) -> NodeResult<Vec<OutMsg>> {
        let mut res = vec![];
        let tr_cell = transaction.serialize()?;
        transaction.iterate_out_msgs(|ref msg| {
            res.push(if msg.is_internal() {
                let env = MsgEnvelope::with_message_and_fee(msg, Grams::one())?;
                if shard_id.contains_address(&msg.dst().unwrap())? {
                    OutMsg::immediately_msg(
                        env.serialize()?,
                        tr_cell.clone(),
                        reimport.serialize()?,
                    )
                } else {
                    OutMsg::new_msg(env.serialize()?, tr_cell.clone())
                }
            } else {
                OutMsg::external_msg(msg.serialize()?, tr_cell.clone())
            });
            Ok(true)
        })?;
        Ok(res)
    }
}

/// Json rpc server for receiving external outbound messages.
/// TODO the struct is not used now (15.08.19). It is candidate to deletion.
pub struct JsonRpcMsgReceiver {
    host: String,
    port: String,
    server: Option<Server>,
}

#[allow(dead_code)]
impl MessagesReceiver for JsonRpcMsgReceiver {
    /// Start to receive messages. The function runs the receive thread and returns control.
    fn run(&mut self, queue: Arc<InMessagesQueue>) -> NodeResult<()> {
        if self.server.is_some() {
            Err(NodeError::InvalidOperation)
        } else {
            let mut io = IoHandler::default();
            io.add_method("call", move |params| {
                Self::process_call(params, Arc::clone(&queue))
            });

            self.server = Some(
                ServerBuilder::new(io)
                    .cors(DomainsValidation::AllowOnly(vec![
                        AccessControlAllowOrigin::Null,
                    ]))
                    .start_http(&format!("{}:{}", self.host, self.port).parse().unwrap())?,
            );

            Ok(())
        }
    }
}

#[allow(dead_code)]
impl JsonRpcMsgReceiver {
    /// Create a new instance of the struct witch put received messages into given queue
    pub fn with_params(host: &str, port: &str) -> Self {
        Self {
            host: String::from(host),
            port: String::from(port),
            server: None,
        }
    }

    /// Stop receiving. Sends message to the receive thread and waits while it stops.
    pub fn stop(&mut self) -> NodeResult<()> {
        if self.server.is_some() {
            let s = std::mem::replace(&mut self.server, None);
            s.unwrap().close();
            Ok(())
        } else {
            Err(NodeError::InvalidOperation)
        }
    }

    fn process_call(
        params: Params,
        msg_queue: Arc<InMessagesQueue>,
    ) -> jsonrpc_http_server::jsonrpc_core::Result<Value> {
        const MESSAGE: &str = "message";

        let map = match params {
            Params::Map(map) => map,
            _ => return Err(Error::invalid_params("Unresolved parameters object.")),
        };

        let message = match map.get(MESSAGE) {
            Some(Value::String(string)) => string,
            Some(_) => {
                return Err(Error::invalid_params(format!(
                    "\"{}\" parameter must be a string.",
                    MESSAGE
                )))
            }
            _ => {
                return Err(Error::invalid_params(format!(
                    "\"{}\" parameter not found.",
                    MESSAGE
                )))
            }
        };

        let message = Message::construct_from_base64(&message)
            .map_err(|err| Error::invalid_params(format!("Error parsing message: {}", err)))?;

        msg_queue
            .queue(QueuedMessage::with_message(message).unwrap())
            .expect("Error queue message");

        Ok(Value::String(String::from(
            "The message has been successfully received",
        )))
    }
}

/// Struct RouteMessage. Stored peedId of thew node received message
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RouteMessage {
    pub peer: usize,
    pub msg: Message,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QueuedMessageInternal {
    Message(Message),
    RouteMessage(RouteMessage),
}

impl QueuedMessageInternal {
    pub fn message(&self) -> &Message {
        match self {
            QueuedMessageInternal::Message(ref msg) => msg,
            QueuedMessageInternal::RouteMessage(ref r_msg) => &r_msg.msg,
        }
    }

}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueuedMessage {
    internal: QueuedMessageInternal,
    hash: UInt256,
}

impl Default for QueuedMessage {
    fn default() -> Self {
        Self::with_message(Message::default()).unwrap()
    }
}

impl PartialOrd for QueuedMessage {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for QueuedMessage {
    fn cmp(&self, other: &Self) -> Ordering {
        // All messages without LT will be at the end of the queue
        let result = self
            .message()
            .lt()
            .unwrap_or(u64::max_value())
            .cmp(&other.message().lt().unwrap_or(u64::max_value()));
        if result == Ordering::Equal {
            return self.hash.cmp(&other.hash);
        }
        result
    }
}

impl QueuedMessage {
    pub fn with_message(message: Message) -> Result<Self> {
        Self::new(QueuedMessageInternal::Message(message))
    }

    pub fn with_route_message(message: RouteMessage) -> Result<Self> {
        Self::new(QueuedMessageInternal::RouteMessage(message))
    }

    fn new(internal: QueuedMessageInternal) -> Result<Self> {
        let hash = internal.message().serialize()?.repr_hash();
        Ok(Self { internal, hash })
    }

    pub fn message(&self) -> &Message {
        self.internal.message()
    }

    pub fn message_hash(&self) -> &UInt256 {
        &self.hash
    }

}

impl Serializable for QueuedMessage {
    fn write_to(&self, cell: &mut BuilderData) -> Result<()> {
        match &self.internal {
            QueuedMessageInternal::Message(msg) => {
                cell.append_bits(0b1001, 4)?;
                msg.write_to(cell)?;
            }
            QueuedMessageInternal::RouteMessage(rm) => {
                cell.append_bits(0b0110, 4)?;
                (rm.peer as u64).write_to(cell)?;
                rm.msg.write_to(cell)?;
            }
        }
        Ok(())
    }
}

impl Deserializable for QueuedMessage {
    fn read_from(&mut self, slice: &mut SliceData) -> Result<()> {
        let tag = slice.get_next_int(4)? as usize;
        match tag {
            0b1001 => {
                *self = Self::with_message(Message::construct_from(slice)?)?;
            }
            0b0110 => {
                let mut peer: u64 = 0;
                let mut msg = Message::default();
                peer.read_from(slice)?;
                msg.read_from(slice)?;
                *self = Self::with_route_message(RouteMessage {
                    peer: peer as usize,
                    msg,
                })?;
            }
            _ => (),
        }

        Ok(())
    }
}

/// This FIFO accumulates inbound messages from all types of receivers.
/// The struct might be used from many threads. It provides internal mutability.
pub struct InMessagesQueue {
    shard_id: ShardIdent,
    storage: Mutex<BTreeSet<QueuedMessage>>,
    out_storage: Mutex<VecDeque<QueuedMessage>>,
    db: Option<Arc<dyn DocumentsDb>>,
    used_accs: Mutex<HashSet<AccountId>>,
    capacity: usize,
    ready_to_process: AtomicBool,
}

#[allow(dead_code)]
impl InMessagesQueue {
    /// Create new instance of InMessagesQueue.
    pub fn new(shard_id: ShardIdent, capacity: usize) -> Self {
        InMessagesQueue {
            shard_id,
            storage: Mutex::new(BTreeSet::new()),
            out_storage: Mutex::new(VecDeque::new()),
            used_accs: Mutex::new(HashSet::new()),
            db: None,
            capacity,
            ready_to_process: AtomicBool::new(false),
        }
    }

    pub fn with_db(shard_id: ShardIdent, capacity: usize, db: Arc<dyn DocumentsDb>) -> Self {
        InMessagesQueue {
            shard_id,
            storage: Mutex::new(BTreeSet::new()),
            out_storage: Mutex::new(VecDeque::new()),
            used_accs: Mutex::new(HashSet::new()),
            db: Some(db),
            capacity,
            ready_to_process: AtomicBool::new(false),
        }
    }

    ///
    /// Set in message queue ready-mode
    /// true - node ready to process messages and generate block
    /// false - node receive messages and route they to another nodes
    ///
    pub fn set_ready(&self, mode: bool) {
        log::info!(target: "node", "in message queue set ready-mode: {}", mode);
        self.ready_to_process.store(mode, AtomicOrdering::SeqCst);
    }

    ///
    /// Get mode
    ///
    pub fn ready(&self) -> bool {
        self.ready_to_process.load(AtomicOrdering::SeqCst)
    }

    pub fn has_delivery_problems(&self) -> bool {
        self.db
            .as_ref()
            .map_or(false, |db| db.has_delivery_problems())
    }

    fn route_message_to_other_node(
        &self,
        msg: QueuedMessage,
    ) -> std::result::Result<(), QueuedMessage> {
        let mut out_storage = self.out_storage.lock();
        out_storage.push_back(msg);
        Ok(())
    }

    fn is_message_to_current_node(&self, msg: &Message) -> bool {
        if let Some(msg_dst) = msg.dst() {
            return self.shard_id.contains_address(&msg_dst).unwrap();
        }
        true // if message hasn't workchain or address, it will be process any node
    }

    /// Include message into end queue.
    pub fn queue(&self, msg: QueuedMessage) -> std::result::Result<(), QueuedMessage> {
        // messages unsuitable to this node route all time
        if !self.is_message_to_current_node(msg.message()) {
            log::debug!(target: "node", "MESSAGE-IS-FOR-OTHER-NODE {:?}", msg);
            return self.route_message_to_other_node(msg);
        }

        if self.has_delivery_problems() {
            log::debug!(target: "node", "Has delivery problems");
            return Err(msg);
        }

        let mut storage = self.storage.lock();
        if storage.len() >= self.capacity {
            return Err(msg);
        }

        storage.insert(msg.clone());
        log::debug!(target: "node", "Queued message: {:?}", msg.message());

        Ok(())
    }

    /// Include message into begin queue
    fn priority_queue(&self, msg: QueuedMessage) -> std::result::Result<(), QueuedMessage> {
        if !self.is_message_to_current_node(msg.message()) {
            return self.route_message_to_other_node(msg);
        }

        let mut storage = self.storage.lock();
        let msg_str = format!("{:?}", msg.message());
        storage.insert(msg);
        log::debug!(target: "node", "Priority queued message: {}", msg_str);

        Ok(())
    }

    /// Extract oldest message from queue.
    pub fn dequeue(&self) -> Option<QueuedMessage> {
        let mut storage = self.storage.lock();
        let first = if let Some(first) = storage.iter().next() {
            first.clone()
        } else {
            return None;
        };
        storage.remove(&first);
        Some(first)
    }

    /// Extract oldest message from out_queue.
    pub fn dequeue_out(&self) -> Option<QueuedMessage> {
        let mut out_storage = self.out_storage.lock();
        out_storage.pop_front()
    }

    /// Extract oldest message from queue if message account not using in executor
    pub fn dequeue_first_unused(&self) -> Option<QueuedMessage> {
        let mut storage = self.storage.lock();
        let used_accs = self.used_accs.lock();
        // iterate from front and find unused account message
        let result = storage
            .iter()
            .find(|msg| {
                msg.message()
                    .int_dst_account_id()
                    .map(|acc_id| !used_accs.contains(&acc_id))
                    .unwrap_or(false)
            })
            .cloned();

        if let Some(ref msg) = result {
            storage.remove(msg);
        }

        result
    }

    pub fn print_message(msg: &Message) {
        log::info!("message: {:?}", msg);
        if let Ok(cell) = msg.serialize() {
            if let Ok(data) = serialize_toc(&cell) {
                std::fs::create_dir_all("export").ok();
                std::fs::write(&format!("export/msg_{:x}", cell.repr_hash()), &data).ok();
            }
        }
    }

    pub fn is_full(&self) -> bool {
        dbg!(self.len()) >= self.capacity
    }

    /// The length of queue.
    pub fn len(&self) -> usize {
        self.storage.lock().len()
    }

    /// lock account message for dequeue
    pub fn lock_account(&self, account_id: AccountId) {
        self.used_accs.lock().insert(account_id);
    }

    /// unlock account messages for dequeue
    pub fn unlock_account(&self, account_id: &AccountId) {
        self.used_accs.lock().remove(account_id);
    }

    /// Unlock all accounts
    pub fn locks_clear(&self) {
        self.used_accs.lock().clear();
    }
}

