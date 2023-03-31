use crate::block::{BlockBuilder, BlockFinality};
use crate::data::{DocumentsDb, NodeStorage, ShardStorage};
use crate::engine::{BlockTimeMode, InMessagesQueue};
use crate::error::NodeResult;
use parking_lot::Mutex;
use std::sync::Arc;
use ton_block::{Block, ShardIdent, ShardStateUnsplit};
use ton_executor::BlockchainConfig;
use ton_types::HashmapType;

pub struct Shardchain {
    pub(crate) finality_was_loaded: bool,
    blockchain_config: Arc<BlockchainConfig>,
    message_queue: Arc<InMessagesQueue>,
    block_finality: Arc<Mutex<BlockFinality>>,
    debug_mode: bool,
}

impl Shardchain {
    pub fn with_params(
        shard: ShardIdent,
        global_id: i32,
        blockchain_config: Arc<BlockchainConfig>,
        message_queue: Arc<InMessagesQueue>,
        documents_db: Arc<dyn DocumentsDb>,
        storage: &dyn NodeStorage,
        debug_mode: bool,
    ) -> NodeResult<Self> {
        let block_finality = Arc::new(Mutex::new(BlockFinality::with_params(
            global_id,
            shard.clone(),
            ShardStorage::new(storage.shard_storage(shard.clone())?),
            Some(documents_db.clone()),
        )));
        let finality_was_loaded = block_finality.lock().load()?;
        if finality_was_loaded {
            log::info!(target: "node", "load block finality successfully");
        };

        Ok(Self {
            finality_was_loaded,
            blockchain_config,
            message_queue,
            block_finality: block_finality.clone(),
            debug_mode,
        })
    }

    pub(crate) fn out_message_queue_is_empty(&self) -> bool {
        self.block_finality.lock().out_message_queue_is_empty()
    }

    pub(crate) fn build_block(
        &self,
        time: u32,
        time_mode: BlockTimeMode,
    ) -> NodeResult<(Block, ShardStateUnsplit, bool)> {
        let (shard_state, blk_prev_info) = self.block_finality.lock().get_last_info()?;
        log::debug ! (target: "node", "PARENT block: {:?}", blk_prev_info);

        let collator = BlockBuilder::with_params(shard_state, blk_prev_info, time, time_mode)?;
        collator.build_block(
            &self.message_queue,
            &self.blockchain_config,
            self.debug_mode,
        )
    }

    ///
    /// Generate new block if possible
    ///
    pub fn generate_block(&self, time: u32, time_mode: BlockTimeMode) -> NodeResult<Option<Block>> {
        let (block, new_shard_state, is_empty) = self.build_block(time, time_mode)?;
        Ok(if !is_empty {
            log::trace!(target: "node", "block generated successfully");
            Self::print_block_info(&block);
            self.finality_and_apply_block(&block, new_shard_state)?;
            Some(block)
        } else {
            log::trace!(target: "node", "empty block was not generated");
            None
        })
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

    /// finality and apply block
    pub(crate) fn finality_and_apply_block(
        &self,
        block: &Block,
        applied_shard: ShardStateUnsplit,
    ) -> NodeResult<Arc<ShardStateUnsplit>> {
        log::info!(target: "node", "Apply block seq_no = {}", block.read_info()?.seq_no());
        let new_state = Arc::new(applied_shard);
        self.block_finality
            .lock()
            .put_block_with_info(block, new_state.clone())?;
        Ok(new_state)
    }

    /// get last finalized block
    pub fn get_last_finalized_block(&self) -> NodeResult<Block> {
        Ok(self
            .block_finality
            .lock()
            .last_finalized_block
            .block
            .clone())
    }
}
