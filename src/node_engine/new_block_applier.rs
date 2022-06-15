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
use std::sync::Arc;
use ton_block::ShardStateUnsplit;
/// Trait for save finality states blockchain
pub trait BlockFinality {
    fn put_block_with_info(
        &mut self,
        signed_block: &SignedBlock,
        signed_block_data: Option<Vec<u8>>,
        block_hash: Option<UInt256>,
        shard_state: Arc<ShardStateUnsplit>,
        finality_hashes: Vec<UInt256>,
        is_sync: bool,
    ) -> NodeResult<()>;

    fn get_last_seq_no(&self) -> u32;

    fn get_last_block_info(&self) -> NodeResult<BlkPrevInfo>;

    fn get_last_shard_state(&self) -> Arc<ShardStateUnsplit>;

    fn find_block_by_hash(&self, hash: &UInt256) -> u64;

    fn rollback_to(&mut self, hash: &UInt256) -> NodeResult<()>;

    fn get_raw_block_by_seqno(&self, seq_no: u32, vert_seq_no: u32) -> NodeResult<Vec<u8>>;

    fn get_last_finality_shard_hash(&self) -> NodeResult<(u64, UInt256)>;

    fn reset(&mut self) -> NodeResult<()>;
}


/// Applies changes provided by new block to shard state (in memory instance)
/// and saves all changes into all kinds of storages
#[allow(dead_code)]
pub struct NewBlockApplier<F> where
    F: BlockFinality
{
    db: Arc<dyn DocumentsDb>,
    finality: Arc<Mutex<F>>,
}

impl<F> NewBlockApplier<F> where
    F: BlockFinality
{
    /// Create new NewBlockApplier with given storages and shard state
    pub fn with_params(finality: Arc<Mutex<F>>, db: Arc<dyn DocumentsDb>) -> Self {

        NewBlockApplier {
            finality,
            db,
        }
    }

    /// Applies changes provided by given block, returns new shard state
    pub fn apply(&mut self,
        block: &SignedBlock,
        block_data: Option<Vec<u8>>,
        finality_hash: Vec<UInt256>,
        applied_shard: ShardStateUnsplit,
        is_sync: bool,
        ) -> NodeResult<Arc<ShardStateUnsplit>> {

        let mut finality = self.finality.lock();
        let new_shard_state = Arc::new(applied_shard);
        let root_hash = block.block().hash().unwrap();

        log::info!(target: "node", "Apply block. finality hashes = {:?}", finality_hash);

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
