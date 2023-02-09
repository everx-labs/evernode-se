use crate::block::{BlockBuilder, OrdinaryBlockFinality};
use crate::data::{DocumentsDb, FileBasedStorage};
use crate::engine::InMessagesQueue;
use crate::error::{NodeError, NodeResult};
use parking_lot::Mutex;
use std::io::ErrorKind;
use std::path::PathBuf;
use std::sync::Arc;
use ton_block::{Block, ShardIdent, ShardStateUnsplit, UnixTime32};
use ton_executor::BlockchainConfig;
use ton_types::HashmapType;

type Storage = FileBasedStorage;
type ArcBlockFinality = Arc<Mutex<OrdinaryBlockFinality<Storage, Storage, Storage, Storage>>>;

pub struct Shardchain {
    pub(crate) finality_was_loaded: bool,
    pub(crate) shard_ident: ShardIdent,
    pub(crate) blockchain_config: Arc<BlockchainConfig>,
    message_queue: Arc<InMessagesQueue>,
    pub(crate) finalizer: ArcBlockFinality,
}

impl Shardchain {
    pub fn with_params(
        shard: ShardIdent,
        global_id: i32,
        blockchain_config: Arc<BlockchainConfig>,
        message_queue: Arc<InMessagesQueue>,
        documents_db: Arc<dyn DocumentsDb>,
        storage_path: PathBuf,
    ) -> NodeResult<Self> {
        let storage = Arc::new(Storage::with_path(shard.clone(), storage_path.clone())?);
        let block_finality = Arc::new(Mutex::new(OrdinaryBlockFinality::with_params(
            global_id,
            shard.clone(),
            storage_path,
            storage.clone(),
            storage.clone(),
            storage.clone(),
            storage.clone(),
            Some(documents_db.clone()),
            Vec::new(),
        )));
        let finality_was_loaded = match block_finality.lock().load() {
            Ok(_) => {
                log::info!(target: "node", "load block finality successfully");
                true
            }
            Err(NodeError::Io(err)) => {
                if err.kind() != ErrorKind::NotFound {
                    return Err(NodeError::Io(err));
                }
                false
            }
            Err(err) => {
                return Err(err);
            }
        };

        Ok(Self {
            finality_was_loaded,
            shard_ident: shard.clone(),
            blockchain_config,
            message_queue,
            finalizer: block_finality.clone(),
        })
    }

    pub(crate) fn build_block(
        &self,
        time_delta: u32,
        debug: bool,
    ) -> NodeResult<(Block, ShardStateUnsplit, bool)> {
        let timestamp = UnixTime32::now().as_u32() + time_delta;
        let (shard_state, blk_prev_info) = self.finalizer.lock().get_last_info()?;
        log::debug ! (target: "node", "PARENT block: {:?}", blk_prev_info);

        let collator = BlockBuilder::with_params(shard_state, blk_prev_info, timestamp)?;
        collator.build_block(&self.message_queue, &self.blockchain_config, debug)
    }

    ///
    /// Generate new block if possible
    ///
    pub fn generate_block(&self, time_delta: u32, debug: bool) -> NodeResult<Option<Block>> {
        let (block, new_shard_state, is_empty) = self.build_block(time_delta, debug)?;
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
        self.finalizer
            .lock()
            .put_block_with_info(block, new_state.clone())?;
        Ok(new_state)
    }
}
