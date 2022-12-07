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

use super::SerializedItem;
use crate::data::DocumentsDb;
use crate::error::{NodeError, NodeResult};
use parking_lot::Mutex;
use serde_derive::Deserialize;
use std::cmp::min;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, SendError, Sender};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

const FIRST_TIMEOUT: u64 = 1000;
const MAX_TIMEOUT: u64 = 20000;
const TIMEOUT_BACKOFF_MULTIPLIER: f64 = 1.2;

pub enum ArangoOverwriteMode {
    #[allow(dead_code)]
    Ignore,
    Replace,
    Update,
}

impl ArangoOverwriteMode {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Ignore => "ignore",
            Self::Replace => "replace",
            Self::Update => "update",
        }
    }
}

enum ArangoRecord {
    Block(SerializedItem),
    Transaction(SerializedItem),
    Account(SerializedItem),
    Message(SerializedItem),
}

#[derive(Clone, Deserialize, Default)]
struct ArangoHelperConfig {
    pub server: String,
    pub database: String,
    pub blocks_collection: String,
    pub messages_collection: String,
    pub transactions_collection: String,
    pub accounts_collection: String,
}

struct ArangoHelperContext {
    pub config: ArangoHelperConfig,
    pub has_delivery_problems: Arc<AtomicBool>,
    pub client: reqwest::Client,
}

pub struct ArangoHelper {
    sender: Arc<Mutex<Sender<ArangoRecord>>>,
    has_delivery_problems: Arc<AtomicBool>,
}

impl ArangoHelper {
    pub fn from_config(config: &str) -> NodeResult<Self> {
        let config: ArangoHelperConfig = serde_json::from_str(config).map_err(|e| {
            NodeError::InvalidData(format!("can't deserialize ArangoHelperConfig: {}", e))
        })?;

        let (sender, receiver) = channel::<ArangoRecord>();
        let has_delivery_problems = Arc::new(AtomicBool::new(false));
        let context = ArangoHelperContext {
            config,
            has_delivery_problems: has_delivery_problems.clone(),
            client: reqwest::Client::new(),
        };

        thread::spawn(move || {
            Self::put_records_worker(receiver, context);
        });

        Ok(ArangoHelper {
            sender: Arc::new(Mutex::new(sender)),
            has_delivery_problems,
        })
    }

    fn replace_key(item: SerializedItem) -> NodeResult<String> {
        match item.data {
            serde_json::Value::Object(mut map) => {
                map.remove("id");
                map.insert("_key".to_owned(), item.id.into());
                Ok(format!("{:#}", serde_json::json!(map)))
            }
            _ => Err(NodeError::InvalidData(
                "serialized item is not an object".to_owned(),
            )),
        }
    }

    fn process_item(
        context: &ArangoHelperContext,
        collection: &str,
        item: SerializedItem,
        overwrite_mode: ArangoOverwriteMode,
    ) -> NodeResult<()> {
        let data = Self::replace_key(item)?;
        Self::post_to_arango(
            &context.config.server,
            &context.config.database,
            collection,
            &context.has_delivery_problems,
            data,
            &context.client,
            overwrite_mode,
        );
        Ok(())
    }

    fn put_records_worker(receiver: Receiver<ArangoRecord>, context: ArangoHelperContext) {
        for record in receiver {
            let result = match record {
                ArangoRecord::Block(item) => Self::process_item(
                    &context,
                    &context.config.blocks_collection,
                    item,
                    ArangoOverwriteMode::Replace,
                ),
                ArangoRecord::Transaction(item) => Self::process_item(
                    &context,
                    &context.config.transactions_collection,
                    item,
                    ArangoOverwriteMode::Replace,
                ),
                ArangoRecord::Account(item) => Self::process_item(
                    &context,
                    &context.config.accounts_collection,
                    item,
                    ArangoOverwriteMode::Replace,
                ),
                ArangoRecord::Message(item) => Self::process_item(
                    &context,
                    &context.config.messages_collection,
                    item,
                    ArangoOverwriteMode::Update,
                ),
            };

            if result.is_err() {
                log::error!(target: "node", "Error put record into arango");
            }
        }
    }

    fn post_to_arango(
        server: &str,
        db_name: &str,
        collection: &str,
        has_delivery_problems: &AtomicBool,
        doc: String,
        client: &reqwest::Client,
        overwrite_mode: ArangoOverwriteMode,
    ) {
        let mut timeout = FIRST_TIMEOUT;
        loop {
            let url = format!(
                "http://{}/_db/{}/_api/document/{}",
                server, db_name, collection
            );
            let res = client
                .post(&url)
                .query(&[
                    ("overwriteMode", overwrite_mode.as_str()),
                    ("waitForSync", "true"),
                ])
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
    fn put_block(&self, item: SerializedItem) -> NodeResult<()> {
        if let Err(SendError(ArangoRecord::Block(item))) =
            self.sender.lock().send(ArangoRecord::Block(item))
        {
            log::error!(target: "node", "Error sending block {}:", item.id);
        };

        Ok(())
    }

    fn put_message(&self, item: SerializedItem) -> NodeResult<()> {
        if let Err(SendError(ArangoRecord::Message(item))) =
            self.sender.lock().send(ArangoRecord::Message(item))
        {
            log::error!(target: "node", "Error sending message {}:", item.id);
        };

        Ok(())
    }

    fn put_transaction(&self, item: SerializedItem) -> NodeResult<()> {
        if let Err(SendError(ArangoRecord::Transaction(item))) =
            self.sender.lock().send(ArangoRecord::Transaction(item))
        {
            log::error!(target: "node", "Error sending transaction {}:", item.id);
        };

        Ok(())
    }

    fn put_account(&self, item: SerializedItem) -> NodeResult<()> {
        if let Err(SendError(ArangoRecord::Account(item))) =
            self.sender.lock().send(ArangoRecord::Account(item))
        {
            log::error!(target: "node", "Error sending account {}:", item.id);
        };

        Ok(())
    }

    fn has_delivery_problems(&self) -> bool {
        self.has_delivery_problems.load(Ordering::SeqCst)
    }
}
