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

use crate::data::DocumentsDb;
use crate::error::NodeResult;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::RwLock;

use super::SerializedItem;

pub struct MemDocumentsDb {
    data: RwLock<Data>,
}

struct Data {
    accounts: HashMap<String, Value>,
    blocks: Vec<Value>,
    transactions: Vec<Value>,
    messages: Vec<Value>,
}

impl Data {
    fn new() -> Self {
        Self {
            accounts: HashMap::new(),
            blocks: Vec::new(),
            transactions: Vec::new(),
            messages: Vec::new(),
        }
    }
}

impl Default for MemDocumentsDb {
    fn default() -> Self {
        Self::new()
    }
}

impl MemDocumentsDb {
    pub fn new() -> Self {
        Self {
            data: RwLock::new(Data::new()),
        }
    }

    pub fn blocks(&self) -> Vec<Value> {
        self.data.read().unwrap().blocks.clone()
    }

    pub fn transactions(&self) -> Vec<Value> {
        self.data.read().unwrap().transactions.clone()
    }
}

impl DocumentsDb for MemDocumentsDb {
    fn put_account(&self, acc: SerializedItem) -> NodeResult<()> {
        self.data.write().unwrap().accounts.insert(acc.id, acc.data);
        Ok(())
    }

    fn put_block(&self, block: SerializedItem) -> NodeResult<()> {
        self.data.write().unwrap().blocks.push(block.data);
        Ok(())
    }

    fn put_message(&self, message: SerializedItem) -> NodeResult<()> {
        self.data.write().unwrap().messages.push(message.data);
        Ok(())
    }

    fn put_transaction(&self, transaction: SerializedItem) -> NodeResult<()> {
        self.data
            .write()
            .unwrap()
            .transactions
            .push(transaction.data);
        Ok(())
    }

    fn has_delivery_problems(&self) -> bool {
        false
    }
}
