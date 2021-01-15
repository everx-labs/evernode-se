
use super::*;
use std::sync::Arc;
use ton_block::{ShardStateUnsplit, Deserializable};
//use console::style;

/// Applies changes provided by new block to shard state (in memory instance)
/// and saves all changes into all kinds of storages
#[allow(dead_code)]
pub struct NewBlockApplier<F> where
    F: BlockFinality
{
    db: Arc<Box<dyn DocumentsDb>>,
    finality: Arc<Mutex<F>>,
}

impl<F> NewBlockApplier<F> where
    F: BlockFinality
{
    /// Create new NewBlockApplier with given storages and shard state
    pub fn with_params(finality: Arc<Mutex<F>>, db: Arc<Box<dyn DocumentsDb>>) -> Self {
        
        NewBlockApplier {
            finality,
            db,
        }
    }    

    /// Applyes changes provided by given block, returns new shard state
    pub fn apply(&mut self, 
        block: SignedBlock, 
        block_data: Option<Vec<u8>>,
        finality_hash: Vec<UInt256>,
        applied_shard: Option<ShardStateUnsplit>,
        is_sync: bool,
        ) -> NodeResult<Arc<ShardStateUnsplit>> {

        let mut finality = self.finality.lock();        
        let new_shard_state: Arc<ShardStateUnsplit>;

        if let Some(applied_shard_state) = applied_shard {            
            new_shard_state = Arc::new(applied_shard_state);
        } else {
            // update shard state's bag of cells by Merkle update
//            info!(target: "node", "updating shard state by Merkle update {:?}", block.block().state_update);
            info!(target: "node", "updating shard state by Merkle update");

            let old_ss_cell = finality.get_last_shard_state().serialize()?;

            let new_ss_cell = block.block().read_state_update()?.apply_for(&old_ss_cell)
                .map_err(|err| {
                    warn!(target: "node", "{}", err.to_string());
                    NodeError::from_kind(NodeErrorKind::InvalidMerkleUpdate)
                })?;

            new_shard_state = Arc::new(ShardStateUnsplit::construct_from(&mut SliceData::from(new_ss_cell))?);
        }
        let root_hash = block.block().hash().unwrap();

        info!(target: "node", "Apply block. finality hashes = {:?}", finality_hash);

        finality.put_block_with_info(
            block, 
            block_data,
            Some(root_hash), 
            new_shard_state.clone(), 
            finality_hash,
            is_sync
        )?;

        Ok(new_shard_state)
    }

}