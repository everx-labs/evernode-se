use crate::block::ShardBlock;
use crate::data::{DocumentsDb, SerializedItem};
use crate::error::NodeError;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;
use ton_block::{
    Account, AccountBlock, AccountStatus, Block, BlockProcessingStatus, Deserializable,
    HashmapAugType, Message, MessageProcessingStatus, MsgAddrStd, MsgAddressInt, Serializable,
    Transaction, TransactionProcessingStatus,
};
use ton_types::{
    AccountId, BuilderData, Cell, HashmapType, Result, SliceData, UInt256, write_boc,
};

use super::builder::EngineTraceInfoData;

lazy_static::lazy_static!(
    static ref ACCOUNT_NONE_HASH: UInt256 = Account::default().serialize().unwrap().repr_hash();
    pub static ref MINTER_ADDRESS: MsgAddressInt =
        MsgAddressInt::AddrStd(MsgAddrStd::with_address(None, 0, [0; 32].into()));
);

pub fn reflect_block_in_db(db: Arc<dyn DocumentsDb>, shard_block: &mut ShardBlock) -> Result<()> {
    let add_proof = false;
    let block_id = &shard_block.root_hash;
    log::trace!("Processor block_stuff.id {}", block_id.to_hex_string());
    let file_hash = shard_block.file_hash.clone();
    let block = &shard_block.block;
    let block_root = shard_block.block.serialize()?;
    let block_extra = block.read_extra()?;
    let block_boc = &shard_block.serialized_block;
    let info = block.read_info()?;
    let block_order = ton_block_json::block_order(&block, info.seq_no())?;
    let workchain_id = info.shard().workchain_id();
    let shard_accounts = shard_block.shard_state.read_accounts()?;

    let now_all = std::time::Instant::now();

    // Prepare sorted ton_block transactions and addresses of changed accounts
    let mut changed_acc = HashSet::new();
    let mut deleted_acc = HashSet::new();
    let mut acc_last_trans_chain_order = HashMap::new();
    let now = std::time::Instant::now();
    let mut tr_count = 0;
    let mut transactions = BTreeMap::new();
    block_extra
        .read_account_blocks()?
        .iterate_objects(|account_block: AccountBlock| {
            // extract ids of changed accounts
            let state_upd = account_block.read_state_update()?;
            let mut check_account_existed = false;
            if state_upd.new_hash == *ACCOUNT_NONE_HASH {
                deleted_acc.insert(account_block.account_id().clone());
                if state_upd.old_hash == *ACCOUNT_NONE_HASH {
                    check_account_existed = true;
                }
            } else {
                changed_acc.insert(account_block.account_id().clone());
            }

            let mut account_existed = false;
            account_block
                .transactions()
                .iterate_slices(|_, transaction_slice| {
                    // extract transactions
                    let cell = transaction_slice.reference(0)?;
                    let transaction =
                        Transaction::construct_from(&mut SliceData::load_cell(cell.clone())?)?;
                    let ordering_key =
                        (transaction.logical_time(), transaction.account_id().clone());
                    if transaction.orig_status != AccountStatus::AccStateNonexist
                        || transaction.end_status != AccountStatus::AccStateNonexist
                    {
                        account_existed = true;
                    }
                    transactions.insert(ordering_key, (cell, transaction));
                    tr_count += 1;

                    Ok(true)
                })?;

            if check_account_existed && !account_existed {
                deleted_acc.remove(account_block.account_id());
            }

            Ok(true)
        })?;
    log::trace!(
        "TIME: preliminary prepare {} transactions {}ms;   {}",
        tr_count,
        now.elapsed().as_millis(),
        block_id
    );

    // Iterate ton_block transactions to:
    // - prepare messages and transactions for external db
    // - prepare last_trans_chain_order for accounts
    let now = std::time::Instant::now();
    let mut index = 0;
    let mut messages = Default::default();
    for (_, (cell, transaction)) in transactions.into_iter() {
        let tr_chain_order = format!(
            "{}{}",
            block_order,
            ton_block_json::u64_to_string(index as u64)
        );

        prepare_messages_from_transaction(
            &transaction,
            block_root.repr_hash(),
            &tr_chain_order,
            add_proof.then(|| &block_root),
            &mut messages,
        )?;

        let account_id = transaction.account_id().clone();
        acc_last_trans_chain_order.insert(account_id, tr_chain_order.clone());
        let trace = shard_block.transaction_traces.remove(&cell.repr_hash());

        let mut doc = prepare_transaction_json(
            cell,
            transaction,
            &block_root,
            block_root.repr_hash(),
            workchain_id,
            add_proof,
            trace,
        )?;
        doc.insert("chain_order".to_string(), tr_chain_order.into());
        db.put_transaction(doc_to_item(doc))?;
        index += 1;
    }
    let msg_count = messages.len(); // is 0 if not process_message
    for (_, message) in messages {
        db.put_message(doc_to_item(message))?;
    }
    log::trace!(
        "TIME: prepare {} transactions and {} messages {}ms;   {}",
        tr_count,
        msg_count,
        now.elapsed().as_millis(),
        block_id,
    );

    // Prepare accounts (changed and deleted)
    let now = std::time::Instant::now();
    for account_id in changed_acc.iter() {
        let acc = shard_accounts.account(account_id)?.ok_or_else(|| {
            NodeError::InvalidData(
                "Block and shard state mismatch: \
                        state doesn't contain changed account"
                    .to_string(),
            )
        })?;
        let acc = acc.read_account()?;

        db.put_account(prepare_account_record(
            acc,
            None,
            acc_last_trans_chain_order.remove(account_id),
        )?)?;
    }

    for account_id in deleted_acc {
        let last_trans_chain_order = acc_last_trans_chain_order.remove(&account_id);
        db.put_account(prepare_deleted_account_record(
            account_id,
            workchain_id,
            None,
            last_trans_chain_order,
        )?)?;
    }
    log::trace!(
        "TIME: accounts {} {}ms;   {}",
        changed_acc.len(),
        now.elapsed().as_millis(),
        block_id
    );

    // Block
    let now = std::time::Instant::now();
    db.put_block(prepare_block_record(
        block,
        &block_root,
        block_boc,
        &file_hash,
        block_order.clone(),
    )?)?;
    log::trace!(
        "TIME: block {}ms;   {}",
        now.elapsed().as_millis(),
        block_id
    );
    log::trace!(
        "TIME: prepare & build jsons {}ms;   {}",
        now_all.elapsed().as_millis(),
        block_id
    );
    Ok(())
}

pub(crate) fn prepare_messages_from_transaction(
    transaction: &Transaction,
    block_id: UInt256,
    tr_chain_order: &str,
    block_root_for_proof: Option<&Cell>,
    messages: &mut HashMap<UInt256, serde_json::value::Map<String, serde_json::Value>>,
) -> Result<()> {
    if let Some(message_cell) = transaction.in_msg_cell() {
        let message = Message::construct_from_cell(message_cell.clone())?;
        let message_id = message_cell.repr_hash();
        let mut doc =
            if message.is_inbound_external() || message.src_ref() == Some(&*MINTER_ADDRESS) {
                prepare_message_json(
                    message_cell,
                    message,
                    block_root_for_proof,
                    block_id.clone(),
                    Some(transaction.now()),
                )?
            } else {
                messages.remove(&message_id).unwrap_or_else(|| {
                    let mut doc = serde_json::value::Map::with_capacity(2);
                    doc.insert("id".to_string(), message_id.as_hex_string().into());
                    doc
                })
            };

        doc.insert(
            "dst_chain_order".to_string(),
            format!("{}{}", tr_chain_order, ton_block_json::u64_to_string(0)).into(),
        );

        messages.insert(message_id, doc);
    };

    let mut index: u64 = 1;
    transaction.out_msgs.iterate_slices(|slice| {
        let message_cell = slice.reference(0)?;
        let message_id = message_cell.repr_hash();
        let message = Message::construct_from_cell(message_cell.clone())?;
        let mut doc = prepare_message_json(
            message_cell,
            message,
            block_root_for_proof,
            block_id.clone(),
            None, // transaction_now affects ExtIn messages only
        )?;

        // messages are ordered by created_lt
        doc.insert(
            "src_chain_order".to_string(),
            format!("{}{}", tr_chain_order, ton_block_json::u64_to_string(index)).into(),
        );

        index += 1;
        messages.insert(message_id, doc);
        Ok(true)
    })?;

    Ok(())
}

pub(crate) fn prepare_message_json(
    message_cell: Cell,
    message: Message,
    block_root_for_proof: Option<&Cell>,
    block_id: UInt256,
    transaction_now: Option<u32>,
) -> Result<serde_json::value::Map<String, serde_json::Value>> {
    let boc = write_boc(&message_cell)?;
    let proof = block_root_for_proof
        .map(|cell| write_boc(&message.prepare_proof(true, cell)?))
        .transpose()?;

    let set = ton_block_json::MessageSerializationSet {
        message,
        id: message_cell.repr_hash(),
        block_id: Some(block_id.clone()),
        transaction_id: None, // it would be ambiguous for internal or replayed messages
        status: MessageProcessingStatus::Finalized,
        boc,
        proof,
        transaction_now, // affects ExtIn messages only
    };
    let doc = ton_block_json::db_serialize_message("id", &set)?;
    Ok(doc)
}

pub(crate) fn doc_to_item(
    doc: serde_json::value::Map<String, serde_json::Value>,
) -> SerializedItem {
    SerializedItem {
        id: doc["id"].as_str().unwrap().to_owned(),
        data: serde_json::json!(doc),
    }
}

pub(crate) fn prepare_transaction_json(
    tr_cell: Cell,
    transaction: Transaction,
    block_root: &Cell,
    block_id: UInt256,
    workchain_id: i32,
    add_proof: bool,
    trace: Option<Vec<EngineTraceInfoData>>,
) -> Result<serde_json::value::Map<String, serde_json::Value>> {
    let boc = write_boc(&tr_cell)?;
    let proof = if add_proof {
        Some(write_boc(&transaction.prepare_proof(&block_root)?)?)
    } else {
        None
    };
    let set = ton_block_json::TransactionSerializationSet {
        transaction,
        id: tr_cell.repr_hash(),
        status: TransactionProcessingStatus::Finalized,
        block_id: Some(block_id.clone()),
        workchain_id,
        boc,
        proof,
        ..Default::default()
    };
    let mut doc = ton_block_json::db_serialize_transaction("id", &set)?;
    if let Some(trace) = trace {
        doc.insert("trace".to_owned(), serde_json::to_value(trace)?);
    }
    Ok(doc)
}

pub(crate) fn prepare_account_record(
    account: Account,
    prev_account_state: Option<Account>,
    last_trans_chain_order: Option<String>,
) -> Result<SerializedItem> {
    let mut boc1 = None;
    if account.init_code_hash().is_some() {
        // new format
        let mut builder = BuilderData::new();
        account.write_original_format(&mut builder)?;
        boc1 = Some(write_boc(&builder.into_cell()?)?);
    }
    let boc = account.write_to_bytes()?;

    let set = ton_block_json::AccountSerializationSet {
        account,
        prev_account_state,
        proof: None,
        boc,
        boc1,
        ..Default::default()
    };

    let mut doc = ton_block_json::db_serialize_account("id", &set)?;
    if let Some(last_trans_chain_order) = last_trans_chain_order {
        doc.insert(
            "last_trans_chain_order".to_owned(),
            last_trans_chain_order.into(),
        );
    }
    Ok(doc_to_item(doc))
}

pub(crate) fn prepare_deleted_account_record(
    account_id: AccountId,
    workchain_id: i32,
    prev_account_state: Option<Account>,
    last_trans_chain_order: Option<String>,
) -> Result<SerializedItem> {
    let set = ton_block_json::DeletedAccountSerializationSet {
        account_id,
        workchain_id,
        prev_account_state,
        ..Default::default()
    };

    let mut doc = ton_block_json::db_serialize_deleted_account("id", &set)?;
    if let Some(last_trans_chain_order) = last_trans_chain_order {
        doc.insert(
            "last_trans_chain_order".to_owned(),
            last_trans_chain_order.into(),
        );
    }
    Ok(doc_to_item(doc))
}

pub(crate) fn prepare_block_record(
    block: &Block,
    block_root: &Cell,
    boc: &[u8],
    file_hash: &UInt256,
    block_order: String,
) -> Result<SerializedItem> {
    let set = ton_block_json::BlockSerializationSetFH {
        block,
        id: &block_root.repr_hash(),
        status: BlockProcessingStatus::Finalized,
        boc,
        file_hash: Some(file_hash),
    };
    let mut doc = ton_block_json::db_serialize_block("id", set)?;
    doc.insert("chain_order".to_owned(), block_order.into());
    Ok(doc_to_item(doc))
}
