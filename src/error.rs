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

use std::io;

#[derive(Debug, thiserror::Error)]
pub enum NodeError {
    #[error("Io error: {}", 0)]
    Io(io::Error),
    #[error("Failure error: {}", 0)]
    FailureError(failure::Error),
    #[error("Block error: {}", 0)]
    BlockError(ton_block::BlockError),
    #[error("Requested item not found")]
    NotFound,
    #[error("Database problem")]
    PathError(String),
    #[error("Path problem {}", 0)]
    DataBaseProblem,
    #[error("Invalid external message")]
    InvalidExtMessage,
    #[error("Invalid Merkle update")]
    InvalidMerkleUpdate,
    #[error("Invalid operation")]
    InvalidOperation,
    #[error("Load finality error")]
    LoadFinalityError,
    #[error("Finality error. Block not found into cache")]
    FinalityError,
    #[error("Rollback block error. Block not found into cache")]
    RoolbackBlockError,
    #[error("TL data serialize error")]
    TlSerializeError,
    #[error("TL data deserialize error")]
    TlDeserializeError,
    #[error("TL packet has unknown type")]
    TlIncompatiblePacketType,
    #[error("ShardState is invalid")]
    InvalidShardState,
    #[error("Internal message queue is full")]
    QueueFull,
    #[error("Transaction executor internal error: '{}'", 0)]
    TrExecutorError(String),
    #[error("Invalid data: {}", 0)]
    InvalidData(String),
    #[error("Document DB error: {}", 0)]
    DocumentDbError(String),
    #[error("Config read error: {}", 0)]
    ConfigError(String),
    #[error("SE API failed: {}", 0)]
    ApiError(String),
}

pub type NodeResult<T> = Result<T, NodeError>;

impl From<failure::Error> for NodeError {
    fn from(error: failure::Error) -> Self {
        NodeError::FailureError(error).into()
    }
}

impl From<io::Error> for NodeError {
    fn from(error: io::Error) -> Self {
        NodeError::Io(error).into()
    }
}
