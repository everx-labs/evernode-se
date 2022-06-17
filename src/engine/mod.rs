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

use super::error::NodeResult;
use parking_lot::Mutex;
use std::clone::Clone;
use std::collections::{HashMap, VecDeque};
use std::convert::From;
use std::sync::Arc;
use ton_block::{
    Account, Block, CurrencyCollection, GetRepresentationHash, Serializable, ShardIdent,
    Transaction,
};
use ton_types::{AccountId, Cell, UInt256};

pub mod messages;
pub use self::messages::*;
pub mod engine;

lazy_static::lazy_static! {
    static ref ACCOUNTS: Mutex<Vec<AccountId>> = Mutex::new(vec![]);
    static ref SUPER_ACCOUNT_ID: AccountId = AccountId::from([0; 32]);
}

const GIVER_BALANCE: u128 = 5_000_000_000_000_000_000;
const MULTISIG_BALANCE: u128 = 1_000_000_000_000_000;
const GIVER_ABI1_DEPLOY_MSG: &[u8] = include_bytes!("../../data/giver_abi1_deploy_msg.boc");
const DEPRECATED_GIVER_ABI2_DEPLOY_MSG: &[u8] =
    include_bytes!("../../data/deprecated_giver_abi2_deploy_msg.boc");
const GIVER_ABI2_DEPLOY_MSG: &[u8] = include_bytes!("../../data/giver_abi2_deploy_msg.boc");
const MULTISIG_DEPLOY_MSG: &[u8] = include_bytes!("../../data/safemultisig_deploy_msg.boc");

pub trait MessagesReceiver: Send {
    fn run(&mut self, queue: Arc<InMessagesQueue>) -> NodeResult<()>;
}

pub trait LiveControl: Send + Sync {
    fn increase_time(&self, delta: u32) -> NodeResult<()>;
    fn reset_time(&self) -> NodeResult<()>;
}

pub trait LiveControlReceiver: Send + Sync {
    fn run(&self, control: Box<dyn LiveControl>) -> NodeResult<()>;
}

#[cfg(test)]
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
