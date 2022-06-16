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

use crate::error::NodeResult;
use ton_block::{Account, Block, Message, Transaction};
use ton_types::{AccountId, UInt256};
use super::DocumentsDb;

pub struct DocumentsDbMock;

impl DocumentsDb for DocumentsDbMock {
    fn put_account(&self, _: Account) -> NodeResult<()> {
        Ok(())
    }

    fn put_deleted_account(&self, _: i32, _: AccountId) -> NodeResult<()> {
        Ok(())
    }

    fn put_block(&self, _: &Block) -> NodeResult<()> {
        Ok(())
    }

    fn put_message(
        &self,
        _: Message,
        _: Option<UInt256>,
        _: Option<u32>,
        _: Option<UInt256>,
    ) -> NodeResult<()> {
        Ok(())
    }

    fn put_transaction(&self, _: Transaction, _: Option<UInt256>, _: i32) -> NodeResult<()> {
        Ok(())
    }

    fn has_delivery_problems(&self) -> bool {
        false
    }
}
