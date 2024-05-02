use crate::block::ShardBlock;
use crate::data::{DocumentsDb, SerializedItem};
use serde_json::Value;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Instant;
use ever_block::{
    Account, BlockIdExt, MsgAddrStd, MsgAddressInt, Serializable,
    read_single_root_boc, Result, UInt256
};
use ever_block_json::{BlockParser, NoReduce, NoTrace, ParsedEntry, ParsingBlock};

lazy_static::lazy_static!(
    static ref ACCOUNT_NONE_HASH: UInt256 = Account::default().serialize().unwrap().repr_hash();
    pub static ref MINTER_ADDRESS: MsgAddressInt =
        MsgAddressInt::AddrStd(MsgAddrStd::with_address(None, 0, [0; 32].into()));
);

pub fn reflect_block_in_db(
    db: Arc<dyn DocumentsDb>,
    parser: &BlockParser<NoTrace, NoReduce>,
    mc_seq_no: u32,
    shard_block: &mut ShardBlock,
) -> Result<()> {
    let now = Instant::now();
    let root = read_single_root_boc(&shard_block.serialized_block)?;
    let block = &shard_block.block;
    let info = block.info.read_struct()?;
    let root_hash = root.repr_hash().clone();

    let file_hash = shard_block.file_hash.clone();
    let id = BlockIdExt::with_params(info.shard().clone(), info.seq_no(), root_hash, file_hash);

    let parsed = parser.parse(
        ParsingBlock {
            id: &id,
            block,
            root: &root,
            data: &shard_block.serialized_block,
            mc_seq_no: Some(mc_seq_no),
            proof: None,
            shard_state: Some(&shard_block.shard_state),
        },
        false,
    )?;
    fn serialize_entry(entry: ParsedEntry) -> SerializedItem {
        SerializedItem {
            id: entry.id,
            data: Value::Object(entry.body),
        }
    }

    if let Some(block) = parsed.block {
        db.put_block(serialize_entry(block))?;
    }
    for acc in parsed.accounts {
        db.put_account(serialize_entry(acc))?;
    }
    for mut tr in parsed.transactions {
        let hash = UInt256::from_str(&tr.id)?;
        if let Some(trace) = shard_block.transaction_traces.remove(&hash) {
            tr.body
                .insert("trace".to_owned(), serde_json::to_value(trace)?);
        }
        db.put_transaction(serialize_entry(tr))?;
    }
    for msg in parsed.messages {
        db.put_message(serialize_entry(msg))?;
    }

    log::trace!(
        "TIME: prepare & build jsons {}ms;   {}",
        now.elapsed().as_millis(),
        shard_block.root_hash
    );
    Ok(())
}
