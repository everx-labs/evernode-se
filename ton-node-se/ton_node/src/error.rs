extern crate ethcore_network as network;

// use poa::error::PoaError;
use std::io;
// use tvm::types::Exception;

error_chain! {

    types {
        NodeError, NodeErrorKind, NodeResultExt, NodeResult;
    }

    foreign_links {
        Eth(network::Error);
        Io(io::Error);
    }

    errors {
        FailureError(error: failure::Error) {
            description("Failure error"),
            display("Failure error: {}", error.to_string())
        }
        BlockError(error: ton_block::BlockError) {
            description("Block error"),
            display("Block error: {}", error.to_string())
        }
        NotFound {
            description("Requested item not found")
        }
        DataBaseProblem {
            description("Database problem")
        }
        InvalidExtMessage {
            description("Invalid external message")
        }
        InvalidMerkleUpdate {
            description("Invalid Merkle update")
        }
        InvalidOperation {
            description("Invalid operation")
        }
        LoadFinalityError {
            description("Load finality error")
        }
        FinalityError {
            description("Finality error. Block not found into cache")
        }
        RoolbackBlockError {
            description("Rollback block error. Block not found into cache")
        }
        NetworkError {
            description("Devp2p network error")
        }
        TlSerializeError {
            description("TL data serialize error")
        }
        TlDeserializeError {
            description("TL data deserialize error")
        }
        TlIncompatiblePacketType {
            description("TL packet have anover type")
        }
        ValidationEmptyStepError {
            description("Validation empty step error")
        }
        ValidationBlockError {
            description("Validation block error")
        }
        SignatureError {
            description("Signature key invalid")
        }
        InvalidShardState {
            description("ShardState is invalid")
        }
        SynchronizeEnded {
            description("Synchronize node ended.")
        }
        SynchronizeError {
            description("Synchronize node error.")
        }
        SynchronizeInProcess {
            description("Synchronize node in process.")
        }
        QueueFull {
            description("Internal message queue is full")
        }
        TrExecutorError(t: String) {
            description("Transaction executor internal error")
            display("Transaction executor internal error: '{}'", t)
        }
        InvalidData(msg: String) {
            description("Invalid data"),
            display("Invalid data: {}", msg)
        }
        DocumentDbError(msg: String) {
            description("Document DB error"),
            display("Document DB error: {}", msg)
        }
    }
}

#[macro_export]
macro_rules! node_err {
    ($code:expr) => {
        Err(NodeError::from_kind($code))
    };
}

// error_chain! 

//     types {
//         TonError, TonErrorKind, TonResultExt, TonResult;
//     }

//     foreign_links {
//         Adnl(AdnlError) #[doc = "Adnl error"];
//         Node(NodeError) #[doc = "Node error"];
//         Poa(PoaError) #[doc = "PoA error"];
//         Tvm(Exception) #[doc = "TVM exception"];
//         Block(BlockError) #[doc = "TonBlock error"];
//     }

// }

// impl From<Exception> for NodeError {
//     fn from(error: Exception) -> Self {
//         NodeError::from_kind(NodeErrorKind::Tvm(TvmError::from_kind(TvmErrorKind::Tvm(error))))
//     }
// }

impl From<failure::Error> for NodeError {
    fn from(error: failure::Error) -> Self {
        NodeErrorKind::FailureError(error).into()
    }
}
