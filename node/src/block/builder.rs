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

use std::ops::Deref;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use ton_block::{
    Account, AddSub, Augmentation, BlkPrevInfo, Block, BlockExtra, BlockInfo, ComputeSkipReason,
    CurrencyCollection, Deserializable, EnqueuedMsg, HashUpdate, HashmapAugType, InMsg, InMsgDescr,
    MerkleUpdate, Message, MsgEnvelope, OutMsg, OutMsgDescr, OutMsgQueue, OutMsgQueueInfo,
    OutMsgQueueKey, Serializable, ShardAccount, ShardAccountBlocks, ShardAccounts, ShardIdent,
    ShardStateUnsplit, TrComputePhase, TrComputePhaseVm, Transaction, TransactionDescr,
    TransactionDescrOrdinary, UnixTime32, ValueFlow,
};
use ton_executor::{
    BlockchainConfig, ExecuteParams, ExecutorError, OrdinaryTransactionExecutor,
    TransactionExecutor,
};
use ton_types::{error, AccountId, Cell, HashmapRemover, HashmapType, Result, UInt256, SliceData};

use crate::engine::{InMessagesQueue, QueuedMessage};
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
    /// Initialize BlockBuilder
    ///
    pub fn with_params(
        shard_state: Arc<ShardStateUnsplit>,
        prev_ref: BlkPrevInfo,
        block_at: u32,
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

    ///
    /// Initialize BlockBuilder with shard identifier
    ///
    #[cfg(test)]
    pub fn with_shard_ident(
        shard_id: ShardIdent,
        seq_no: u32,
        prev_ref: BlkPrevInfo,
        block_at: u32,
    ) -> Self {
        let mut shard_state = ShardStateUnsplit::with_ident(shard_id);
        if seq_no != 1 {
            shard_state.set_seq_no(seq_no - 1);
        }
        Self::with_params(Arc::new(shard_state), prev_ref, block_at).unwrap()
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
        let tr_cell = transaction.serialize()?;

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

    pub fn build_block(
        mut self,
        queue: &InMessagesQueue,
        blockchain_config: BlockchainConfig,
        debug: bool,
    ) -> NodeResult<Option<(Block, ShardStateUnsplit)>> {
        let mut is_empty = true;

        // first import internal messages
        let mut block_full = false;
        let out_queue = self.out_queue_info.out_queue().clone();
        let msg_count = out_queue.len()?;
        log::debug!(target: "node", "out queue len={}", msg_count);
        let mut sorted = Vec::with_capacity(msg_count);
        for out in out_queue.iter() {
            let (key, mut slice) = out?;
            let key = key.into_cell()?;
            // key is not matter for one shard
            sorted.push((key, OutMsgQueue::value_aug(&mut slice)?));
        }
        sorted.sort_by(|a, b| a.1.1.cmp(&b.1.1));
        for (key, (enq, _create_lt)) in sorted {
            let env = enq.read_out_msg()?;
            let message = env.read_message()?;
            if let Some(acc_id) = message.int_dst_account_id() {
                let msg = QueuedMessage::with_message(message)?;
                self.execute(msg, blockchain_config.clone(), &acc_id, debug)?;
            }
            self.out_queue_info.out_queue_mut().remove(SliceData::load_cell(key)?)?;
            // TODO: check block full
            is_empty = false;
            if self.total_gas_used > 1_000_000 {
                block_full = true;
                break;
            }
        }
        // second import external messages
        while let Some(msg) = queue.dequeue() {
            let acc_id = msg.message().int_dst_account_id().unwrap();

            // lock account in queue
            let res = self.execute(msg, blockchain_config.clone(), &acc_id, debug);
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
                    self.execute(
                        QueuedMessage::with_message(message)?,
                        blockchain_config.clone(),
                        &acc_id,
                        debug,
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
            // let seq_no = self.block_info.seq_no();
            let (block, new_shard_state) = self.finalize_block()?;
            // let block_text = ton_block_json::debug_block_full(&block)?;
            // std::fs::write(&format!("e:\\block_{}.json", seq_no), block_text)?;
            Ok(Some((block, new_shard_state)))
        } else {
            log::debug!(target: "node", "block is empty in messages queue len={}", queue.len());
            Ok(None)
        }
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

    #[cfg(test)]
    pub fn add_test_transaction(&mut self, in_msg: InMsg, out_msgs: &[OutMsg]) -> Result<()> {
        log::debug!("Inserting test transaction");
        let tr_cell = in_msg.transaction_cell().unwrap();
        let transaction = Transaction::construct_from_cell(tr_cell.clone())?;

        self.account_blocks
            .add_serialized_transaction(&transaction, &tr_cell)?;

        self.in_msg_descr
            .set(&in_msg.message_cell()?.repr_hash(), &in_msg, &in_msg.aug()?)?;

        for out_msg in out_msgs {
            let msg_hash = out_msg.read_message_hash()?;
            self.out_msg_descr
                .set(&msg_hash, &out_msg, &out_msg.aug()?)?;
        }
        Ok(())
    }

    ///
    /// Check if BlockBuilder is Empty
    ///
    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.in_msg_descr.is_empty()
            && self.out_msg_descr.is_empty()
            && self.account_blocks.is_empty()
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

        // it can be long, even we don't need to build it
        // let new_ss_root = new_shard_state.serialize()?;
        // let old_ss_root = self.shard_state.serialize()?;
        // let state_update = MerkleUpdate::create(&old_ss_root, &new_ss_root)?;
        let state_update = MerkleUpdate::default();

        let block = Block::with_params(0, self.block_info, value_flow, state_update, block_extra)?;
        Ok((block, new_shard_state))
    }
}
