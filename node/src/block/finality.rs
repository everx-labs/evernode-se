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

use crate::block::finality_block::key_by_seqno;
use crate::block::json::reflect_block_in_db;
use crate::block::{FinalityBlock, ShardBlock, ShardBlockHash};
use crate::data::{shard_storage_key, DocumentsDb, ShardStateInfo, ShardStorage};
use crate::error::NodeError;
use crate::NodeResult;
use std::io::{Cursor, ErrorKind, Read, Seek, Write};
use std::{collections::HashMap, sync::Arc};
use ton_block::{
    Account, BlkPrevInfo, Block, Deserializable, ExtBlkRef, Serializable, ShardIdent,
    ShardStateUnsplit,
};
use ton_types::{ByteOrderRead, HashmapType, UInt256};

use super::builder::EngineTraceInfoData;

lazy_static::lazy_static!(
    static ref ACCOUNT_NONE_HASH: UInt256 = Account::default().serialize().unwrap().repr_hash();
);

/// Structure for Block finality layer
/// This realize next functionality:
/// Store block received from POA
/// Finalize block witch POA finality - store to permanent storage
/// Store (in memory) current shard-state for every block-candidate
/// Get current shard-state
/// Get last block by number or hash
/// Rollback block (and shardstate) if block is rolled
#[allow(dead_code)]
pub struct BlockFinality {
    shard_ident: ShardIdent,
    storage: ShardStorage,
    db: Option<Arc<dyn DocumentsDb>>,
    pub(crate) current_block: Box<ShardBlock>,
    pub(crate) blocks_by_hash: HashMap<UInt256, Box<FinalityBlock>>,
    // need to remove
    pub(crate) blocks_by_no: HashMap<u64, Box<FinalityBlock>>,
    // need to remove
    pub(crate) last_finalized_block: Box<ShardBlock>,
}

impl BlockFinality {
    /// Create new instance BlockFinality
    /// with all kind of storages
    pub fn with_params(
        global_id: i32,
        shard_ident: ShardIdent,
        storage: ShardStorage,
        db: Option<Arc<dyn DocumentsDb>>,
    ) -> Self {
        Self {
            shard_ident: shard_ident.clone(),
            storage,
            db,
            current_block: Box::new(ShardBlock::new(global_id, shard_ident.clone())),
            blocks_by_hash: HashMap::new(),
            blocks_by_no: HashMap::new(),
            last_finalized_block: Box::new(ShardBlock::new(global_id, shard_ident.clone())),
        }
    }

    pub(crate) fn out_message_queue_is_empty(&self) -> bool {
        match self.get_last_shard_state().read_out_msg_queue_info() {
            Ok(queue_info) => queue_info.out_queue().is_empty(),
            Err(_) => true,
        }
    }

    pub(crate) fn get_last_info(&self) -> NodeResult<(Arc<ShardStateUnsplit>, BlkPrevInfo)> {
        Ok((self.get_last_shard_state(), self.get_last_block_info()?))
    }

    fn finality_blocks(&mut self, root_hash: UInt256) -> NodeResult<()> {
        log::debug!("FIN-BLK {:x}", root_hash);
        if let Some(fin_sb) = self.blocks_by_hash.remove(&root_hash) {
            let mut sb = self.stored_to_loaded(fin_sb)?;

            // create shard state info
            let info = sb.block.read_info()?;
            let shard_info = ShardStateInfo {
                seq_no: key_by_seqno(info.seq_no(), info.vert_seq_no()),
                lt: info.end_lt(),
                hash: sb.root_hash.clone(),
            };
            let shard = sb.shard_state.write_to_bytes()?;
            // save shard state
            self.storage.save_serialized_shardstate_ex(
                &ShardStateUnsplit::default(),
                Some(shard),
                &sb.block.read_state_update()?.new_hash,
                shard_info,
            )?;
            // save finalized block
            self.storage
                .save_raw_block(&sb.block, Some(&sb.serialized_block))?;
            // remove shard-block from number hashmap
            self.blocks_by_no
                .remove(&sb.seq_no)
                .expect("block by number remove error");

            if let Some(db) = &self.db {
                if let Err(err) = reflect_block_in_db(db.clone(), &mut sb) {
                    log::warn!(target: "node", "reflect_block_in_db(Finalized) error: {}", err);
                }
            }

            self.last_finalized_block = sb;
            log::info!(target: "node", "FINALITY:save block seq_no: {:?}, serialized len = {}",
                self.last_finalized_block.block.read_info()?.seq_no(),
                self.last_finalized_block.serialized_block.len()
            );
        } else {
            if root_hash != UInt256::ZERO && root_hash != self.last_finalized_block.root_hash {
                log::warn!(target: "node", "Can`t finality unknown hash!!!");
                return Err(NodeError::FinalityError);
            }
        }
        Ok(())
    }

    pub(crate) fn stored_to_loaded(
        &self,
        fin_sb: Box<FinalityBlock>,
    ) -> NodeResult<Box<ShardBlock>> {
        Ok(match *fin_sb {
            FinalityBlock::Stored(sb_hash) => self.read_one_sb_from_key(
                &shard_storage_key::block_finality_hash_key(sb_hash.root_hash.clone()),
            )?,
            FinalityBlock::Loaded(l_sb) => l_sb,
        })
    }

    pub fn save_finality(&self) -> NodeResult<()> {
        self.storage.save_non_finalized_block(
            self.current_block.root_hash.clone(),
            self.current_block.serialize()?,
        )?;
        self.storage.save_non_finalized_block(
            self.last_finalized_block.root_hash.clone(),
            self.last_finalized_block.serialize()?,
        )?;
        self.save_index()?;
        Ok(())
    }

    fn save_index(&self) -> NodeResult<()> {
        let mut data = Vec::new();
        self.serialize(&mut data)?;
        self.storage
            .set(shard_storage_key::BLOCKS_FINALITY_INFO_KEY, data, true)
    }

    ///
    /// Load from disk
    ///
    pub fn load(&mut self) -> NodeResult<bool> {
        let loaded = if let Ok(data) = self
            .storage
            .get(shard_storage_key::BLOCKS_FINALITY_INFO_KEY)
        {
            self.deserialize(&mut Cursor::new(data))?;
            true
        } else {
            match self.storage.get(shard_storage_key::ZEROSTATE_KEY) {
                Ok(zerostate) => {
                    self.load_zerostate(zerostate)?;
                }
                Err(NodeError::NotFound) => {}
                Err(err) => Err(err)?,
            }
            false
        };
        Ok(loaded)
    }

    ///
    /// Load zerostate
    ///
    fn load_zerostate(&mut self, zerostate: Vec<u8>) -> NodeResult<()> {
        let mut zerostate = ShardStateUnsplit::construct_from_bytes(&zerostate)?;
        zerostate.set_global_id(self.current_block.block.global_id());
        self.current_block.shard_state = Arc::new(zerostate.clone());
        self.last_finalized_block.shard_state = Arc::new(zerostate);
        Ok(())
    }

    ///
    /// Write data BlockFinality to file
    pub(crate) fn serialize(&self, writer: &mut dyn Write) -> NodeResult<()> {
        // serialize struct:
        // 32bit - length of structure ShardBlock
        // structure ShardBlock
        // ...
        // first save current block
        writer.write_all(self.current_block.root_hash.as_slice())?;
        writer.write_all(&self.current_block.seq_no.to_le_bytes())?;
        // save last finality block
        writer.write_all(self.last_finalized_block.root_hash.as_slice())?;
        writer.write_all(&self.last_finalized_block.seq_no.to_le_bytes())?;
        for (key, sb) in self.blocks_by_hash.iter() {
            writer.write_all(key.as_slice())?;
            writer.write_all(&sb.seq_no().to_le_bytes())?;
        }
        Ok(())
    }

    fn read_one_sb_hash<R>(&self, rdr: &mut R) -> NodeResult<(u64, UInt256)>
    where
        R: Read + Seek,
    {
        // first read current block
        let hash = rdr.read_u256()?;
        let seq_no = rdr.read_le_u64()?;
        Ok((seq_no, UInt256::from(hash)))
    }

    fn read_one_sb<R>(&self, rdr: &mut R) -> NodeResult<Box<ShardBlock>>
    where
        R: Read + Seek,
    {
        // first read current block
        let hash = UInt256::from(rdr.read_u256()?);
        let seq_no = rdr.read_le_u64()?;
        log::info!(target: "node", "read_one_sb:seq_no: {}", seq_no);
        if seq_no == 0 {
            Ok(Box::new(ShardBlock::new(
                self.current_block.block.global_id,
                self.shard_ident.clone(),
            )))
        } else {
            self.read_one_sb_from_key(&shard_storage_key::block_finality_hash_key(hash))
        }
    }

    fn read_one_sb_from_key(&self, key: &str) -> NodeResult<Box<ShardBlock>> {
        Ok(Box::new(ShardBlock::deserialize(&mut Cursor::new(
            self.storage.get(key)?,
        ))?))
    }

    ///
    /// Read saved data BlockFinality from file
    ///
    pub fn deserialize<R>(&mut self, rdr: &mut R) -> NodeResult<()>
    where
        R: Read + Seek,
    {
        log::info!(target: "node", "load current block");
        self.current_block = self.read_one_sb(rdr)?;
        log::info!(target: "node", "load last finalized block");
        self.last_finalized_block = self.read_one_sb(rdr)?;
        loop {
            log::info!(target: "node", "load non finalized block");
            match self.read_one_sb_hash(rdr) {
                Ok((seq_no, hash)) => {
                    let sb_hash = Box::new(FinalityBlock::Stored(Box::new(
                        ShardBlockHash::with_hash(seq_no.clone(), hash.clone()),
                    )));
                    self.blocks_by_hash.insert(hash.clone(), sb_hash.clone());
                    self.blocks_by_no.insert(seq_no.clone(), sb_hash.clone());
                }
                Err(NodeError::Io(err)) if err.kind() == ErrorKind::UnexpectedEof => {
                    break;
                }
                Err(err) => return Err(err),
            }
        }
        Ok(())
    }

    /// Save block until finality comes
    pub(crate) fn put_block_with_info(
        &mut self,
        block: Block,
        shard_state: Arc<ShardStateUnsplit>,
        transaction_traces: HashMap<UInt256, Vec<EngineTraceInfoData>>,
    ) -> NodeResult<()> {
        log::info!(target: "node", "FINALITY:    block seq_no: {:?}", block.read_info()?.seq_no());

        let mut sb = Box::new(ShardBlock::with_block_and_state(block, shard_state));
        sb.transaction_traces = transaction_traces;
        log::debug!(target: "node", "PUT-BLOCK-HASH {:?}", sb.root_hash);

        self.current_block = sb.clone();

        // insert block to map
        let sb_hash = Box::new(FinalityBlock::Loaded(sb.clone()));
        self.blocks_by_hash
            .insert(sb.root_hash.clone(), sb_hash.clone());
        self.blocks_by_no.insert(sb.get_seq_no(), sb_hash.clone());
        // finalize block by finality_hash
        self.finality_blocks(sb.root_hash.clone())?;
        self.save_finality()?;
        Ok(())
    }

    /// get last block info
    fn get_last_block_info(&self) -> NodeResult<BlkPrevInfo> {
        let info = &self.current_block.block.read_info()?;
        Ok(BlkPrevInfo::Block {
            prev: ExtBlkRef {
                end_lt: info.end_lt(),
                seq_no: info.seq_no(),
                root_hash: self.current_block.root_hash.clone(), //UInt256::from(self.current_block.block.root_hash().clone()),
                file_hash: self.current_block.file_hash.clone(),
            },
        })
    }

    /// get last shard bag
    fn get_last_shard_state(&self) -> Arc<ShardStateUnsplit> {
        //log::warn!("LAST SHARD BAG {}", self.current_block.shard_bag.get_repr_hash_by_index(0).unwrap().to_hex_string()));
        Arc::clone(&self.current_block.shard_state)
    }
}
