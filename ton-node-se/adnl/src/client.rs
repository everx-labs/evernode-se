use bytes::BytesMut;
use codec::DataCodec;
use common::{AbortLoop, deserialize, serialize, deserialize_variant};
use config::AdnlClientConfig;
use debug::{dump, TARGET};
use error::{AdnlError, AdnlErrorKind, AdnlResult};
use rand::Rng;
use socket2::{Domain, Protocol, Socket, Type};
use std::collections::HashMap;
use std::fmt::{self, Debug, Formatter};
use std::io::Write;
use std::net::SocketAddr;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::codec::Decoder;
use tokio::net::TcpStream;
use tokio::sync::mpsc::UnboundedSender;
use tokio::prelude::{Future, Sink, Stream};
use ton_api::{BoxedDeserialize, BoxedSerialize};
use ton_api::ton;
use ton_api::ton::adnl::Message as AdnlMessage;
use ton_api::ton::adnl::message::message::Query as AdnlQueryMessage;
use ton_api::ton::rpc::lite_server::Query as LiteServerQuery;
use tokio_io_timeout::TimeoutStream;
use ton_api::ton::TLObject;

type QueryCallback = dyn Fn(Answer) + Send + Sync + 'static;
type QueryId = [u8; 32];

const DEFAULT_TIMEOUT: Option<Duration> = Some(Duration::from_secs(2));

/// Query to the Node
struct Query {
    id: QueryId,
    data: Vec<u8>,
    callback: Arc<QueryCallback>,
}

impl Query {
    pub fn new(id: QueryId, data: Vec<u8>, callback: Arc<QueryCallback>) -> Self {
        Self { id, data, callback }
    }
    pub fn id(&self) -> QueryId {
        self.id
    }
    pub fn data(&self) -> &Vec<u8> {
        &self.data
    }
    pub fn callback(&self) -> Arc<QueryCallback> {
        Arc::clone(&self.callback)
    }
}

impl Debug for Query {
    fn fmt(&self, f: &mut Formatter) -> Result<(), fmt::Error> {
        f.write_fmt(format_args!(
            "Query id: {:?}, data: {:?}",
            self.id, self.data
        ))
    }
}

/// Answer from the Node
#[derive(Debug)]
pub struct Answer {
    query_id: QueryId,
    data: Vec<u8>,
}

impl Answer {
    pub(crate) fn new(query_id: QueryId, data: Vec<u8>) -> Self {
        Answer { query_id, data }
    }
    pub fn query_id(&self) -> &QueryId {
        &self.query_id
    }
    pub fn data<T: BoxedDeserialize>(&mut self) -> AdnlResult<T> {
        deserialize(&mut self.data.as_slice())
    }
    pub fn variant(&mut self) -> AdnlResult<TLObject> {
        deserialize_variant(&mut self.data.as_slice())
    }
}

/// ADNL protocol client implementation.
pub struct AdnlClient {
    query_sender: Option<UnboundedSender<Query>>,
    abort: Arc<AtomicBool>,
    error_callback: Option<Arc<dyn Fn(AdnlError) + Send + Sync + 'static>>,
}

fn create_custom_socket() -> Result<std::net::TcpStream, std::io::Error> {
    let socket = Socket::new(Domain::ipv4(), Type::stream(), Some(Protocol::tcp()))?;

    socket.set_reuse_address(true)?;
    socket.set_linger(Some(Duration::from_secs(0)))?;
    socket.bind(&"0.0.0.0:0".parse::<SocketAddr>().unwrap().into())?;

    Ok(socket.into_tcp_stream())
}

impl AdnlClient {

    /// Constructs new ADNL client.
    pub fn new() -> AdnlClient {
        AdnlClient {
            query_sender: None,
            abort: Arc::new(AtomicBool::new(false)),
            error_callback: None
        }
    }

    /// Constructs connection future
    pub fn connect(&mut self, config: &AdnlClientConfig, read_timeout: Option<Duration>, write_timeout: Option<Duration>) -> impl Future<Item=(), Error=AdnlError> {

        let mut data_codec = DataCodec::with_client_config(config);
        let init_packet = data_codec.build_init_packet();

        let (api_tx, api_rx) = tokio::sync::mpsc::unbounded_channel();
        self.query_sender = Some(api_tx);
        let error_callback_opt = self.error_callback.clone();//Arc::clone(&self.error_callback);
        let abort = self.abort.clone();

        let std_socket = create_custom_socket();

        let connect = TcpStream::connect_std(
            std_socket.unwrap(),
            &config.server_address(),
            &tokio::reactor::Handle::default(),
        )
        .and_then(move |mut stream| {
            stream.write_all(&init_packet[..])?;
            let mut s = TimeoutStream::new(stream);
            s.set_write_timeout(write_timeout.or(DEFAULT_TIMEOUT));
            s.set_read_timeout(read_timeout.or(DEFAULT_TIMEOUT));

            Ok(s)
        })
        .and_then(|stream| {
            let mut buf = BytesMut::with_capacity(4);
            buf.resize(4, 0);
            tokio::io::read_exact(stream, buf)
        })
        .map_err(|err| 
            adnl_err!(AdnlErrorKind::TransmissionFailed(err.to_string()))
        )
        .and_then(move |(stream, mut buf)| {

            data_codec.decode(&mut buf)?;
            let (net_tx, net_rx) = data_codec.framed(stream).split();

            let queries_map_tx =
                Arc::new(Mutex::new(HashMap::<QueryId, Arc<QueryCallback>>::new()));
            let queries_map_rx = queries_map_tx.clone();

            debug!(target: TARGET, "Starting to process queries...");
            let api_rx = api_rx
                .map(move |query| {
                    debug!(target: TARGET, "{:?}", query);
                    queries_map_tx
                        .lock()
                        .expect("Cannot lock query HashMap")
                        .insert(query.id(), query.callback());
                    query.data().to_vec()
                })
                .map_err(|err| {
                    let message = format!("Query TX error {}", err.to_string());
                    log::error!(target: TARGET, "{}", message);
                    std::io::Error::new(std::io::ErrorKind::Other, message)
                });

            let net_tx = net_tx
                .send_all(api_rx)
                .and_then(|_| {
                    debug!(target: TARGET, "Network TX channel closed");
                    Ok(())
                })
                .or_else(|err| {
                    log::error!(target: TARGET, "Network TX channel closed with error: {:?}", err);
                    Err(())
                });

            let net_rx = net_rx
                .filter(|data_packet| data_packet.len() > 0)
                .for_each(move |data_packet| {
                    let error = |err| {
                        log::error!(target: TARGET, "{}", err);
                        if let Some(ref callback) = error_callback_opt {
                            callback(err)
                        }
                    };
                    dump(format!("Received packet ({} bytes)", &data_packet.len()).as_str(), &data_packet[..]);
                    let result = deserialize(&mut &data_packet[..]);
                    if let Err(err) = result {
                        error(err)
                    } else if let Ok(AdnlMessage::Adnl_Message_Answer(answer)) = result {
                        let callback = queries_map_rx
                            .lock()
                            .expect("Cannot lock query HashMap")
                            .remove(&answer.query_id.0);
                        if let Some(callback) = callback {
                            callback(Answer::new(answer.query_id.0, answer.answer.0))
                        } else {
                            error(adnl_err!(AdnlErrorKind::UnknownAnswer(answer.query_id.0)))
                        }
                    } else if let Ok(message) = result {
                        error(adnl_err!(AdnlErrorKind::UnsupportedMessage(message)))
                    }
                    Ok(())
                })
                .and_then(|_| {
                    debug!(target: TARGET, "Network RX channel closed");
                    Ok(())
                })
                .or_else(|err| {
                    log::error!(target: TARGET, "Network RX channel closed with error: {:?}", err);
                    Err(())
                });

            let network = net_tx.join(net_rx).map(|_| ());
            tokio::spawn(
                AbortLoop::with_switch(abort)
                    .select(network) 
                    .map(|_| ())
                    .map_err(|_| ())
            );
            Ok(())

        });

        connect

    }

    /// Asynchronously sends object to the remote Node.
    pub fn query<O, F>(&mut self, object: &O, callback: F) -> AdnlResult<()>
    where
        O: BoxedSerialize,
        F: Fn(Answer) + Send + Sync + 'static,
    {
        let query_id: QueryId = rand::thread_rng().gen();                        
        let query = AdnlMessage::Adnl_Message_Query(Box::new(
            AdnlQueryMessage {
                query_id: ton::int256(query_id.clone()),
                query: ton::bytes(serialize(
                    &LiteServerQuery {
                        data: ton::bytes(serialize(object)?),
                    }
                )?),
            }));
        let serialized = serialize(&query)?;
        self.query_sender
            .as_mut()
            .expect("Call connect() first, then pass the result to Tokio runtime.")
            .try_send(Query::new(query_id, serialized, Arc::new(callback)))
            .map_err(|err| 
                adnl_err!(AdnlErrorKind::TransmissionFailed(err.to_string()))
            )
    }

    /// Sets a closure for processing of all errors.
    pub fn set_error_callback<F: Fn(AdnlError) + Send + Sync + 'static>(&mut self, callback: F) {
        self.error_callback = Some(Arc::new(callback));
    }

    /// Resets an error processing closure.
    pub fn reset_error_callback(&mut self) {
        self.error_callback = None;
    }

}
