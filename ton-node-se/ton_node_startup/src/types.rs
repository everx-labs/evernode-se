use std::cmp::min;
use super::*;
use parking_lot::{ Mutex };
use std::sync::Arc;
use std::sync::mpsc::{ channel, SendError, Sender, Receiver };
use std::thread;
use ton_block::{
    Account, Block, BlockId, BlockProcessingStatus, Deserializable, Serializable,
    Message, Transaction, TransactionId,
    MessageProcessingStatus, TransactionProcessingStatus
};
use ton_block_json::{
    AccountSerializationSet, BlockSerializationSet, DeletedAccountSerializationSet,
    MessageSerializationSet, TransactionSerializationSet
};
use ton_block_json::{
    db_serialize_account, db_serialize_block, db_serialize_deleted_account,
    db_serialize_transaction, db_serialize_message
};
use ton_node::node_engine::{DocumentsDb, MessagesReceiver};
use ton_node::error::{NodeResult, NodeErrorKind, NodeError};
use ton_node::node_engine::messages::{InMessagesQueue, QueuedMessage};
use serde::Deserialize;
use ton_types::{AccountId, SliceData};
use ton_types::cells_serialization::serialize_toc;
use ton_types::types::UInt256;
use std::io::Cursor;
use iron::prelude::*;
use router::Router;
use iron::status;
use std::io::Read;
use std::time::Duration;
use std::sync::atomic::{AtomicBool, Ordering};

const FIRST_TIMEOUT: u64 = 1000;
const TIMEOUT_BACKOFF_MULTIPLIER: f64 = 1.2;
const MAX_TIMEOUT: u64 = 20000;

#[derive(Deserialize, Default)]
struct KafkaProxyMsgReceiverConfig {
    pub input_topic: String,
    pub address: String,
    pub port: u16
}

// TODO: rename into KafkaProxyMsgReceiver
pub struct KafkaProxyMsgReceiver {
    config: KafkaProxyMsgReceiverConfig
}

impl MessagesReceiver for KafkaProxyMsgReceiver {
    fn run(&mut self, queue: Arc<InMessagesQueue>) -> NodeResult<()> {
        let path = format!("/topics/{}", self.config.input_topic);
        let addr = format!("{}:{}", self.config.address, self.config.port);
        
        std::thread::spawn(move || {
            let mut router = Router::new();
            router.post(
                path,
                move |req: &mut Request| Self::process_request(req, Arc::clone(&queue)),
                "");
            Iron::new(router).http(addr)
                .expect("error starting kafka proxy messages receiver");
        });

        Ok(())
    }
}

impl KafkaProxyMsgReceiver {

    pub fn from_config(config: &str) -> NodeResult<KafkaProxyMsgReceiver> {

        let config: KafkaProxyMsgReceiverConfig = serde_json::from_str(config)
                .map_err(|e| NodeError::from(NodeErrorKind::InvalidData(
                    format!("can't deserialize KafkaProxyMsgReceiverConfig: {}", e))))?;

        Ok(KafkaProxyMsgReceiver { config })
    }

    fn process_request(req: &mut Request, queue: Arc<InMessagesQueue>) -> Result<Response, IronError> {

        info!(target: "node", "Rest service: request got!");

        let mut body = String::new();
        if req.body.read_to_string(&mut body).is_ok() {
            if let Ok(body) = serde_json::from_str::<serde_json::Value>(&body) {
                if let Some(records) = body.as_object()
                    .and_then(|body| body.get("records"))
                    .and_then(|records| records.as_array()) {
                    
                    for record in records {
                        let key = record.as_object()
                            .and_then(|record| record.get("key"))
                            .and_then(|val| val.as_str());

                        let value = record.as_object()
                            .and_then(|record| record.get("value"))
                            .and_then(|val| val.as_str());

                        if let (Some(key), Some(value)) = (key, value) {

                            let message = match Self::parse_message(key, value) {
                                Ok(m) => m,
                                Err(err) => {
                                    warn!(target: "node", "Error parsing message: {}", err);
                                    return Ok(Response::with((status::BadRequest, format!("Error parsing message: {}", err))));
                                }
                            };

                            let mut message = QueuedMessage::with_message(message).unwrap();
                            loop {
                                if let Err(msg) = queue.queue(message) {
                                    if queue.has_delivery_problems() {
                                        warn!(target: "node", "Request was refused because downstream services are not accessible");
                                        return Ok(Response::with(status::ServiceUnavailable));
                                    }
                                    warn!(target: "node", "Error queue message");
                                    message = msg;
                                    std::thread::sleep(std::time::Duration::from_micros(100));
                                } else {
                                    break;
                                }
                            }

                            return Ok(Response::with(status::Ok))
                        }
                    }
                }
            }
        }

        warn!(target: "node", "Error parsing request's body");
        Ok(Response::with((status::BadRequest, "Error parsing request's body")))
    }

    fn parse_message(id_b64: &str, message_b64: &str) -> Result<Message, String> {

        let message_bytes = match base64::decode(message_b64) {
            Ok(bytes) => bytes,
            Err(error) => return Err(format!("Error decoding base64-encoded message: {}", error))
        };

        let id = match base64::decode(id_b64) {
            Ok(bytes) => bytes,
            Err(error) => return Err(format!("Error decoding base64-encoded message's id: {}", error))
        };
        if id.len() != 32 {
            return Err("Error decoding base64-encoded message's id: invalid length".to_string())
        }

        let message_cell = match ton_types::cells_serialization::deserialize_tree_of_cells(&mut Cursor::new(message_bytes)) {
            Err(err) => {
                log::error!(target: "node", "Error deserializing message: {}", err);
                return Err(format!("Error deserializing message: {}", err))
            }
            Ok(cell) => cell,
        };
        if message_cell.repr_hash() != UInt256::from(id) {
            return Err(format!("Error: calculated message's hash doesn't correnpond given key"))
        }

        let mut message = Message::default();
        if let Err(error) = message.read_from(&mut SliceData::from(message_cell)) {
            return Err(format!("Error parcing message's cells tree: {}", error))
        }

        Ok(message)
    }
}

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
            .map_err(|e| NodeError::from(NodeErrorKind::InvalidData(
                format!("can't deserialize ArangoHelperConfig: {}", e))))?;

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
            format!("{:#}", json!(doc)));
        Ok(())
    }

    fn process_put_message(context: &ArangoHelperContext, set: MessageSerializationSet) -> NodeResult<()> {
        let doc = db_serialize_message("_key", &set)?;
        Self::post_to_arango(
            &context.config.server,
            &context.config.database,
            &context.config.messages_collection,
            &context.has_delivery_problems,
            format!("{:#}", json!(doc)));
        Ok(())
    }

    fn process_put_transaction(context: &ArangoHelperContext, set: TransactionSerializationSet) -> NodeResult<()> {
        let doc = db_serialize_transaction("_key", &set)?;
        Self::post_to_arango(
            &context.config.server,
            &context.config.database,
            &context.config.transactions_collection,
            &context.has_delivery_problems,
            format!("{:#}", json!(doc)));
        Ok(())
    }

    fn process_put_account(context: &ArangoHelperContext, set: AccountSerializationSet) -> NodeResult<()> {
        let doc = db_serialize_account("_key", &set)?;
        Self::post_to_arango(
            &context.config.server,
            &context.config.database,
            &context.config.accounts_collection,
            &context.has_delivery_problems,
            format!("{:#}", json!(doc)));
        Ok(())
    }

    fn process_put_deleted_account(context: &ArangoHelperContext, set: DeletedAccountSerializationSet) -> NodeResult<()> {
        let doc = db_serialize_deleted_account("_key", &set)?;
        Self::post_to_arango(
            &context.config.server,
            &context.config.database,
            &context.config.accounts_collection,
            &context.has_delivery_problems,
            format!("{:#}", json!(doc)));
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
                        debug!(target: "node", "sucessfully sent to arango");
                        break;
                    }
                }
                Err(err) => {
                    has_delivery_problems.store(true, Ordering::SeqCst);
                    log::error!(target: "node", "error sending to arango ({})", err);
                }
            }
            debug!(target: "node", "post_to_arango: timeout {}ms", timeout);
            thread::sleep(Duration::from_millis(timeout));
            timeout = min(MAX_TIMEOUT, timeout * TIMEOUT_BACKOFF_MULTIPLIER as u64);
        }
    }
}

impl DocumentsDb for ArangoHelper {
    fn put_block(&self, block: Block, status: BlockProcessingStatus) -> NodeResult<()> {
        let cell = block.serialize()?;
        let boc = serialize_toc(&cell)?;
        let set = BlockSerializationSet {
            block,
            id: cell.repr_hash(),
            status,
            boc
        };

        if let Err(SendError(ArangoRecord::Block(set))) = self.sender.lock().send(ArangoRecord::Block(set)) {
            error!(target: "node", "Error sending block {:x}:", set.id);
        };

        Ok(())
    }

    fn put_message(&self, message: Message, status: MessageProcessingStatus,
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
            status,
            boc,
            proof: None
        };

        if let Err(SendError(ArangoRecord::Message(set))) = self.sender.lock().send(ArangoRecord::Message(set)) {
            error!(target: "node", "Error sending message {:x}:", set.id);
        };

        Ok(())
    }

    fn put_transaction(&self, transaction: Transaction, status: TransactionProcessingStatus, 
        block_id: Option<BlockId>, workchain_id: i32) -> NodeResult<()> {

        let cell = transaction.serialize()?;
        let boc = serialize_toc(&cell)?;
        let set = TransactionSerializationSet {
            transaction,
            id: cell.repr_hash(),
            status,
            block_id,
            workchain_id,
            boc,
            proof: None
        };

        if let Err(SendError(ArangoRecord::Transaction(set))) = self.sender.lock().send(ArangoRecord::Transaction(set)) {
            error!(target: "node", "Error sending transaction {:x}:", set.id);
        };

        Ok(())
    }

    fn put_account(&self, account: Account) -> NodeResult<()> {
        let account_addr = account.get_id().unwrap_or_default();
        let cell = account.serialize()?;
        let boc = serialize_toc(&cell)?;
        let set = AccountSerializationSet {
            account,
            boc,
            proof: None
        };

        if let Err(SendError(ArangoRecord::Account(_))) = self.sender.lock().send(ArangoRecord::Account(set)) {
            error!(target: "node", "Error sending account {:x}:", account_addr);
        };

        Ok(())
    }

    fn put_deleted_account(&self, workchain_id: i32, account_id: AccountId) -> NodeResult<()> {
        let set = DeletedAccountSerializationSet {
            account_id: account_id.clone(),
            workchain_id,
        };

        if let Err(SendError(ArangoRecord::DeletedAccount(_))) = self.sender.lock().send(ArangoRecord::DeletedAccount(set)) {
            error!(target: "node", "Error sending deleted account {}:{:x}:", workchain_id, account_id);
        };

        Ok(())
    }


    fn has_delivery_problems(&self) -> bool {
        self.has_delivery_problems.load(Ordering::SeqCst)
    }
}
