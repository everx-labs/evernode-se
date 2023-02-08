use crate::data::DocumentsDb;
use crate::engine::shardchain::Shardchain;
use crate::engine::InMessagesQueue;
use crate::error::NodeResult;
use parking_lot::RwLock;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use ton_block::{BinTree, Block, InRefValue, McBlockExtra, Serializable, ShardDescr, ShardIdent};
use ton_executor::BlockchainConfig;
use ton_types::UInt256;

pub struct Masterchain {
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
        storage_path: PathBuf,
    ) -> NodeResult<Self> {
        let shardchain = Shardchain::with_params(
            ShardIdent::masterchain(),
            global_id,
            blockchain_config,
            message_queue,
            documents_db,
            storage_path,
        )?;
        Ok(Self {
            shardchain,
            shards: RwLock::new(HashMap::new()),
            shards_has_been_changed: AtomicBool::new(false),
        })
    }

    pub fn register_new_shard_block(&self, block: &Block) -> NodeResult<()> {
        let info = block.info.read_struct()?;
        let block_cell = block.serialize().unwrap();
        let block_boc = ton_types::cells_serialization::serialize_toc(&block_cell)?;
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
    pub fn generate_block(&self, time_delta: u32, debug: bool) -> NodeResult<Option<Block>> {
        let (mut master_block, new_shard_state, is_empty) =
            self.shardchain.build_block(time_delta, debug)?;

        if is_empty && !self.shards_has_been_changed.load(Ordering::Relaxed) {
            return Ok(None);
        }

        let mut info = master_block.info.read_struct()?;
        info.set_key_block(true);
        master_block.info.write_struct(&info)?;
        let mut extra = master_block.extra.read_struct()?;
        let mut mc_extra = McBlockExtra::default();
        mc_extra.set_config(self.shardchain.blockchain_config.raw_config().clone());
        let shards = mc_extra.shards_mut();
        for (shard, descr) in &*self.shards.read() {
            shards.set(
                &shard.workchain_id(),
                &InRefValue(BinTree::with_item(descr)?),
            )?;
        }

        extra.write_custom(Some(&mc_extra))?;
        master_block.write_extra(&extra)?;
        self.shardchain
            .finality_and_apply_block(&master_block, new_shard_state)?;
        self.shards_has_been_changed.store(false, Ordering::Relaxed);
        log::trace!(target: "node", "master block generated successfully");
        Ok(Some(master_block))
    }
}
