use super::*;
use iron::prelude::*;
use iron::status;
use router::Router;
use std::io::Cursor;
use std::io::Read;
use std::sync::{Arc, Mutex};
use ton_block::{Deserializable, Message};
use ton_node::error::NodeResult;
use ton_node::node_engine::messages::{InMessagesQueue, QueuedMessage};
use ton_node::node_engine::MessagesReceiver;
use ton_types::types::UInt256;
use ton_types::SliceData;

pub struct MessageReceiverApi {
    path: String,
    router: Arc<Mutex<Option<Router>>>,
}

impl MessagesReceiver for MessageReceiverApi {
    fn run(&mut self, queue: Arc<InMessagesQueue>) -> NodeResult<()> {
        let path = format!("/{}", self.path);
        if let Some(ref mut router) = *self.router.lock().unwrap() {
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

                            let mut message = QueuedMessage::with_message(message).unwrap();
                            loop {
                                if let Err(msg) = queue.queue(message) {
                                    if queue.has_delivery_problems() {
                                        log::warn!(target: "node", "Request was refused because downstream services are not accessible");
                                        return Ok(Response::with(status::ServiceUnavailable));
                                    }
                                    log::warn!(target: "node", "Error queue message");
                                    message = msg;
                                    thread::sleep(Duration::from_micros(100));
                                } else {
                                    break;
                                }
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

        let id = match base64::decode(id_b64) {
            Ok(bytes) => bytes,
            Err(error) => {
                return Err(format!(
                    "Error decoding base64-encoded message's id: {}",
                    error
                ))
            }
        };
        if id.len() != 32 {
            return Err("Error decoding base64-encoded message's id: invalid length".to_string());
        }

        let message_cell = match ton_types::cells_serialization::deserialize_tree_of_cells(
            &mut Cursor::new(message_bytes),
        ) {
            Err(err) => {
                log::error!(target: "node", "Error deserializing message: {}", err);
                return Err(format!("Error deserializing message: {}", err));
            }
            Ok(cell) => cell,
        };
        if message_cell.repr_hash() != UInt256::from_slice(&id) {
            return Err(format!(
                "Error: calculated message's hash doesn't correspond given key"
            ));
        }

        let mut message = Message::default();
        if let Err(error) = message.read_from(&mut SliceData::from(message_cell)) {
            return Err(format!("Error parsing message's cells tree: {}", error));
        }

        Ok(message)
    }
}
