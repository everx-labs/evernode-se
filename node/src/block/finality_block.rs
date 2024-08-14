use crate::error::NodeResult;
use ever_block::{
    Block, ByteOrderRead, Deserializable, Serializable, ShardIdent, ShardStateUnsplit, SliceData,
    UInt256,
};
use std::collections::HashMap;
use std::io::{Read, Seek};
use std::sync::Arc;

use super::builder::EngineTraceInfoData;

#[derive(Clone, Debug)]
pub enum FinalityBlock {
    Loaded(Box<ShardBlock>),
    Stored(Box<ShardBlockHash>),
}

impl FinalityBlock {
    pub fn seq_no(&self) -> u64 {
        match self {
            FinalityBlock::Stored(sb) => sb.seq_no,
            FinalityBlock::Loaded(sb) => sb.seq_no,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ShardBlockHash {
    pub(crate) seq_no: u64,
    pub(crate) root_hash: UInt256,
}

impl ShardBlockHash {
    pub fn with_hash(seq_no: u64, hash: UInt256) -> Self {
        Self {
            seq_no,
            root_hash: hash,
        }
    }
}

/// Structure for store one block and his ShardState
#[derive(Clone, Debug)]
pub struct ShardBlock {
    pub(crate) seq_no: u64,
    pub(crate) serialized_block: Vec<u8>,
    pub(crate) root_hash: UInt256,
    pub(crate) file_hash: UInt256,
    pub(crate) block: Block,
    pub(crate) shard_state: Arc<ShardStateUnsplit>,
    pub(crate) transaction_traces: HashMap<UInt256, Vec<EngineTraceInfoData>>,
}

#[cfg(test)]
impl PartialEq for ShardBlock {
    fn eq(&self, other: &Self) -> bool {
        self.seq_no == other.seq_no
            && self.serialized_block == other.serialized_block
            && self.root_hash == other.root_hash
            && self.file_hash == other.file_hash
            && self.block == other.block
            && self.shard_state == other.shard_state
    }
}

impl ShardBlock {
    pub(crate) fn new(global_id: i32, shard: ShardIdent) -> Self {
        let mut shard_state = ShardStateUnsplit::default();
        shard_state.set_global_id(global_id);
        shard_state.set_shard(shard);
        let mut block = Block::default();
        block.global_id = global_id;
        Self {
            seq_no: 0,
            serialized_block: Vec::new(),
            root_hash: UInt256::ZERO,
            file_hash: UInt256::ZERO,
            block,
            shard_state: Arc::new(shard_state),
            transaction_traces: HashMap::new(),
        }
    }

    /// get current block sequence number
    pub fn get_seq_no(&self) -> u64 {
        self.seq_no
    }

    /// Create new instance of shard block with Block and new shard state
    pub fn with_block_and_state(block: Block, shard_state: Arc<ShardStateUnsplit>) -> Self {
        let cell = block.serialize().unwrap();
        let root_hash = cell.repr_hash();

        let serialized_block = ever_block::write_boc(&cell).unwrap();
        let file_hash = UInt256::calc_file_hash(&serialized_block);
        let info = block.read_info().unwrap();

        Self {
            seq_no: key_by_seqno(info.seq_no(), info.vert_seq_no()),
            serialized_block,
            root_hash,
            file_hash,
            block,
            shard_state,
            transaction_traces: HashMap::new(),
        }
    }

    /// serialize shard block (for save on disk)
    pub fn serialize(&self) -> NodeResult<Vec<u8>> {
        let mut buf = Vec::new();
        buf.extend_from_slice(&self.seq_no.to_le_bytes());
        buf.extend_from_slice(&(self.serialized_block.len() as u32).to_le_bytes());
        buf.extend_from_slice(self.serialized_block.as_slice());
        buf.extend_from_slice(self.root_hash.as_slice());
        buf.extend_from_slice(self.file_hash.as_slice());

        buf.append(&mut self.shard_state.write_to_bytes()?);

        let mut block_buf = self.block.write_to_bytes()?;
        buf.append(&mut block_buf);
        Ok(buf)
    }

    /// deserialize shard block
    pub fn deserialize<R: Read + Seek>(rdr: &mut R) -> NodeResult<Self> {
        let mut sb = ShardBlock::new(0, ShardIdent::default());
        sb.seq_no = rdr.read_le_u64()?;
        let sb_len = rdr.read_le_u32()?;
        let mut sb_buf = vec![0; sb_len as usize];
        rdr.read(&mut sb_buf)?;
        sb.serialized_block = sb_buf;

        let hash = rdr.read_u256()?;
        sb.root_hash = UInt256::from(hash);

        let hash = rdr.read_u256()?;
        sb.file_hash = UInt256::from(hash);

        let mut shard_slice = SliceData::load_cell(
            ever_block::BocReader::new()
                .read(rdr)?
                .withdraw_single_root()?,
        )?;

        sb.shard_state.read_from(&mut shard_slice)?;

        sb.block = Block::construct_from_cell(
            ever_block::BocReader::new()
                .read(rdr)?
                .withdraw_single_root()?,
        )?;
        Ok(sb)
    }
}

pub fn key_by_seqno(seq_no: u32, vert_seq_no: u32) -> u64 {
    ((vert_seq_no as u64) << 32) | (seq_no as u64)
}
