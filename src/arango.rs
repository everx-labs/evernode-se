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

use ton_block::{Account, Block, BlockId, BlockProcessingStatus, Message, MessageProcessingStatus, Serializable, Transaction, TransactionId, TransactionProcessingStatus};
use ton_types::{AccountId, BuilderData, serialize_toc};
use ton_block_json::{AccountSerializationSet, BlockSerializationSet, db_serialize_account, db_serialize_block, db_serialize_deleted_account, db_serialize_message, db_serialize_transaction, DeletedAccountSerializationSet, MessageSerializationSet, TransactionSerializationSet};
use std::sync::mpsc::{channel, Receiver, Sender, SendError};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use parking_lot::Mutex;
use std::time::Duration;
use std::cmp::min;
use serde_derive::Deserialize;
use crate::error::{NodeError, NodeResult};
use crate::node_engine::DocumentsDb;

const FIRST_TIMEOUT: u64 = 1000;
const MAX_TIMEOUT: u64 = 20000;
const TIMEOUT_BACKOFF_MULTIPLIER: f64 = 1.2;

enum ArangoRecord {
    Block(BlockSerializationSet),
    Transaction(TransactionSerializationSet),
    Account(AccountSerializationSet),
    DeletedAccount(DeletedAccountSerializationSet),
    Message(MessageSerializationSet),
}

#[derive(Clone, Deserialize, Default)]
struct ArangoHelperConfig {
    pub server: String,
    pub database: String,
    pub blocks_collection: String,
    pub messages_collection: String,
    pub transactions_collection: String,
    pub accounts_collection: String
}

struct ArangoHelperContext {
    pub config: ArangoHelperConfig,
    pub has_delivery_problems: Arc<AtomicBool>,
}

pub struct ArangoHelper {
    sender: Arc<Mutex<Sender<ArangoRecord>>>,
    has_delivery_problems: Arc<AtomicBool>,
}

impl ArangoHelper {

    pub fn from_config(config: &str) -> NodeResult<Self> {

        let config: ArangoHelperConfig = serde_json::from_str(config)
            .map_err(|e| NodeError::InvalidData(
                format!("can't deserialize ArangoHelperConfig: {}", e)))?;

        let (sender, receiver) = channel::<ArangoRecord>();
        let has_delivery_problems = Arc::new(AtomicBool::new(false));
        let context = ArangoHelperContext {
            config,
            has_delivery_problems: has_delivery_problems.clone()
        };

        thread::spawn(move || {
            Self::put_records_worker(receiver, context);
        });

        Ok(ArangoHelper {
            sender: Arc::new(Mutex::new(sender)),
            has_delivery_problems
        })
    }

    fn process_put_block(context: &ArangoHelperContext, set: BlockSerializationSet) -> NodeResult<()> {
        let doc = db_serialize_block("_key", &set)?;
        Self::post_to_arango(
            &context.config.server,
            &context.config.database,
            &context.config.blocks_collection,
            &context.has_delivery_problems,
            format!("{:#}", serde_json::json!(doc)));
        Ok(())
    }

    fn process_put_message(context: &ArangoHelperContext, set: MessageSerializationSet) -> NodeResult<()> {
        let doc = db_serialize_message("_key", &set)?;
        Self::post_to_arango(
            &context.config.server,
            &context.config.database,
            &context.config.messages_collection,
            &context.has_delivery_problems,
            format!("{:#}", serde_json::json!(doc)));
        Ok(())
    }

    fn process_put_transaction(context: &ArangoHelperContext, set: TransactionSerializationSet) -> NodeResult<()> {
        let doc = db_serialize_transaction("_key", &set)?;
        Self::post_to_arango(
            &context.config.server,
            &context.config.database,
            &context.config.transactions_collection,
            &context.has_delivery_problems,
            format!("{:#}", serde_json::json!(doc)));
        Ok(())
    }

    fn process_put_account(context: &ArangoHelperContext, set: AccountSerializationSet) -> NodeResult<()> {
        let doc = db_serialize_account("_key", &set)?;
        Self::post_to_arango(
            &context.config.server,
            &context.config.database,
            &context.config.accounts_collection,
            &context.has_delivery_problems,
            format!("{:#}", serde_json::json!(doc)));
        Ok(())
    }

    fn process_put_deleted_account(context: &ArangoHelperContext, set: DeletedAccountSerializationSet) -> NodeResult<()> {
        let doc = db_serialize_deleted_account("_key", &set)?;
        Self::post_to_arango(
            &context.config.server,
            &context.config.database,
            &context.config.accounts_collection,
            &context.has_delivery_problems,
            format!("{:#}", serde_json::json!(doc)));
        Ok(())
    }

    fn put_records_worker(receiver: Receiver<ArangoRecord>, context: ArangoHelperContext) {

        for record in receiver {
            let result = match record {
                ArangoRecord::Block(set) =>
                    Self::process_put_block(&context, set),
                ArangoRecord::Transaction(set) =>
                    Self::process_put_transaction(&context, set),
                ArangoRecord::Account(set) =>
                    Self::process_put_account(&context, set),
                ArangoRecord::DeletedAccount(set) =>
                    Self::process_put_deleted_account(&context, set),
                ArangoRecord::Message(set) =>
                    Self::process_put_message(&context, set),
            };

            if result.is_err() {
                log::error!(target: "node", "Error put record into arango");
            }
        }
    }

    fn post_to_arango(server: &str, db_name: &str, collection: &str,
        has_delivery_problems: &AtomicBool, doc: String) {

        let client = reqwest::Client::new();
        let mut timeout = FIRST_TIMEOUT;
        loop {
            let url = format!("http://{}/_db/{}/_api/document/{}", server, db_name, collection);
            let res = client.post(&url)
                .query(&[("overwrite", "true")])
                .header("accept", "application/json")
                .body(doc.clone())
                .send();
            match res {
                Ok(resp) => {
                    if resp.status().is_server_error() || resp.status().is_client_error() {
                        has_delivery_problems.store(true, Ordering::SeqCst);
                        log::error!(target: "node", "error sending to arango {}, ({:?})", url, resp.status().canonical_reason());
                    } else {
                        has_delivery_problems.store(false, Ordering::SeqCst);
                        log::debug!(target: "node", "sucessfully sent to arango");
                        break;
                    }
                }
                Err(err) => {
                    has_delivery_problems.store(true, Ordering::SeqCst);
                    log::error!(target: "node", "error sending to arango ({})", err);
                }
            }
            log::debug!(target: "node", "post_to_arango: timeout {}ms", timeout);
            thread::sleep(Duration::from_millis(timeout));
            timeout = min(MAX_TIMEOUT, timeout * TIMEOUT_BACKOFF_MULTIPLIER as u64);
        }
    }
}

impl DocumentsDb for ArangoHelper {
    fn put_block(&self, block: &Block) -> NodeResult<()> {
        let cell = block.serialize()?;
        let boc = serialize_toc(&cell)?;
        let set = BlockSerializationSet {
            block: block.clone(),
            id: cell.repr_hash(),
            status: BlockProcessingStatus::Finalized,
            boc,
            ..Default::default()
        };

        if let Err(SendError(ArangoRecord::Block(set))) = self.sender.lock().send(ArangoRecord::Block(set)) {
            log::error!(target: "node", "Error sending block {:x}:", set.id);
        };

        Ok(())
    }

    fn put_message(&self, message: Message,
        transaction_id: Option<TransactionId>, transaction_now: Option<u32>, block_id: Option<BlockId>)
        -> NodeResult<()> {

        let cell = message.serialize()?;
        let boc = serialize_toc(&cell)?;
        let set = MessageSerializationSet {
            message,
            id: cell.repr_hash(),
            block_id,
            transaction_id,
            transaction_now,
            status: MessageProcessingStatus::Finalized,
            boc,
            proof: None,
            ..Default::default()
        };

        if let Err(SendError(ArangoRecord::Message(set))) = self.sender.lock().send(ArangoRecord::Message(set)) {
            log::error!(target: "node", "Error sending message {:x}:", set.id);
        };

        Ok(())
    }

    fn put_transaction(&self, transaction: Transaction,
        block_id: Option<BlockId>, workchain_id: i32) -> NodeResult<()> {

        let cell = transaction.serialize()?;
        let boc = serialize_toc(&cell)?;
        let set = TransactionSerializationSet {
            transaction,
            id: cell.repr_hash(),
            status: TransactionProcessingStatus::Finalized,
            block_id,
            workchain_id,
            boc,
            proof: None,
            ..Default::default()
        };

        if let Err(SendError(ArangoRecord::Transaction(set))) = self.sender.lock().send(ArangoRecord::Transaction(set)) {
            log::error!(target: "node", "Error sending transaction {:x}:", set.id);
        };

        Ok(())
    }

    fn put_account(&self, account: Account) -> NodeResult<()> {
        let account_addr = account.get_id().unwrap_or_default();
        let cell = account.serialize()?;
        let boc = serialize_toc(&cell)?;
        let mut boc1 = None;
        if account.init_code_hash().is_some() {
            // new format
            let mut builder = BuilderData::new();
            account.write_original_format(&mut builder)?;
            boc1 = Some(serialize_toc(&builder.into_cell()?)?);
        }
        let set = AccountSerializationSet {
            account,
            boc,
            boc1,
            proof: None,
            ..Default::default()
        };

        if let Err(SendError(ArangoRecord::Account(_))) = self.sender.lock().send(ArangoRecord::Account(set)) {
            log::error!(target: "node", "Error sending account {:x}:", account_addr);
        };

        Ok(())
    }

    fn put_deleted_account(&self, workchain_id: i32, account_id: AccountId) -> NodeResult<()> {
        let set = DeletedAccountSerializationSet {
            account_id: account_id.clone(),
            workchain_id,
            ..Default::default()
        };

        if let Err(SendError(ArangoRecord::DeletedAccount(_))) = self.sender.lock().send(ArangoRecord::DeletedAccount(set)) {
            log::error!(target: "node", "Error sending deleted account {}:{:x}:", workchain_id, account_id);
        };

        Ok(())
    }


    fn has_delivery_problems(&self) -> bool {
        self.has_delivery_problems.load(Ordering::SeqCst)
    }
}
