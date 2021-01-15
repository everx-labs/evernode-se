#[cfg(feature = "server")]
use ton_api::ton::TLObject;
use ton_api::ton::adnl::Message;
#[cfg(feature = "server")]
use ton_api::ton::adnl::message::message::Query;

error_chain! {

    types {
        AdnlError, AdnlErrorKind, AdnlResultExt, AdnlResult;
    }

    foreign_links {
        Io(std::io::Error);
        Json(serde_json::Error);
        Time(std::time::SystemTimeError);
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
        BadChecksum {
            description("Bad checksum in ADNL packet"),
        }
        BadLength(len: usize) {
            description("Bad length in ADNL packet"),
            display("Bad length in ADNL packet ({})", len)
        }
        #[cfg(feature = "server")]
        QueueIsFull {
            description("Message queue is full"),
        }
        #[cfg(feature = "server")]
        QueryProcessingFailed(msg: String) {
            description("ADNL query processing failed"),
            display("ADNL query processing failed: {}", msg)
        }
        StartingPacketBadChecksum {
            description("Bad starting ADNL packet (bad checksum)"),
        }
        StartingPacketBadLength(len: usize) {
            description("Bad starting ADNL packet (bad length)"),
            display("Bad starting ADNL packet (len == {})", len)
        }
        StartingPacketBadPublicKey {
            description("Bad starting ADNL packet (bad public key)"),
        }
        StartingPacketUnknownId {
            description("Bad starting ADNL packet (unknown ID)"),
        }
        TlDeserializationFailed(err: failure::Error) {
            description("ADNL TL deserialization failed"),
            display("ADNL TL deserialization failed: {}", err)
        }
        TlSerializationFailed(err: failure::Error) {
            description("ADNL TL serialization failed"),
            display("ADNL TL serialization failed: {}", err)
        }
        TransmissionFailed(msg: String) {
            description("Data transmission failed"),
            display("Data transmission failed: {}", msg)
        }
        UnexpectedPacket {
            description("Unexpected ADNL packet"),
        }                                                            	
        #[cfg(feature = "server")]
        UnexpectedTlObject(object: TLObject) {
            description("Wrong TL object in ANDL query"),
            display("Wrong TL object in ADNL query: {:?}", object)
        }
        #[cfg(feature = "client")]
        UnknownAnswer(id: [u8; 32]) {
            description("Unknown ANDL answer"),
            display("Unknown ADNL answer: {}", hex::encode(id))
        }
        UnsupportedMessage(msg: Message) {
            description("Unsupported ANDL message"),
            display("Unsupported ADNL message: {:?}", msg)
        }
        #[cfg(feature = "server")]
        UnsupportedQuery(query: Box<Query>) {
            description("Unsupported ANDL query"),
            display("Unsupported ADNL query: {:?}", query)
        }

    }
    
}

impl From<failure::Error> for AdnlError {
    fn from(error: failure::Error) -> Self {
        AdnlErrorKind::FailureError(error).into()
    }
}

#[macro_export]
macro_rules! adnl_err {
    ($code:expr) => {
        AdnlError::from_kind($code)
    };
}

