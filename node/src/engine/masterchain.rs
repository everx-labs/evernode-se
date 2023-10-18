use crate::data::{DocumentsDb, NodeStorage, ExternalAccountsProvider};
use crate::engine::shardchain::Shardchain;
use crate::engine::{BlockTimeMode, InMessagesQueue};
use crate::error::NodeResult;
use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use ton_block::{
    BinTree, BinTreeType, Block, InRefValue, McBlockExtra, Serializable, ShardDescr, ShardIdent,
};
use ton_executor::BlockchainConfig;
use ton_types::{SliceData, UInt256, write_boc};

pub struct Masterchain {
    blockchain_config: Arc<BlockchainConfig>,
    shardchain: Shardchain,
    shards: RwLock<HashMap<ShardIdent, ShardDescr>>,
    shards_has_been_changed: AtomicBool,
}

impl Masterchain {
    pub fn with_params(
        global_id: i32,
        blockchain_config: Arc<BlockchainConfig>,
        message_queue: Arc<InMessagesQueue>,
        documents_db: Arc<dyn DocumentsDb>,
        storage: &dyn NodeStorage,
        accounts_provider: Option<Arc<dyn ExternalAccountsProvider>>,
    ) -> NodeResult<Self> {
        let shardchain = Shardchain::with_params(
            ShardIdent::masterchain(),
            global_id,
            blockchain_config.clone(),
            message_queue,
            documents_db,
            storage,
            accounts_provider,
        )?;
        Ok(Self {
            blockchain_config,
            shardchain,
            shards: RwLock::new(HashMap::new()),
            shards_has_been_changed: AtomicBool::new(false),
        })
    }

    pub(crate) fn out_message_queue_is_empty(&self) -> bool {
        self.shardchain.out_message_queue_is_empty()
    }

    pub fn register_new_shard_block(&self, block: &Block) -> NodeResult<()> {
        let info = block.info.read_struct()?;
        let block_cell = block.serialize().unwrap();
        let block_boc = write_boc(&block_cell)?;
        let descr = ShardDescr {
            seq_no: info.seq_no(),
            reg_mc_seqno: 1,
            start_lt: info.start_lt(),
            end_lt: info.end_lt(),
            root_hash: block_cell.repr_hash(),
            file_hash: UInt256::calc_file_hash(&block_boc),
            before_split: info.before_split(),
            // before_merge: info.before_merge,
            want_split: info.want_split(),
            want_merge: info.want_merge(),
            // nx_cc_updated: info.nx_cc_updated,
            flags: info.flags(),
            // next_catchain_seqno: info.next_catchain_seqno,
            next_validator_shard: info.shard().shard_prefix_with_tag(),
            min_ref_mc_seqno: info.min_ref_mc_seqno(),
            gen_utime: info.gen_utime().as_u32(),
            // split_merge_at: info.split_merge_at,
            // fees_collected: info.fees_collected,
            // funds_created: info.funds_created,
            // copyleft_rewards: info.copyleft_rewards,
            // proof_chain: info.proof_chain,
            ..ShardDescr::default()
        };

        self.shards.write().insert(info.shard().clone(), descr);
        self.shards_has_been_changed.store(true, Ordering::Relaxed);
        Ok(())
    }

    ///
    /// Generate new block
    ///
    pub fn generate_block(
        &self,
        time: u32,
        time_mode: BlockTimeMode,
    ) -> NodeResult<Option<Block>> {
        let mut master_block =
            self.shardchain.build_block(time, time_mode)?;

        if master_block.is_empty && !self.shards_has_been_changed.load(Ordering::Relaxed) {
            return Ok(None);
        }

        let mut info = master_block.block.info.read_struct()?;
        info.set_key_block(true);
        master_block.block.info.write_struct(&info)?;
        let mut extra = master_block.block.extra.read_struct()?;
        let mut mc_extra = McBlockExtra::default();
        mc_extra.set_config(self.blockchain_config.raw_config().clone());
        let shards = mc_extra.shards_mut();
        for (shard, descr) in &*self.shards.read() {
            shards.set(
                &shard.workchain_id(),
                &InRefValue(BinTree::with_item(descr)?),
            )?;
        }

        extra.write_custom(Some(&mc_extra))?;
        master_block.block.write_extra(&extra)?;
        self.shardchain
            .finality_and_apply_block(master_block.block.clone(), master_block.state, master_block.transaction_traces)?;
        self.shards_has_been_changed.store(false, Ordering::Relaxed);
        log::trace!(target: "node", "master block generated successfully");
        Ok(Some(master_block.block))
    }

    fn get_last_finalized_mc_extra(&self) -> Option<McBlockExtra> {
        self.shardchain
            .get_last_finalized_block()
            .map_or(None, |block| block.read_extra().ok())
            .map_or(None, |extra| extra.read_custom().ok())
            .flatten()
    }

    pub(crate) fn restore_state(&self) -> NodeResult<()> {
        if !self.shardchain.finality_was_loaded {
            return Ok(());
        }
        if let Some(mc) = self.get_last_finalized_mc_extra() {
            {
                let mut shards = self.shards.write();
                mc.shards().iterate_with_keys(&mut |workchain_id: i32,
                                                     InRefValue(tree): InRefValue<
                    BinTree<ShardDescr>,
                >| {
                    tree.iterate(&mut |shard: SliceData, descr: ShardDescr| {
                        let shard = ShardIdent::with_prefix_slice(
                            workchain_id,
                            shard,
                        )
                        .unwrap();
                        shards.insert(shard, descr);
                        Ok(true)
                    })
                })?;
                self.shards_has_been_changed.store(true, Ordering::Relaxed);
            }
            if let Some(last_config) = mc.config() {
                if last_config != self.blockchain_config.raw_config() {
                    self.generate_block(0, BlockTimeMode::System)?;
                }
            }
        }
        Ok(())
    }
}
