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

use std::thread;
use std::ops::Deref;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use ton_block::*;
use ton_executor::*;
use ton_types::*;

use crate::engine::{QueuedMessage, InMessagesQueue};
use crate::error::NodeResult;

#[cfg(test)]
#[path = "../../../../tonos-se-tests/unit/test_block_builder.rs"]
mod tests;

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
    in_msg_descr: InMsgDescr,
    out_msg_descr: OutMsgDescr,
    out_queue_info: OutMsgQueueInfo,

    account_blocks: ShardAccountBlocks,
    total_gas_used: u64,
    start_lt: u64,
    end_lt: u64, // biggest logical time of all messages
}

impl BlockBuilder {
    ///
    /// Initialize BlockBuilder with shard identifier
    ///
    pub fn with_params(
        shard_state: Arc<ShardStateUnsplit>,
        seq_no: u32,
        prev_ref: BlkPrevInfo,
        block_at: u32,
    ) -> Result<Self> {
        let accounts = shard_state.read_accounts()?;
        let out_queue_info = shard_state.read_out_msg_queue_info()?;

        let start_lt = prev_ref.prev1().map_or(0, |p| p.end_lt) + 1;
        let rand_seed = UInt256::rand(); // we don't need strict randomization like real node

        let mut block_info = BlockInfo::default();
        block_info.set_shard(shard_state.shard().clone());
        block_info.set_seq_no(seq_no).unwrap();
        block_info.set_prev_stuff(false, &prev_ref).unwrap();
        block_info.set_gen_utime(UnixTime32::new(block_at));
        block_info.set_start_lt(start_lt);

        Ok(BlockBuilder {
            shard_state,
            out_queue_info,
            from_prev_blk: accounts.full_balance().clone(),
            accounts,
            block_info,
            rand_seed,
            start_lt,
            ..Default::default()
        })
    }

    pub fn with_shard_ident(
        shard_id: ShardIdent,
        seq_no: u32,
        prev_ref: BlkPrevInfo,
        block_at: u32,
    ) -> Self {
        Self::with_params(
            Arc::new(ShardStateUnsplit::with_ident(shard_id)),
            seq_no,
            prev_ref,
            block_at
        ).unwrap()
    }

    /// Shard ident
    fn shard_ident(&self) -> &ShardIdent {
        self.shard_state.shard()
    }

    fn out_msg_key(&self, prefix: u64, hash: UInt256) -> OutMsgQueueKey {
        OutMsgQueueKey::with_workchain_id_and_prefix(
            self.shard_ident().workchain_id(),
            prefix,
            hash
        )
    }

    fn try_prepare_transaction(
        &mut self,
        executor: &OrdinaryTransactionExecutor,
        acc_root: &mut Cell,
        msg: &Message,
        acc_last_lt: u64,
        debug: bool,
    ) -> NodeResult<(Transaction, u64)> {
        let (block_unixtime, block_lt) = self.at_and_lt();
        let last_lt = std::cmp::max(acc_last_lt, block_lt);
        let lt = Arc::new(AtomicU64::new(last_lt + 1));
        let result = executor.execute_with_libs_and_params(
            Some(&msg),
            acc_root,
            ExecuteParams {
                block_unixtime,
                block_lt,
                last_tr_lt: Arc::clone(&lt),
                seed_block: self.rand_seed.clone(),
                debug,
                ..Default::default()
            },
        );
        match result {
            Ok(transaction) => {
                if let Some(gas_used) = transaction.gas_used() {
                    self.total_gas_used += gas_used;
                }
                Ok((transaction, lt.load(Ordering::Relaxed)))
            }
            Err(err) => {
                let lt = last_lt + 1;
                let account = Account::construct_from_cell(acc_root.clone())?;
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

    pub fn execute(
        &mut self,
        msg: QueuedMessage,
        blockchain_config: BlockchainConfig,
        acc_id: &AccountId,
        debug: bool,
    ) -> NodeResult<()> {
        let shard_acc = self.accounts.account(acc_id)?.unwrap_or_default();
        let mut acc_root = shard_acc.account_cell();
        let executor = OrdinaryTransactionExecutor::new(blockchain_config);

        log::debug!("Executing message {:x}", msg.message_hash());
        let (mut transaction, max_lt) = self.try_prepare_transaction(
            &executor,
            &mut acc_root,
            msg.message(),
            shard_acc.last_trans_lt(),
            debug,
        )?;
        self.end_lt = std::cmp::max(self.end_lt, max_lt);
        transaction.set_prev_trans_hash(shard_acc.last_trans_hash().clone());
        transaction.set_prev_trans_lt(shard_acc.last_trans_lt());
        // log::info!(target: "profiler", "Init time: {} micros", executor.lock().timing(0));
        // log::info!(target: "profiler", "Compute time: {} micros", executor.lock().timing(1));
        // log::info!(target: "profiler", "Finalization time: {} micros", executor.lock().timing(2));

        log::debug!("Transaction ID {:x}", transaction.hash()?);
        log::debug!("Transaction aborted: {}", transaction.read_description()?.is_aborted());

        // update or remove shard account in new shard state
        let acc = Account::construct_from_cell(acc_root.clone())?;
        if !acc.is_none() {
            let shard_acc =
                ShardAccount::with_account_root(acc_root, transaction.hash()?, transaction.logical_time());
            let data = shard_acc.write_to_new_cell()?;
            self.accounts.set_builder_serialized(acc_id.clone(), &data, &acc.aug()?)?;
        } else {
            self.accounts.remove(acc_id.clone())?;
        }

        if let Err(err) = self.add_raw_transaction(transaction) {
            log::warn!(target: "node", "Error append serialized transaction info to BlockBuilder {}", err);
            // TODO log error, write to transaction DB about error
        }
        Ok(())
    }

    pub fn build_block(
        mut self,
        queue: Arc<InMessagesQueue>,
        blockchain_config: BlockchainConfig,
        timeout: Duration,
        debug: bool
    ) -> NodeResult<Option<(Block, ShardStateUnsplit)>> {
        let start_time = Instant::now();

        let mut is_empty = true;

        // first import internal messages
        let mut block_full = false;
        let out_queue = self.out_queue_info.out_queue().clone();
        log::info!(target: "node", "out queue len={}", out_queue.len()?);
        for out in out_queue.iter() {
            let (key, mut slice) = out?;
            let key = key.into_cell()?;
            // key is not matter for one shard
            let (enq, _create_lt) = OutMsgQueue::value_aug(&mut slice)?;
            let env = enq.read_out_msg()?;
            let message = env.read_message()?;
            if let Some(acc_id) = message.int_dst_account_id() {
                let msg = QueuedMessage::with_message(message)?;
                self.execute(
                    msg,
                    blockchain_config.clone(),
                    &acc_id,
                    debug
                )?;
            }
            self.out_queue_info.out_queue_mut().remove(key.into())?;
            // TODO: check block full
            is_empty = false;
            if self.total_gas_used > 1_000_000 {
                block_full = true;
                break;
            }
        }
        // second import external messages
        while start_time.elapsed() < timeout {
            if queue.len() != 0 {
                log::error!(target: "node", "external messages queue len={}", queue.len());
            }
            if let Some(msg) = queue.dequeue() {
                let acc_id = msg.message().int_dst_account_id().unwrap();

                // lock account in queue
                let res = self.execute(
                    msg,
                    blockchain_config.clone(),
                    &acc_id,
                    debug,
                );
                if let Err(err) = res {
                    log::warn!(target: "node", "Executor execute failed. {}", err);
                } else {
                    log::info!(target: "node", "external msg transaction compleete");
                }
                is_empty = false;
                if self.total_gas_used > 1_000_000 {
                    block_full = true;
                    break;
                }
            } else {
                thread::sleep(Duration::from_millis(100));
            }
        }

        // proceed new messages
        while !self.new_messages.is_empty() {
            log::info!(target: "node", "new messages len {}", self.new_messages.len());
            for (message, tr_cell) in std::mem::take(&mut self.new_messages).drain(..) {
                let info = message.int_header().ok_or_else(|| error!("message is not internal"))?;
                let fwd_fee = info.fwd_fee().clone();
                let msg_cell = message.serialize()?;
                let env = MsgEnvelope::with_message_cell_and_fee(msg_cell.clone(), fwd_fee);
                let acc_id = message.int_dst_account_id().unwrap_or_default();
                if !self.shard_ident().contains_address(&info.dst)? || block_full {
                    let enq = EnqueuedMsg::with_param(info.created_lt, &env)?;
                    let key = self.out_msg_key(acc_id.clone().get_next_u64()?, msg_cell.repr_hash());
                    self.out_queue_info.out_queue_mut().set(&key, &enq, &enq.aug()?)?;
                    let out_msg = OutMsg::new_msg(enq.out_msg_cell(), tr_cell);
                    self.out_msg_descr.set(&msg_cell.repr_hash(), &out_msg, &out_msg.aug()?)?;
                    // collator_data.add_out_msg_to_block(out_msg.read_message_hash()?, &out_msg)?;
                } else {
                    // let created_lt = info.created_lt;
                    // let hash = msg_cell.repr_hash();
                    // collator_data.update_last_proc_int_msg((created_lt, hash))?;
                    self.execute(
                        QueuedMessage::with_message(message)?,
                        blockchain_config.clone(),
                        &acc_id,
                        debug
                    )?;
                    if self.total_gas_used > 1_000_000 {
                        block_full = true;
                    }
                };
                is_empty = false;
            }
        }
        
        if !is_empty {
            log::info!(target: "node", "in messages queue len={}", queue.len());
            let seq_no = self.block_info.seq_no();
            let (block, new_shard_state) = self.finalize_block()?;
            let block_text = ton_block_json::debug_block_full(&block)?;
            std::fs::write(&format!("e:\\block_{}.json", seq_no), block_text)?;
            thread::sleep(Duration::from_millis(2000));
            Ok(Some((block, new_shard_state)))
        } else {
            log::info!(target: "node", "block is empty in messages queue len={}", queue.len());
            Ok(None)
        }
    }

    ///
    /// Add transaction to block
    ///
    pub fn add_raw_transaction(&mut self, transaction: Transaction) -> NodeResult<()> {
        let tr_cell = transaction.serialize()?;
        log::debug!("Inserting transaction {:x}", tr_cell.repr_hash());

        self.account_blocks.add_serialized_transaction(&transaction, &tr_cell)?;

        if let Some(msg_cell) = transaction.in_msg_cell() {
            let msg = Message::construct_from_cell(msg_cell.clone())?;
            let in_msg = if let Some(hdr) = msg.int_header() {
                let fee = hdr.fwd_fee();
                let env = MsgEnvelope::with_message_and_fee(&msg, *fee)?;
                InMsg::immediatelly_msg(env.serialize()?, tr_cell.clone(), *fee)
            } else {
                InMsg::external_msg(msg_cell.clone(), tr_cell.clone())
            };
            self.in_msg_descr.set(&msg_cell.repr_hash(), &in_msg, &in_msg.aug()?)?;
        }

        let mut exported_value = CurrencyCollection::default();
        let mut exported_fees = Grams::default();

        transaction.iterate_out_msgs(|msg| {
            if let Some(hdr) = msg.int_header() {
                exported_value.add(&hdr.value)?;
                exported_fees.add(&hdr.fwd_fee)?;
            }
            self.new_messages.push((msg, tr_cell.clone()));
            Ok(true)
        })?;
        Ok(())
    }

    ///
    /// Check if BlockBuilder is Empty
    ///
    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.in_msg_descr.is_empty() && self.out_msg_descr.is_empty() && self.account_blocks.is_empty()
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
    pub fn finalize_block(self) -> Result<(Block, ShardStateUnsplit)> {
        let mut new_shard_state = self.shard_state.deref().clone();
        new_shard_state.write_accounts(&self.accounts)?;
        new_shard_state.write_out_msg_queue_info(&self.out_queue_info)?;
        let mut block_extra = BlockExtra::default();
        block_extra.write_in_msg_descr(&self.in_msg_descr)?;
        block_extra.write_out_msg_descr(&self.out_msg_descr)?;
        block_extra.write_account_blocks(&self.account_blocks)?;
        block_extra.rand_seed = self.rand_seed;

        let mut value_flow = ValueFlow::default();
        value_flow.fees_collected = self.account_blocks.root_extra().clone();
        value_flow.fees_collected.grams.add(&self.in_msg_descr.root_extra().fees_collected)?;
        value_flow.imported = self.in_msg_descr.root_extra().value_imported.clone();
        value_flow.exported = self.out_msg_descr.root_extra().clone();
        value_flow.from_prev_blk = self.from_prev_blk;
        value_flow.to_next_blk = self.accounts.full_balance().clone();
        
        // it can be long, even we don't need to build it
        // let new_ss_root = new_shard_state.serialize()?;
        // let old_ss_root = self.shard_state.serialize()?;
        // let state_update = MerkleUpdate::create(&old_ss_root, &new_ss_root)?;
        let state_update = MerkleUpdate::default();

        let block = Block::with_params(
            0,
            self.block_info,
            value_flow,
            state_update,
            block_extra,
        )?;
        Ok((block, new_shard_state))

    }
}
