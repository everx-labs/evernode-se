use bytes::Bytes;
use codec::DataCodec;
use common::{deserialize, serialize, AbortLoop, deserialize_variant};
use config::AdnlServerConfig;
use debug::{dump, TARGET};
use error::{AdnlError, AdnlErrorKind, AdnlResult};
use std::collections::HashMap;
use std::marker::PhantomData;
use std::net::IpAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;
use stream_cancel::StreamExt;
use tokio::codec::Decoder;
use tokio::net::TcpListener;
use tokio::net::TcpStream;
use tokio::prelude::{AsyncSink, Future, Sink, Stream};
use tokio::runtime::Runtime;
use tokio::sync::mpsc::{channel, Sender};
use tokio_io_timeout::TimeoutStream;
use ton_api::ton;
use ton_api::ton::adnl::message::message::{
    Answer as AdnlAnswerMessage, Query as AdnlQueryMessage,
};
use ton_api::ton::adnl::Message as AdnlMessage;
use ton_api::{IntoBoxed};

const DEFAULT_TIMEOUT: Option<Duration> = Some(Duration::from_secs(20));
const MAX_CLIENTS_PER_IP: u16 = 2000;

lazy_static! {
    static ref CLIENTS: Mutex<HashMap<IpAddr, u16>> = {
        let m = Mutex::new(HashMap::new());
        m.lock().unwrap().reserve(10000);
        m
    };
}

#[inline(always)]
fn increase_counter(ip: Option<IpAddr>) {
    if let Some(ip) = ip {
        let mut clients = CLIENTS.lock().unwrap();
        if clients.contains_key(&ip) {
            let m = clients.get_mut(&ip);
            if m.is_some() {
                *m.unwrap() += 1;
            }
        }
    }
}

#[inline(always)]
fn decrease_counter(ip: Option<IpAddr>) {
    if let Some(ip) = ip {
        let mut clients = CLIENTS.lock().unwrap();
        if clients.contains_key(&ip) {
            let m = clients.get_mut(&ip);
            if m.is_some() {
                *m.unwrap() -= 1;
            }
        }
    }
}

#[inline(always)]
fn remove_entry(ip: Option<IpAddr>) {
    if let Some(ip) = ip {
        let mut clients = CLIENTS.lock().unwrap();
        if let Some(i) = clients.get(&ip) {
            if *i == 0 {
                clients.remove_entry(&ip);
            }
        }
    }
}

fn get_ip_addr(stream: &TcpStream) -> Option<IpAddr> {
    let peer_addr = stream.peer_addr();
    if peer_addr.is_ok() {
        Some(peer_addr.unwrap().ip())
    } else {
        None
    }
}

pub trait AdnlServerHandler: Sync + Send {
    fn process_query(&self, q: &Box<AdnlQueryMessage>) -> AdnlResult<Vec<u8>>;

    fn process_message(&self, d: Bytes) -> AdnlResult<Vec<u8>> {
        let obj = deserialize_variant(&d[..])?;
        match obj.downcast::<AdnlMessage>() {
            Ok(msg) => match msg {
                AdnlMessage::Adnl_Message_Query(q) => {
                    return serialize(
                        &AdnlAnswerMessage {
                            query_id: q.query_id,
                            answer: ton::bytes(Vec::from(&self.process_query(&q)?[..])),
                        }
                        .into_boxed(),
                    );
                }
                _ => (),
            },
            _ => (),
        }
        Ok(Vec::from(&d[..]))
    }
}

pub struct AdnlServer<A>
where
    A: AdnlServerHandler + 'static,
{
    phantom: PhantomData<A>,
}

impl<A> AdnlServer<A>
where
    A: AdnlServerHandler + 'static,
{
    pub fn listen(config: AdnlServerConfig, handler: Arc<A>) -> AdnlResult<Runtime> {
        let server = TcpListener::bind(config.address())?
            .incoming()
            .map_err(|e| warn!(target: "adnl", "adnl accept failed = {:?}", e))
            .for_each(move |stream| {
                let ip = get_ip_addr(&stream);
                let abort = Arc::new(AtomicBool::new(false));
                if ip.is_some() {
                    if CLIENTS.lock().unwrap().contains_key(&ip.unwrap()) {
                        increase_counter(ip);
                    }
                    else {
                        CLIENTS.lock().unwrap().insert(ip.unwrap(), 1);
                    }
                    if *CLIENTS.lock().unwrap().get(&ip.unwrap()).unwrap() >= MAX_CLIENTS_PER_IP {
                        warn!("MAX_CLIENTS_PER_IP exceeded! {:?}", ip.unwrap());
                        abort.store(true, Ordering::Relaxed);
                        decrease_counter(ip);
                        remove_entry(ip);
                    }
                }
                let mut s = TimeoutStream::new(stream);
                s.set_write_timeout(config.write_timeout().or(DEFAULT_TIMEOUT));
                s.set_read_timeout(config.read_timeout().or(DEFAULT_TIMEOUT));

                let handler = handler.clone();
                let (net_tx, net_rx) = DataCodec::with_server_config(&config)
                    .framed(s)
                    .split();

                let (mut api_tx, api_rx) = channel(10);
                let api_rx = api_rx
                    .map_err(|err| {
                        let message = format!("Answer TX error {}", err.to_string());
                        log::error!(target: TARGET, "{}", message);
                        std::io::Error::new(std::io::ErrorKind::Other, message)
                    });

                let net_tx = net_tx
                    .send_all(api_rx)
                    .and_then(move |_| {
                        info!(target: TARGET, "Sink channel {:?} closed", ip.unwrap());
                        decrease_counter(ip);
                        remove_entry(ip);
                        Ok(())
                    })
                    .or_else(move |err| {
                        warn!(target: TARGET, "Sink channel {:?} closed with error: {:?}", ip.unwrap(), err);
                        decrease_counter(ip);
                        remove_entry(ip);
                        Err(())
                    });
                tokio::spawn(net_tx);

                let aborter = AbortLoop::with_switch(abort.clone());
                let error = move |err| {
                    decrease_counter(ip);
                    remove_entry(ip);
                    info!(target: TARGET, "{}", err);
                    abort.store(true, Ordering::Relaxed);
                };

                let mut initialized = false;
                let net_rx = net_rx
                    .take_until(aborter)
                    .for_each(move |data_packet| {

                        if !initialized {
                            initialized = true;
                            if data_packet.len() == 0 {
                                if let Err(err) = Self::send(&mut api_tx, Vec::new()) {
                                    error(err)
                                }
                                return Ok(());
                            }
                        }

                        dump("Received packet", &data_packet[..]);
                        let result = deserialize(&mut &data_packet[..]);
                        if let Err(err) = result {
                            error(err)
                        } else if let Ok(AdnlMessage::Adnl_Message_Query(query)) = result {

                            match handler.process_query(&query) {
                                Ok(d) => {
                                    let d = serialize(
                                        &AdnlAnswerMessage {
                                            query_id: query.query_id,
                                            answer: ton::bytes(d)
                                        }.into_boxed()
                                    );
                                    if let Err(err) = d {
                                        error(err)
                                    } else if let Err(err) = Self::send(&mut api_tx, d.unwrap()) {
                                        error(err)
                                    }
                                },
                                Err(err) => error(err)
                            }
                        } else if let Ok(message) = result {
                            error(adnl_err!(AdnlErrorKind::UnsupportedMessage(message)))
                        }

                        Ok(())

                    })
                    .and_then(move |_| {
                        decrease_counter(ip);
                        remove_entry(ip);
                        info!(target: TARGET, "Network RX channel closed {:?}", ip.unwrap());
                        Ok(())
                    })
                    .or_else(move |err| {
                        decrease_counter(ip);
                        remove_entry(ip);
                        warn!(target: "adnl", "Network RX channel closed {:?} with error: {:?}", ip.unwrap(), err);
                        Err(())
                    });

                tokio::spawn(net_rx)

            });

        let mut rt = Runtime::new().unwrap();
        rt.spawn(server);
        Ok(rt)
    }

    fn send(tx: &mut Sender<Vec<u8>>, mut data: Vec<u8>) -> AdnlResult<()> {
        let mut errmsg = None;
        dump("Send packet", &data[..]);
        loop {
            let result = tx.start_send(data);
            data = match result {
                Ok(AsyncSink::Ready) => {
                    let result = tx.poll_complete();
                    if let Err(err) = result {
                        errmsg = Some(err.to_string());
                    }
                    break;
                }
                Ok(AsyncSink::NotReady(data)) => data,
                Err(err) => {
                    errmsg = Some(err.to_string());
                    break;
                }
            }
        }
        if let Some(err) = errmsg {
            Err(adnl_err!(AdnlErrorKind::TransmissionFailed(err)))
        } else {
            Ok(())
        }
    }
}
