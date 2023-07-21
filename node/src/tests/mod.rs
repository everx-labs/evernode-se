use crate::block::{BlockBuilder, BlockFinality, FinalityBlock};
use crate::config::NodeConfig;
use crate::data::{shard_storage_key, ShardStateInfo, ShardStorage};
use crate::engine::BlockTimeMode;
use crate::error::{NodeError, NodeResult};
use crate::{blockchain_config_from_json, MemDocumentsDb, MemStorage};
use ed25519_dalek::PublicKey;
use serde_json::Value;
use std::fs;
use std::io::Read;
use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;
use ton_block::{
    AccountStatus, Augmentation, BlkPrevInfo, Block, CurrencyCollection, Deserializable,
    ExtOutMessageHeader, ExternalInboundMessageHeader, HashmapAugType, InMsg,
    InternalMessageHeader, Message, MsgAddressExt, MsgAddressInt, OutMsg, Serializable, ShardIdent,
    ShardStateUnsplit, Transaction, UnixTime32,
};
use ton_executor::BlockchainConfig;
use ton_types::{
    AccountId, ByteOrderRead, Cell, HashmapType, SliceData, UInt256,
};

mod abi_account;
mod test_block_builder;
mod test_block_finality;
mod test_file_based_storage;
mod test_messages;
mod test_node_se;
mod test_time_control;

pub fn get_config_params(json: &str) -> (NodeConfig, Vec<PublicKey>) {
    match NodeConfig::parse(json) {
        Ok(config) => match import_keys(&config) {
            Ok(keys) => (config, keys),
            Err(err) => {
                log::warn!(target: "node", "{}", err);
                panic!("{} / {}", err, json)
            }
        },
        Err(err) => {
            log::warn!(target: "node", "Error parsing configuration file. {}", err);
            panic!("Error parsing configuration file. {}", err)
        }
    }
}

// runs 10 thread to generate N accounts with 1 input and two output messages per every block
// finalizes block and return
pub fn generate_block_with_seq_no(
    shard_ident: ShardIdent,
    seq_no: u32,
    prev_info: BlkPrevInfo,
    account_count: usize,
) -> Block {
    let mut block_builder =
        builder_with_shard_ident(shard_ident, seq_no, prev_info, UnixTime32::now().as_u32());

    //println!("Thread write start.");
    for _ in 0..account_count {
        let acc = AccountId::from_raw(
            (0..32).map(|_| rand::random::<u8>()).collect::<Vec<u8>>(),
            256,
        );
        let mut transaction =
            Transaction::with_address_and_status(acc.clone(), AccountStatus::AccStateActive);
        let mut value = CurrencyCollection::default();
        value.grams = 10202u64.into();
        let mut imh = InternalMessageHeader::with_addresses(
            MsgAddressInt::with_standart(None, 0, acc.clone()).unwrap(),
            MsgAddressInt::with_standart(
                None,
                0,
                AccountId::from_raw(
                    (0..32).map(|_| rand::random::<u8>()).collect::<Vec<u8>>(),
                    256,
                ),
            )
            .unwrap(),
            value,
        );

        imh.ihr_fee = 10u64.into();
        imh.fwd_fee = 5u64.into();
        let mut in_msg_1 = Message::with_int_header(imh);
        in_msg_1.set_body(SliceData::new(vec![0x21; 120]));

        let ext_in_header = ExternalInboundMessageHeader {
            src: MsgAddressExt::with_extern(SliceData::new(vec![0x23, 0x52, 0x73, 0x00, 0x80]))
                .unwrap(),
            dst: MsgAddressInt::with_standart(None, 0, acc.clone()).unwrap(),
            import_fee: 10u64.into(),
        };

        let mut in_msg = Message::with_ext_in_header(ext_in_header);
        in_msg.set_body(SliceData::new(vec![0x01; 120]));

        transaction.write_in_msg(Some(&in_msg_1)).unwrap();

        // out_msgs
        let mut value = CurrencyCollection::default();
        value.grams = 10202u64.into();
        let mut imh = InternalMessageHeader::with_addresses(
            MsgAddressInt::with_standart(None, 0, acc.clone()).unwrap(),
            MsgAddressInt::with_standart(None, 0, AccountId::from_raw(vec![255; 32], 256)).unwrap(),
            value,
        );

        imh.ihr_fee = 10u64.into();
        imh.fwd_fee = 5u64.into();
        let out_msg_1 = Message::with_int_header(imh);

        let ext_out_header = ExtOutMessageHeader::with_addresses(
            MsgAddressInt::with_standart(None, 0, acc.clone()).unwrap(),
            MsgAddressExt::with_extern(SliceData::new(vec![0x23, 0x52, 0x73, 0x00, 0x80])).unwrap(),
        );

        let mut out_msg_2 = Message::with_ext_out_header(ext_out_header);
        out_msg_2.set_body(SliceData::new(vec![0x02; 120]));

        transaction.add_out_message(&out_msg_1).unwrap();
        transaction.add_out_message(&out_msg_2).unwrap();

        let tr_cell = transaction.serialize().unwrap();
        block_builder
            .add_raw_transaction(transaction, tr_cell)
            .unwrap();
    }
    let (block, _) = block_builder.finalize_block().unwrap();
    block
}

pub(crate) fn shard_state(storage: &ShardStorage) -> NodeResult<ShardStateUnsplit> {
    let cell = shard_bag(storage)?;
    Ok(ShardStateUnsplit::construct_from_cell(cell)?)
}

fn shard_bag(storage: &ShardStorage) -> NodeResult<Cell> {
    let data = storage.get(shard_storage_key::SHARD_STATE_BLOCK_KEY)?;
    // TODO: BOC from file
    Ok(ton_types::boc::read_single_root_boc(&data)?)
}

///
/// Save shard state to file
///
pub(crate) fn save_shard_state(
    storage: &ShardStorage,
    shard_state: &ShardStateUnsplit,
) -> NodeResult<()> {
    let data = shard_state.write_to_bytes()?;
    storage.set(shard_storage_key::SHARD_STATE_BLOCK_KEY, data, true)?;
    Ok(())
}

// Blocks

///
/// Get selected block from shard storage
///
pub(crate) fn get_block(
    storage: &ShardStorage,
    seq_no: u32,
    vert_seq_no: u32,
) -> NodeResult<Block> {
    Ok(Block::construct_from_bytes(&storage.get(
        &shard_storage_key::block_key(seq_no, vert_seq_no),
    )?)?)
}

///
/// Save block to file based shard storage
///
pub(crate) fn save_block(
    storage: &ShardStorage,
    block: &Block,
    root_hash: UInt256,
) -> NodeResult<()> {
    let info = block.read_info()?;
    let ssi = shard_state_info_with_params(
        key_by_seqno(info.seq_no(), info.vert_seq_no()),
        info.end_lt(),
        root_hash,
    );
    storage.set(shard_storage_key::SHARD_INFO_KEY, ssi.serialize(), true)?;
    storage.set(
        &shard_storage_key::block_key(info.seq_no(), info.vert_seq_no()),
        block.write_to_bytes()?,
        true,
    )
}

pub(crate) fn key_by_seqno(seq_no: u32, vert_seq_no: u32) -> u64 {
    ((vert_seq_no as u64) << 32) & (seq_no as u64)
}

pub fn shard_state_info_with_params(seq_no: u64, lt: u64, hash: UInt256) -> ShardStateInfo {
    ShardStateInfo { seq_no, lt, hash }
}

pub fn shard_state_info_deserialize<R: Read>(rdr: &mut R) -> NodeResult<ShardStateInfo> {
    let seq_no = rdr.read_be_u64()?;
    let lt = rdr.read_be_u64()?;
    let hash = UInt256::from(rdr.read_u256()?);
    Ok(ShardStateInfo { seq_no, lt, hash })
}

/// Import key values from list of files
pub fn import_keys(config: &NodeConfig) -> Result<Vec<PublicKey>, String> {
    let mut ret = Vec::new();
    for path in config.keys.iter() {
        let data = fs::read(Path::new(path))
            .map_err(|e| format!("Error reading key file {}, {}", path, e))?;
        ret.push(
            PublicKey::from_bytes(&data)
                .map_err(|e| format!("Cannot import key from {}, {}", path, e))?,
        );
    }
    Ok(ret)
}

///
/// Initialize BlockBuilder with shard identifier
///
pub fn builder_with_shard_ident(
    shard_id: ShardIdent,
    seq_no: u32,
    prev_ref: BlkPrevInfo,
    block_at: u32,
) -> BlockBuilder {
    let mut shard_state = ShardStateUnsplit::with_ident(shard_id);
    if seq_no != 1 {
        shard_state.set_seq_no(seq_no - 1);
    }
    BlockBuilder::with_params(
        Arc::new(shard_state),
        prev_ref,
        block_at,
        BlockTimeMode::System,
        1_000_000,
    )
    .unwrap()
}

pub fn builder_add_test_transaction(
    builder: &mut BlockBuilder,
    in_msg: InMsg,
    out_msgs: &[OutMsg],
) -> ton_types::Result<()> {
    log::debug!("Inserting test transaction");
    let tr_cell = in_msg.transaction_cell().unwrap();
    let transaction = Transaction::construct_from_cell(tr_cell.clone())?;

    builder
        .account_blocks
        .add_serialized_transaction(&transaction, &tr_cell)?;

    builder
        .in_msg_descr
        .set(&in_msg.message_cell()?.repr_hash(), &in_msg, &in_msg.aug()?)?;

    for out_msg in out_msgs {
        let msg_hash = out_msg.read_message_hash()?;
        builder
            .out_msg_descr
            .set(&msg_hash, &out_msg, &out_msg.aug()?)?;
    }
    Ok(())
}

///
/// Check if BlockBuilder is Empty
///
pub fn builder_is_empty(builder: &BlockBuilder) -> bool {
    builder.in_msg_descr.is_empty()
        && builder.out_msg_descr.is_empty()
        && builder.account_blocks.is_empty()
}

pub fn find_block_by_hash(finality: &BlockFinality, hash: &UInt256) -> u64 {
    if finality.blocks_by_hash.contains_key(hash) {
        finality.blocks_by_hash.get(hash).unwrap().seq_no()
    } else {
        0xFFFFFFFF // if not found
    }
}

/// Rollback shard state to one of block candidates
pub fn rollback_to(finality: &mut BlockFinality, hash: &UInt256) -> NodeResult<()> {
    if finality.blocks_by_hash.contains_key(hash) {
        let sb = finality.blocks_by_hash.remove(hash).unwrap();
        let mut seq_no = sb.seq_no();
        finality.current_block = finality.stored_to_loaded(sb)?;

        // remove from hashmaps all blocks with greater seq_no
        loop {
            let tmp = finality.blocks_by_no.remove(&seq_no);
            if tmp.is_some() {
                finality
                    .blocks_by_hash
                    .remove(finality_block_root_hash(&tmp.unwrap()));
            } else {
                break;
            }
            seq_no += 1;
        }

        Ok(())
    } else {
        Err(NodeError::RoolbackBlockError)
    }
}

pub fn hexdump(d: &[u8]) {
    let mut str = String::new();
    for i in 0..d.len() {
        str.push_str(&format!(
            "{:02x}{}",
            d[i],
            if (i + 1) % 16 == 0 { '\n' } else { ' ' }
        ));
    }

    log::debug!(target: "node", "{}", str);
}

pub fn finality_block_root_hash(fb: &FinalityBlock) -> &UInt256 {
    match fb {
        FinalityBlock::Stored(sb) => &sb.root_hash,
        FinalityBlock::Loaded(sb) => &sb.root_hash,
    }
}

const ZEROSTATE: &[u8] =
    include_bytes!("../../../docker/ton-node/workchains/WC0/shard_8000000000000000/zerostate");

pub fn mem_storage() -> MemStorage {
    MemStorage::new(ZEROSTATE.to_vec())
}

const BLOCKCHAIN_CONFIG_JSON: &str = include_str!("../../../docker/ton-node/blockchain.conf.json");

pub fn blockchain_config() -> BlockchainConfig {
    blockchain_config_from_json(BLOCKCHAIN_CONFIG_JSON).unwrap()
}

struct DocsReader {
    db: Arc<MemDocumentsDb>,
    blocks_passed: usize,
    transactions_passed: usize,
}

impl DocsReader {
    pub fn new(db: Arc<MemDocumentsDb>) -> Self {
        Self {
            db,
            blocks_passed: 0,
            transactions_passed: 0,
        }
    }

    pub fn next_blocks(&mut self) -> Vec<Value> {
        let blocks = self.db.blocks().split_off(self.blocks_passed);
        self.blocks_passed += blocks.len();
        blocks
    }

    pub fn next_transactions(&mut self) -> Vec<Value> {
        let transactions = self.db.transactions().split_off(self.transactions_passed);
        self.transactions_passed += transactions.len();
        transactions
    }

    pub fn dump_next(&mut self) {
        Self::dump(
            self.next_blocks(),
            "blocks",
            "gen_utime,workchain_id,seq_no,id",
        );
        Self::dump(
            self.next_transactions(),
            "transactions",
            "now,account_addr,lt,id,aborted,in_msg,out_msgs",
        );
    }

    pub fn dump(items: Vec<Value>, title: &str, fields: &str) {
        println!("{}:", title);
        for item in items {
            let mut s = String::new();
            for field in fields.split(',') {
                let value = item.get(field);
                if !s.is_empty() {
                    s.push_str(", ");
                }
                s.push_str(&value.map_or_else(|| String::new(), |x| x.to_string()))
            }
            println!("{}", s);
        }
    }
}

fn parse_address(string: &str) -> NodeResult<MsgAddressInt> {
    match MsgAddressInt::from_str(string) {
        Ok(address) => Ok(address),
        Err(err) => Err(NodeError::InvalidData(format!(
            "Invalid address [{}]: {}",
            string, err
        ))),
    }
}

fn init_logger() {
    log::set_logger(&LOGGER).unwrap();
    // log::set_max_level(LevelFilter::Trace);
}

struct SimpleLogger;

const LOGGER: SimpleLogger = SimpleLogger;

impl log::Log for SimpleLogger {
    fn enabled(&self, _metadata: &log::Metadata) -> bool {
        true
    }

    fn log(&self, record: &log::Record) {
        if !self.enabled(record.metadata()) {
            return;
        }

        if record.target() == "tvm" {
            return;
        }
        println!(
            "{}:{} -- {}",
            record.level(),
            record.target(),
            record.args()
        );
    }
    fn flush(&self) {}
}
