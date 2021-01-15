#![recursion_limit="128"]

extern crate aes_ctr;
#[macro_use]
extern crate arrayref;
extern crate base64;
extern crate bytes;
extern crate curve25519_dalek;
extern crate ed25519_dalek;
#[macro_use]
extern crate error_chain;
extern crate failure;
#[macro_use]
extern crate log;
extern crate rand;
extern crate serde;
extern crate serde_json;
extern crate sha2;
#[cfg(feature = "server")]
extern crate stream_cancel;
extern crate tokio;
extern crate ton_vm as tvm;
extern crate ton_block;
extern crate ton_api;
extern crate x25519_dalek;
extern crate socket2;
extern crate tokio_io_timeout;
#[cfg(feature = "server")]
#[macro_use]
extern crate lazy_static;

#[macro_use]
pub mod error;
mod codec;
pub mod common;
pub mod config;
#[cfg(feature = "client")]
pub mod client;
mod debug;
#[cfg(feature = "server")]
pub mod server;
