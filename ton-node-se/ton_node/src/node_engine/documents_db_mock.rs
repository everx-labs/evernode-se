use crate::error::NodeResult;
use ton_block::{Account, Block, Message, Transaction};
use ton_types::{AccountId, UInt256};
use node_engine::blocks_finality::DocumentsDb;

pub struct DocumentsDbMock;

impl DocumentsDb for DocumentsDbMock {
    fn put_account(&self, _: Account) -> NodeResult<()> {
        Ok(())
    }

    fn put_deleted_account(&self, _: i32, _: AccountId) -> NodeResult<()> {
        Ok(())
    }

    fn put_block(&self, _: Block) -> NodeResult<()> {
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
