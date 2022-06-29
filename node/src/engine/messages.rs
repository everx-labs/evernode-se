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

use crate::error::NodeError;
use jsonrpc_http_server::jsonrpc_core::types::params::Params;
use jsonrpc_http_server::jsonrpc_core::types::Value;
use jsonrpc_http_server::jsonrpc_core::{Error, IoHandler};
use jsonrpc_http_server::{AccessControlAllowOrigin, DomainsValidation, Server, ServerBuilder};
use parking_lot::Mutex;
use std::sync::Arc;
use ton_block::*;
use ton_types::{
    serialize_toc, BuilderData, Result, SliceData,
};
use crate::data::DocumentsDb;

#[cfg(test)]
#[path = "../../../../tonos-se-tests/unit/test_messages.rs"]
mod tests;

/// Json rpc server for receiving external outbound messages.
/// TODO the struct is not used now (15.08.19). It is candidate to deletion.
pub struct JsonRpcMsgReceiver {
    host: String,
    port: String,
    server: Option<Server>,
}

#[allow(dead_code)]
impl MessagesReceiver for JsonRpcMsgReceiver {
    /// Start to receive messages. The function runs the receive thread and returns control.
    fn run(&mut self, queue: Arc<InMessagesQueue>) -> NodeResult<()> {
        if self.server.is_some() {
            Err(NodeError::InvalidOperation)
        } else {
            let mut io = IoHandler::default();
            io.add_method("call", move |params| {
                Self::process_call(params, Arc::clone(&queue))
            });

            self.server = Some(
                ServerBuilder::new(io)
                    .cors(DomainsValidation::AllowOnly(vec![
                        AccessControlAllowOrigin::Null,
                    ]))
                    .start_http(&format!("{}:{}", self.host, self.port).parse().unwrap())?,
            );

            Ok(())
        }
    }
}

#[allow(dead_code)]
impl JsonRpcMsgReceiver {
    /// Create a new instance of the struct witch put received messages into given queue
    pub fn with_params(host: &str, port: &str) -> Self {
        Self {
            host: String::from(host),
            port: String::from(port),
            server: None,
        }
    }

    /// Stop receiving. Sends message to the receive thread and waits while it stops.
    pub fn stop(&mut self) -> NodeResult<()> {
        if self.server.is_some() {
            let s = std::mem::replace(&mut self.server, None);
            s.unwrap().close();
            Ok(())
        } else {
            Err(NodeError::InvalidOperation)
        }
    }

    fn process_call(
        params: Params,
        msg_queue: Arc<InMessagesQueue>,
    ) -> jsonrpc_http_server::jsonrpc_core::Result<Value> {
        const MESSAGE: &str = "message";

        let map = match params {
            Params::Map(map) => map,
            _ => return Err(Error::invalid_params("Unresolved parameters object.")),
        };

        let message = match map.get(MESSAGE) {
            Some(Value::String(string)) => string,
            Some(_) => {
                return Err(Error::invalid_params(format!(
                    "\"{}\" parameter must be a string.",
                    MESSAGE
                )))
            }
            _ => {
                return Err(Error::invalid_params(format!(
                    "\"{}\" parameter not found.",
                    MESSAGE
                )))
            }
        };

        let message = Message::construct_from_base64(&message)
            .map_err(|err| Error::invalid_params(format!("Error parsing message: {}", err)))?;

        msg_queue
            .queue(QueuedMessage::with_message(message).unwrap())
            .expect("Error queue message");

        Ok(Value::String(String::from(
            "The message has been successfully received",
        )))
    }
}


#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct QueuedMessage {
    message: Message,
    hash: UInt256,
}

impl QueuedMessage {
    pub fn with_message(message: Message) -> Result<Self> {
        let hash = message.hash()?;
        Ok(Self {
            message,
            hash
        })
    }

    pub fn message(&self) -> &Message {
        &self.message
    }

    pub fn message_hash(&self) -> &UInt256 {
        &self.hash
    }

}

impl Serializable for QueuedMessage {
    fn write_to(&self, cell: &mut BuilderData) -> Result<()> {
        self.message.write_to(cell)
    }
}

impl Deserializable for QueuedMessage {
    fn construct_from(slice: &mut SliceData) -> Result<Self> {
        let message = Message::construct_from(slice)?;
        Self::with_message(message)
    }
}

/// This FIFO accumulates inbound messages from all types of receivers.
/// The struct might be used from many threads. It provides internal mutability.
pub struct InMessagesQueue {
    shard_id: ShardIdent,
    storage: Mutex<VecDeque<QueuedMessage>>,
    out_storage: Mutex<VecDeque<QueuedMessage>>,
    db: Option<Arc<dyn DocumentsDb>>,
    capacity: usize,
}

#[allow(dead_code)]
impl InMessagesQueue {
    /// Create new instance of InMessagesQueue.
    pub fn new(shard_id: ShardIdent, capacity: usize) -> Self {
        InMessagesQueue {
            shard_id,
            storage: Mutex::new(Default::default()),
            out_storage: Mutex::new(Default::default()),
            db: None,
            capacity,
        }
    }

    pub fn with_db(shard_id: ShardIdent, capacity: usize, db: Arc<dyn DocumentsDb>) -> Self {
        InMessagesQueue {
            shard_id,
            storage: Mutex::new(Default::default()),
            out_storage: Mutex::new(Default::default()),
            db: Some(db),
            capacity,
        }
    }

    pub fn has_delivery_problems(&self) -> bool {
        self.db
            .as_ref()
            .map_or(false, |db| db.has_delivery_problems())
    }

    /// Include message into end queue.
    pub fn queue(&self, msg: QueuedMessage) -> std::result::Result<(), QueuedMessage> {
        if self.has_delivery_problems() {
            log::debug!(target: "node", "Has delivery problems");
            return Err(msg);
        }

        let mut storage = self.storage.lock();
        if storage.len() >= self.capacity {
            return Err(msg);
        }

        log::debug!(target: "node", "Queued message: {:?}", msg.message());
        storage.push_back(msg);

        Ok(())
    }

    // /// Include message into begin queue
    // fn priority_queue(&self, msg: QueuedMessage) -> std::result::Result<(), QueuedMessage> {
    //     log::debug!(target: "node", "Priority queued message: {:?}", msg.message());
    //     self.storage.lock().push_back(msg);
    //     Ok(())
    // }

    /// Extract oldest message from queue.
    pub fn dequeue(&self) -> Option<QueuedMessage> {
        self.storage.lock().pop_front()
    }

    // /// Extract oldest message from queue if message account not using in executor
    // pub fn dequeue_first_unused(&self) -> Option<QueuedMessage> {
    //     self.storage.lock().pop_front()
    // }

    pub fn print_message(msg: &Message) {
        log::info!("message: {:?}", msg);
        if let Ok(cell) = msg.serialize() {
            if let Ok(data) = serialize_toc(&cell) {
                std::fs::create_dir_all("export").ok();
                std::fs::write(&format!("export/msg_{:x}", cell.repr_hash()), &data).ok();
            }
        }
    }

    pub fn is_full(&self) -> bool {
        self.len() >= self.capacity
    }

    /// The length of queue.
    pub fn len(&self) -> usize {
        self.storage.lock().len()
    }
}

