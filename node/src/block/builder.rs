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

use crate::engine::messages::InMessagesQueue;
use crate::engine::BlockTimeMode;
use std::collections::HashMap;
use std::ops::Deref;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use ton_block::{
    Account, AddSub, Augmentation, BlkPrevInfo, Block, BlockExtra, BlockInfo, ComputeSkipReason,
    CopyleftRewards, CurrencyCollection, Deserializable, EnqueuedMsg, HashUpdate, HashmapAugType,
    InMsg, InMsgDescr, MerkleUpdate, Message, MsgEnvelope, OutMsg, OutMsgDescr, OutMsgQueue,
    OutMsgQueueInfo, OutMsgQueueKey, Serializable, ShardAccount, ShardAccountBlocks, ShardAccounts,
    ShardIdent, ShardStateUnsplit, TrComputePhase, TrComputePhaseVm, Transaction, TransactionDescr,
    TransactionDescrOrdinary, UnixTime32, ValueFlow,
};
use ton_executor::{
    BlockchainConfig, ExecuteParams, ExecutorError, OrdinaryTransactionExecutor,
    TransactionExecutor,
};
use ton_types::{error, AccountId, Cell, HashmapRemover, HashmapType, Result, SliceData, UInt256};

use crate::error::NodeResult;

pub struct PreparedBlock {
    pub block: Block,
    pub state: ShardStateUnsplit,
    pub is_empty: bool,
    pub transaction_traces: HashMap<UInt256, String>,
}

///
/// BlockBuilder structure
///
#[derive(Default)]
pub struct BlockBuilder {
    shard_state: Arc<ShardStateUnsplit>,
    accounts: ShardAccounts,
    block_info: BlockInfo,
    rand_seed: UInt256,
    new_messages: Vec<(Message, Cell)>, // TODO: BinaryHeap, Cell
    from_prev_blk: CurrencyCollection,
    pub(crate) in_msg_descr: InMsgDescr,
    pub(crate) out_msg_descr: OutMsgDescr,
    out_queue_info: OutMsgQueueInfo,
    block_gas_limit: u64,
    pub(crate) account_blocks: ShardAccountBlocks,
    total_gas_used: u64,
    total_message_processed: usize,
    start_lt: u64,
    end_lt: u64, // biggest logical time of all messages
    copyleft_rewards: CopyleftRewards,
    transaction_traces: HashMap<UInt256, String>,

    time_mode: BlockTimeMode,
}

impl BlockBuilder {
    ///
    /// Initialize BlockBuilder
    ///
    pub fn with_params(
        shard_state: Arc<ShardStateUnsplit>,
        prev_ref: BlkPrevInfo,
        time: u32,
        time_mode: BlockTimeMode,
        block_gas_limit: u64,
    ) -> Result<Self> {
        let accounts = shard_state.read_accounts()?;
        let out_queue_info = shard_state.read_out_msg_queue_info()?;
        let seq_no = shard_state.seq_no() + 1;

        let start_lt = prev_ref.prev1().map_or(0, |p| p.end_lt) + 1;
        let rand_seed = UInt256::rand(); // we don't need strict randomization like real node

        let mut block_info = BlockInfo::default();
        block_info.set_shard(shard_state.shard().clone());
        block_info.set_seq_no(seq_no).unwrap();
        block_info.set_prev_stuff(false, &prev_ref).unwrap();
        block_info.set_gen_utime(UnixTime32::new(time));
        block_info.set_start_lt(start_lt);

        Ok(BlockBuilder {
            shard_state,
            out_queue_info,
            from_prev_blk: accounts.full_balance().clone(),
            accounts,
            block_info,
            rand_seed,
            start_lt,
            end_lt: start_lt + 1,
            time_mode,
            block_gas_limit,
            ..Default::default()
        })
    }

    /// Shard ident
    fn shard_ident(&self) -> &ShardIdent {
        self.shard_state.shard()
    }

    fn out_msg_key(&self, prefix: u64, hash: UInt256) -> OutMsgQueueKey {
        OutMsgQueueKey::with_workchain_id_and_prefix(
            self.shard_ident().workchain_id(),
            prefix,
            hash,
        )
    }

    fn trace_info(engine: &ton_vm::executor::Engine, info: &ton_vm::executor::EngineTraceInfo) -> String {
        let mut trace = String::new();
        if info.has_cmd() {
            trace = format!(
                "{trace}\n{}: {}\n",
                info.step,
                info.cmd_str,
            );
        }
        trace = format!(
            "{trace}\nGas: {} ({})\n",
            info.gas_used,
            info.gas_cmd
        );
        trace = format!("{trace}\n{}\n{}", engine.dump_stack("Stack trace", false), engine.dump_ctrls(true));
        if info.info_type == ton_vm::executor::EngineTraceInfoType::Dump {
            trace = format!("{trace}\n{}", info.cmd_str);
        }
        trace
    }

    fn try_prepare_transaction(
        &mut self,
        executor: &OrdinaryTransactionExecutor,
        acc_root: &mut Cell,
        msg: &Message,
        last_lt: u64,
        debug: bool,
    ) -> NodeResult<(Transaction, u64, Option<String>)> {
        let (block_unixtime, block_lt) = self.at_and_lt();
        let lt = Arc::new(AtomicU64::new(last_lt));
        let trace = Arc::new(lockfree::queue::Queue::new());
        let trace_copy = trace.clone();
        let callback = move |engine: &ton_vm::executor::Engine, info: &ton_vm::executor::EngineTraceInfo| {
            trace.push(Self::trace_info(engine, info));
        };
        let start = std::time::Instant::now();
        let result = executor.execute_with_libs_and_params(
            Some(msg),
            acc_root,
            ExecuteParams {
                block_unixtime,
                block_lt,
                last_tr_lt: Arc::clone(&lt),
                seed_block: self.rand_seed.clone(),
                debug,
                signature_id: self.shard_state.global_id(),
                trace_callback: Some(Arc::new(callback)),
                ..Default::default()
            },
        );
        log::trace!("Execution time {} ms", start.elapsed().as_millis());
        match result {
            Ok(transaction) => {
                if let Some(gas_used) = transaction.gas_used() {
                    self.total_gas_used += gas_used;
                }
                let trace = if transaction.read_description()?.is_aborted() {
                    let trace = trace_copy.pop_iter().collect::<String>();
                    if trace.is_empty() { None } else { Some(trace) }
                } else {
                    None
                };
                Ok((transaction, lt.load(Ordering::Relaxed), trace))
            }
            Err(err) => {
                let old_hash = acc_root.repr_hash();
                let mut account = Account::construct_from_cell(acc_root.clone())?;
                let lt = std::cmp::max(
                    account.last_tr_time().unwrap_or(0),
                    std::cmp::max(last_lt, msg.lt().unwrap_or(0) + 1),
                );
                account.set_last_tr_time(lt);
                *acc_root = account.serialize()?;
                let mut transaction = Transaction::with_account_and_message(&account, msg, lt)?;
                transaction.set_now(block_unixtime);
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
                                .and_then(|value| value.into(i32::MIN..=i32::MAX))
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
                let state_update = HashUpdate::with_hashes(old_hash, acc_root.repr_hash());
                transaction.write_state_update(&state_update)?;
                let trace = trace_copy.pop_iter().collect::<String>();
                Ok((transaction, lt, Some(trace)))
            }
        }
    }

    pub fn execute(
        &mut self,
        message: Message,
        blockchain_config: &BlockchainConfig,
        acc_id: &AccountId,
        debug: bool,
    ) -> NodeResult<()> {
        self.total_message_processed += 1;
        let shard_acc = self.accounts.account(acc_id)?.unwrap_or_default();
        let mut acc_root = shard_acc.account_cell();
        let executor = OrdinaryTransactionExecutor::new((*blockchain_config).clone());

        log::debug!(
            "Executing message {:x}",
            message.serialize().unwrap_or_default().repr_hash()
        );
        let (mut transaction, max_lt, trace) = self.try_prepare_transaction(
            &executor,
            &mut acc_root,
            &message,
            std::cmp::max(self.start_lt, shard_acc.last_trans_lt() + 1),
            debug,
        )?;
        self.end_lt = std::cmp::max(self.end_lt, max_lt);
        transaction.set_prev_trans_hash(shard_acc.last_trans_hash().clone());
        transaction.set_prev_trans_lt(shard_acc.last_trans_lt());
        let tr_cell = transaction.serialize()?;

        if let Some(trace) = trace {
            self.transaction_traces.insert(tr_cell.repr_hash(), trace);
        }

        log::debug!("Transaction ID {:x}", tr_cell.repr_hash());
        log::debug!(
            "Transaction aborted: {}",
            transaction.read_description()?.is_aborted()
        );

        let acc = Account::construct_from_cell(acc_root.clone())?;
        if !acc.is_none() {
            let shard_acc = ShardAccount::with_account_root(
                acc_root,
                tr_cell.repr_hash(),
                transaction.logical_time(),
            );
            let data = shard_acc.write_to_new_cell()?;
            self.accounts
                .set_builder_serialized(acc_id.clone(), &data, &acc.aug()?)?;
        } else {
            self.accounts.remove(acc_id.clone())?;
        }

        if let Err(err) = self.add_raw_transaction(transaction, tr_cell) {
            log::warn!(target: "node", "Error append transaction {}", err);
            // TODO log error, write to transaction DB about error
        }
        Ok(())
    }

    fn is_limits_reached(&self) -> bool {
        self.total_gas_used > self.block_gas_limit
            || (self.time_mode.is_seq() && self.total_message_processed > 0)
    }

    pub fn build_block(
        mut self,
        queue: &InMessagesQueue,
        blockchain_config: &BlockchainConfig,
        debug: bool,
    ) -> NodeResult<PreparedBlock> {
        let mut is_empty = true;
        let mut block_full = false;

        // first import internal messages
        let out_queue = self.out_queue_info.out_queue().clone();
        let msg_count = out_queue.len()?;
        log::debug!(target: "node", "out_queue.len={}, queue.len={}", msg_count, queue.len());
        let mut sorted = Vec::with_capacity(msg_count);
        for out in out_queue.iter() {
            let (key, mut slice) = out?;
            let key = key.into_cell()?;
            // key is not matter for one shard
            sorted.push((key, OutMsgQueue::value_aug(&mut slice)?));
        }
        sorted.sort_by(|a, b| a.1 .1.cmp(&b.1 .1));
        for (key, (enq, _create_lt)) in sorted {
            let env = enq.read_out_msg()?;
            let message = env.read_message()?;
            if let Some(acc_id) = message.int_dst_account_id() {
                self.execute(message, blockchain_config, &acc_id, debug)?;
            }
            self.out_queue_info
                .out_queue_mut()
                .remove(SliceData::load_cell(key)?)?;
            // TODO: check block full
            is_empty = false;
            if self.is_limits_reached() {
                block_full = true;
                break;
            }
        }
        let workchain_id = self.shard_state.shard().workchain_id();
        // second import external messages
        if !block_full {
            while let Some(msg) = queue.dequeue(workchain_id) {
                let acc_id = msg.int_dst_account_id().unwrap();

                // lock account in queue
                let res = self.execute(msg, blockchain_config, &acc_id, debug);
                if let Err(err) = res {
                    log::warn!(target: "node", "Executor execute failed. {}", err);
                } else {
                    log::info!(target: "node", "external msg transaction complete");
                }
                is_empty = false;
                if self.is_limits_reached() {
                    block_full = true;
                    break;
                }
            }
        }

        // proceed new messages
        while !self.new_messages.is_empty() {
            log::info!(target: "node", "new messages len {}", self.new_messages.len());
            for (message, tr_cell) in std::mem::take(&mut self.new_messages).drain(..) {
                let info = message
                    .int_header()
                    .ok_or_else(|| error!("message is not internal"))?;
                let fwd_fee = info.fwd_fee();
                let msg_cell = message.serialize()?;
                // TODO: use it when interface is merged
                // let env = MsgEnvelope::with_message_cell_and_fee(msg_cell.clone(), *fwd_fee);
                let env = MsgEnvelope::with_message_and_fee(&message, *fwd_fee)?;
                let acc_id = message.int_dst_account_id().unwrap_or_default();
                if !self.shard_ident().contains_address(&info.dst)? || block_full {
                    let enq = EnqueuedMsg::with_param(info.created_lt, &env)?;
                    let prefix = acc_id.clone().get_next_u64()?;
                    let key = self.out_msg_key(prefix, msg_cell.repr_hash());
                    self.out_queue_info
                        .out_queue_mut()
                        .set(&key, &enq, &enq.aug()?)?;
                    let out_msg = OutMsg::new(enq.out_msg_cell(), tr_cell);
                    self.out_msg_descr
                        .set(&msg_cell.repr_hash(), &out_msg, &out_msg.aug()?)?;
                } else {
                    self.execute(message, blockchain_config, &acc_id, debug)?;
                    if self.is_limits_reached() {
                        block_full = true;
                    }
                };
                is_empty = false;
            }
        }
        if !is_empty {
            log::info!(target: "node", "in messages queue len={}", queue.len());
        } else {
            log::debug!(target: "node", "block is empty in messages queue len={}", queue.len());
        }
        let transaction_traces = std::mem::take(&mut self.transaction_traces);
        let (block, new_shard_state) = self.finalize_block()?;
        Ok(PreparedBlock {
            block,
            state: new_shard_state,
            is_empty,
            transaction_traces,
        })
    }

    ///
    /// Add transaction to block
    ///
    pub fn add_raw_transaction(
        &mut self,
        transaction: Transaction,
        tr_cell: Cell,
    ) -> NodeResult<()> {
        log::debug!("Inserting transaction {:x}", tr_cell.repr_hash());

        self.account_blocks
            .add_serialized_transaction(&transaction, &tr_cell)?;

        if let Some(copyleft_reward) = transaction.copyleft_reward() {
            self.copyleft_rewards
                .add_copyleft_reward(&copyleft_reward.address, &copyleft_reward.reward)?;
        }

        if let Some(msg_cell) = transaction.in_msg_cell() {
            let msg = Message::construct_from_cell(msg_cell.clone())?;
            let in_msg = if let Some(hdr) = msg.int_header() {
                let fee = hdr.fwd_fee();
                let env = MsgEnvelope::with_message_and_fee(&msg, *fee)?;
                InMsg::immediate(env.serialize()?, tr_cell.clone(), *fee)
            } else {
                InMsg::external(msg_cell.clone(), tr_cell.clone())
            };
            self.in_msg_descr
                .set(&msg_cell.repr_hash(), &in_msg, &in_msg.aug()?)?;
        }

        transaction.iterate_out_msgs(|msg| {
            if let Some(_) = msg.int_header() {
                self.new_messages.push((msg, tr_cell.clone()));
            } else {
                let msg_cell = msg.serialize()?;
                let out_msg = OutMsg::external(msg_cell.clone(), tr_cell.clone());
                self.out_msg_descr
                    .set(&msg_cell.repr_hash(), &out_msg, &out_msg.aug()?)?;
            }
            Ok(true)
        })?;
        Ok(())
    }

    ///
    /// Get UNIX time and Logical Time of current block
    ///
    pub fn at_and_lt(&self) -> (u32, u64) {
        (self.block_info.gen_utime().as_u32(), self.start_lt)
    }

    ///
    /// Complete the construction of the block and return it.
    /// returns generated block and new shard state bag (and transaction count)
    ///
    pub fn finalize_block(mut self) -> Result<(Block, ShardStateUnsplit)> {
        let mut new_shard_state = self.shard_state.deref().clone();
        new_shard_state.set_seq_no(self.block_info.seq_no());
        new_shard_state.write_accounts(&self.accounts)?;
        new_shard_state.write_out_msg_queue_info(&self.out_queue_info)?;

        let mut block_extra = BlockExtra::default();
        block_extra.write_in_msg_descr(&self.in_msg_descr)?;
        block_extra.write_out_msg_descr(&self.out_msg_descr)?;
        block_extra.write_account_blocks(&self.account_blocks)?;
        block_extra.rand_seed = self.rand_seed;

        let mut value_flow = ValueFlow::default();
        value_flow.fees_collected = self.account_blocks.root_extra().clone();
        value_flow
            .fees_collected
            .grams
            .add(&self.in_msg_descr.root_extra().fees_collected)?;
        value_flow.imported = self.in_msg_descr.root_extra().value_imported.clone();
        value_flow.exported = self.out_msg_descr.root_extra().clone();
        value_flow.from_prev_blk = self.from_prev_blk;
        value_flow.to_next_blk = self.accounts.full_balance().clone();
        value_flow.copyleft_rewards = self.copyleft_rewards;

        // it can be long, even we don't need to build it
        // let new_ss_root = new_shard_state.serialize()?;
        // let old_ss_root = self.shard_state.serialize()?;
        // let state_update = MerkleUpdate::create(&old_ss_root, &new_ss_root)?;
        let state_update = MerkleUpdate::default();
        self.block_info
            .set_end_lt(self.end_lt.max(self.start_lt + 1));

        let block = Block::with_params(
            self.shard_state.global_id(),
            self.block_info,
            value_flow,
            state_update,
            block_extra,
        )?;
        Ok((block, new_shard_state))
    }
}
