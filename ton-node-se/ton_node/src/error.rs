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
    #[error("Devp2p network error")]
    NetworkError,
    #[error("TL data serialize error")]
    TlSerializeError,
    #[error("TL data deserialize error")]
    TlDeserializeError,
    #[error("TL packet has unknown type")]
    TlIncompatiblePacketType,
    #[error("Validation empty step error")]
    ValidationEmptyStepError,
    #[error("Validation block error")]
    ValidationBlockError,
    #[error("Signature key invalid")]
    SignatureError,
    #[error("ShardState is invalid")]
    InvalidShardState,
    #[error("Synchronize node ended")]
    SynchronizeEnded,
    #[error("Synchronize node error")]
    SynchronizeError,
    #[error("Synchronize node in process")]
    SynchronizeInProcess,
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

pub(crate) type NodeResult<T> = Result<T, NodeError>;

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
