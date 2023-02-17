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

use crate::engine::messages::InMessagesQueue;
use crate::engine::MessagesReceiver;
use crate::error::NodeResult;
use iron::{
    prelude::{IronError, Request, Response},
    status,
};
use parking_lot::Mutex;
use router::Router;
use serde_json::Value;
use std::str::FromStr;
use std::{
    io::{Cursor, Read},
    sync::Arc,
    thread,
    time::Duration,
};
use ton_block::{Deserializable, Message};
use ton_types::UInt256;

pub struct MessageReceiverApi {
    path: String,
    router: Arc<Mutex<Option<Router>>>,
}

impl MessagesReceiver for MessageReceiverApi {
    fn run(&mut self, queue: Arc<InMessagesQueue>) -> NodeResult<()> {
        let path = format!("/{}", self.path);
        if let Some(ref mut router) = *self.router.lock() {
            router.post(
                path,
                move |req: &mut Request| Self::process_request(req, Arc::clone(&queue)),
                "messages",
            );
        }
        Ok(())
    }
}

impl MessageReceiverApi {
    pub fn new(path: String, router: Arc<Mutex<Option<Router>>>) -> Self {
        Self { path, router }
    }

    fn process_request(
        req: &mut Request,
        queue: Arc<InMessagesQueue>,
    ) -> Result<Response, IronError> {
        log::info!(target: "node", "Rest service: request got!");

        let mut body = String::new();
        if req.body.read_to_string(&mut body).is_ok() {
            if let Ok(body) = serde_json::from_str::<Value>(&body) {
                if let Some(records) = body
                    .as_object()
                    .and_then(|body| body.get("records"))
                    .and_then(|records| records.as_array())
                {
                    for record in records {
                        let key = record
                            .as_object()
                            .and_then(|record| record.get("key"))
                            .and_then(|val| val.as_str());

                        let value = record
                            .as_object()
                            .and_then(|record| record.get("value"))
                            .and_then(|val| val.as_str());

                        if let (Some(key), Some(value)) = (key, value) {
                            let message = match Self::parse_message(key, value) {
                                Ok(m) => m,
                                Err(err) => {
                                    log::warn!(target: "node", "Error parsing message: {}", err);
                                    return Ok(Response::with((
                                        status::BadRequest,
                                        format!("Error parsing message: {}", err),
                                    )));
                                }
                            };

                            let mut message = message;
                            while let Err(msg) = queue.queue(message) {
                                if queue.has_delivery_problems() {
                                    log::warn!(target: "node", "Request was refused because downstream services are not accessible");
                                    return Ok(Response::with(status::ServiceUnavailable));
                                }
                                log::warn!(target: "node", "Error queue message");
                                message = msg;
                                thread::sleep(Duration::from_micros(100));
                            }

                            return Ok(Response::with(status::Ok));
                        }
                    }
                }
            }
        }

        log::warn!(target: "node", "Error parsing request's body");
        Ok(Response::with((
            status::BadRequest,
            "Error parsing request's body",
        )))
    }

    fn parse_message(id_b64: &str, message_b64: &str) -> Result<Message, String> {
        let message_bytes = match base64::decode(message_b64) {
            Ok(bytes) => bytes,
            Err(error) => return Err(format!("Error decoding base64-encoded message: {}", error)),
        };

        let id = match UInt256::from_str(id_b64) {
            Ok(id) => id,
            Err(error) => {
                return Err(format!(
                    "Error decoding base64-encoded message's id: {}",
                    error
                ))
            }
        };

        let message_cell = match ton_types::cells_serialization::deserialize_tree_of_cells(
            &mut Cursor::new(message_bytes),
        ) {
            Err(err) => {
                log::error!(target: "node", "Error deserializing message: {}", err);
                return Err(format!("Error deserializing message: {}", err));
            }
            Ok(cell) => cell,
        };
        if message_cell.repr_hash() != id {
            return Err(format!(
                "Error: calculated message's hash doesn't correspond given key"
            ));
        }

        match Message::construct_from_cell(message_cell) {
            Ok(message) => Ok(message),
            Err(error) => Err(format!("Error parsing message's cells tree: {}", error)),
        }
    }
}
