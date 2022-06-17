use ton_block::{Block, ShardIdent, ShardStateUnsplit, Transaction};
use ton_types::{AccountId, Cell, UInt256};
use std::cell::{Cell as StdCell, RefCell};
use std::collections::HashMap;
use std::sync::Arc;
use crate::data::{BlocksStorage, FinalityStorage, ShardStateInfo, ShardStateStorage, TransactionsStorage};
use crate::error::{NodeError, NodeResult};

pub struct TestStorage {
    shard_ident: ShardIdent,
    shard_state: StdCell<ShardStateUnsplit>,
    blocks: RefCell<HashMap<UInt256, Block>>,
    transactions: RefCell<Vec<Transaction>>,
    finality_by_hash: RefCell<HashMap<UInt256, Vec<u8>>>,
    finality_by_no: RefCell<HashMap<u64, Vec<u8>>>,
    finality_by_str: RefCell<HashMap<String, Vec<u8>>>,
}

impl TestStorage {
    #[allow(dead_code)]
    pub fn new(shard_ident: ShardIdent) -> Self {
        TestStorage {
            shard_ident,
            shard_state: StdCell::new(ShardStateUnsplit::default()),
            blocks: RefCell::new(HashMap::new()),
            transactions: RefCell::new(Vec::new()),
            finality_by_hash: RefCell::new(HashMap::new()),
            finality_by_no: RefCell::new(HashMap::new()),
            finality_by_str: RefCell::new(HashMap::new()),
        }
    }

    ///
    /// Get hash-identifier form shard ident and sequence numbers
    ///
    fn get_hash_from_ident_and_seq(
        shard_ident: &ShardIdent,
        seq_no: u32,
        vert_seq_no: u32,
    ) -> UInt256 {
        let mut hash = vec![];
        // TODO: check here
        hash.extend_from_slice(&(shard_ident.shard_prefix_with_tag()).to_be_bytes());
        hash.extend_from_slice(&(seq_no).to_be_bytes());
        hash.extend_from_slice(&(vert_seq_no).to_be_bytes());
        UInt256::from_slice(&hash)
    }
}

impl ShardStateStorage for TestStorage {
    fn shard_state(&self) -> NodeResult<ShardStateUnsplit> {
        let ss = self.shard_state.take();
        self.shard_state.set(ss.clone());
        Ok(ss)
    }
    fn shard_bag(&self) -> NodeResult<Cell> {
        let ss = self.shard_state.take();
        self.shard_state.set(ss.clone());
        Ok(Cell::default())
    }
    fn save_shard_state(&self, shard_state: &ShardStateUnsplit) -> NodeResult<()> {
        self.shard_state.set(shard_state.clone());
        Ok(())
    }

    fn serialized_shardstate(&self) -> NodeResult<Vec<u8>> {
        Ok(vec![])
    }
    fn save_serialized_shardstate(&self, _data: Vec<u8>) -> NodeResult<()> {
        Ok(())
    }
    fn save_serialized_shardstate_ex(
        &self,
        _shard_state: &ShardStateUnsplit,
        _shard_data: Option<Vec<u8>>,
        _shard_hash: &UInt256,
        _shard_state_info: ShardStateInfo,
    ) -> NodeResult<()> {
        Ok(())
    }
}

impl BlocksStorage for TestStorage {
    ///
    /// Get block from memory storage by ID
    ///
    fn block(&self, seq_no: u32, vert_seq_no: u32) -> NodeResult<Block> {
        let hash = Self::get_hash_from_ident_and_seq(&self.shard_ident, seq_no, vert_seq_no);
        match self.blocks.borrow().get(&hash) {
            Some(b) => Ok(b.clone()),
            _ => Err(NodeError::NotFound),
        }
    }

    fn raw_block(&self, _seq_no: u32, _vert_seq_no: u32) -> NodeResult<Vec<u8>> {
        Ok(vec![])
    }

    ///
    /// Save block to memory storage
    ///
    fn save_block(&self, block: &Block, root_hash: UInt256) -> NodeResult<()> {
        self.blocks
            .try_borrow_mut()
            .unwrap()
            .insert(root_hash, block.clone());
        Ok(())
    }

    fn save_raw_block(
        &self,
        _block: &Block,
        _block_data: Option<&Vec<u8>>,
    ) -> NodeResult<()> {
        log::info!(target: "node", "save block with seq_no: {}", _block.read_info()?.seq_no());
        Ok(())
    }
}

impl TransactionsStorage for TestStorage {
    fn save_transaction(&self, tr: Arc<Transaction>) -> NodeResult<()> {
        self.transactions.borrow_mut().push((*tr).clone());
        Ok(())
    }
    fn find_by_lt(&self, lt: u64, _acc_id: &AccountId) -> NodeResult<Option<Transaction>> {
        for tr in self.transactions.borrow().iter() {
            if tr.logical_time() == lt {
                return Ok(Some(tr.clone()));
            }
        }
        Ok(None)
    }
}

impl FinalityStorage for TestStorage {
    fn save_non_finalized_block(
        &self,
        hash: UInt256,
        seq_no: u64,
        data: Vec<u8>,
    ) -> NodeResult<()> {
        println!("save block {:?}", hash);
        self.finality_by_hash
            .try_borrow_mut()
            .unwrap()
            .insert(hash, data.clone());
        self.finality_by_no
            .try_borrow_mut()
            .unwrap()
            .insert(seq_no, data);
        Ok(())
    }
    fn load_non_finalized_block_by_seq_no(&self, seq_no: u64) -> NodeResult<Vec<u8>> {
        println!("load block {:?}", seq_no);
        if self.finality_by_no.borrow().contains_key(&seq_no) {
            Ok(self
                .finality_by_no
                .try_borrow_mut()
                .unwrap()
                .get(&seq_no)
                .unwrap()
                .clone())
        } else {
            Err(NodeError::NotFound)
        }
    }
    fn load_non_finalized_block_by_hash(&self, hash: UInt256) -> NodeResult<Vec<u8>> {
        println!("load block {:?}", hash);
        if self.finality_by_hash.borrow().contains_key(&hash) {
            Ok(self
                .finality_by_hash
                .try_borrow_mut()
                .unwrap()
                .get(&hash)
                .unwrap()
                .clone())
        } else {
            Err(NodeError::NotFound)
        }
    }
    fn remove_form_finality_storage(&self, hash: UInt256) -> NodeResult<()> {
        println!("remove block {:?}", hash);
        self.finality_by_hash
            .try_borrow_mut()
            .unwrap()
            .remove(&hash)
            .unwrap();
        Ok(())
    }
    fn save_custom_finality_info(&self, key: String, data: Vec<u8>) -> NodeResult<()> {
        println!("save custom {}", key);
        self.finality_by_str
            .try_borrow_mut()
            .unwrap()
            .insert(key, data);
        Ok(())
    }
    fn load_custom_finality_info(&self, key: String) -> NodeResult<Vec<u8>> {
        println!("load custom {}", key);
        if self.finality_by_str.borrow().contains_key(&key) {
            Ok(self
                .finality_by_str
                .try_borrow_mut()
                .unwrap()
                .remove(&key)
                .unwrap())
        } else {
            Err(NodeError::NotFound)
        }
    }
}
