use error::{AdnlError, AdnlErrorKind, AdnlResult};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::prelude::{Async, Future, Poll};
use ton_api::{BareDeserialize, BoxedDeserialize, BoxedSerialize, Deserializer, Serializer, ConstructorNumber};
use ton_api::ton::TLObject;

pub struct AbortLoop(Arc<AtomicBool>);

impl AbortLoop {
    pub fn with_switch(switch: Arc<AtomicBool>) -> Self {
        AbortLoop(switch)
    }
}

impl Future for AbortLoop {
    type Item = ();
    type Error = ();
    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        if self.0.load(Ordering::Relaxed) {
            Ok(Async::Ready(())) 
        } else {
            Ok(Async::NotReady) 
        }
    }
}

pub fn serialize<T: BoxedSerialize>(object: &T) -> AdnlResult<Vec<u8>> {
    let mut ret = Vec::new();
    Serializer::new(&mut ret)
        .write_boxed(object) 
        .map_err(|err| adnl_err!(AdnlErrorKind::TlSerializationFailed(err)))?;
    Ok(ret)
}

pub fn deserialize<T: BoxedDeserialize>(bytes: &mut &[u8]) -> AdnlResult<T> {
    Deserializer::new(bytes)
        .read_boxed()
        .map_err(|err| adnl_err!(AdnlErrorKind::TlDeserializationFailed(err)))
}

pub fn deserialize_variant(bytes: &[u8]) -> AdnlResult<TLObject> {
    let mut reader = bytes;
    let mut de = Deserializer::new(&mut reader);
    let id = ConstructorNumber(
        i32::deserialize_bare(&mut de)
            .map_err(|err| adnl_err!(AdnlErrorKind::TlDeserializationFailed(err)))?
            as u32,
    );
    info!(target: "adnl", "{:x?}", id);
    TLObject::deserialize_boxed(id, &mut de)
        .map_err(|err| adnl_err!(AdnlErrorKind::TlDeserializationFailed(err)))
}