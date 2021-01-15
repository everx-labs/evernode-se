#![recursion_limit="128"]

// External
extern crate ed25519;
extern crate ed25519_dalek;
#[macro_use]                                                                                               
extern crate error_chain;
#[macro_use]
extern crate log;
extern crate parking_lot;
extern crate rand;
extern crate sha2;
#[cfg(test)]
extern crate serde;
#[cfg(test)]
#[macro_use]
extern crate serde_derive;

// TBD
extern crate ethcore_io as io;
extern crate heapsize;
extern crate itertools;
extern crate keccak_hash as hash;
extern crate kvdb;
#[macro_use]
extern crate macros;
extern crate memory_cache;
extern crate parity_bytes as bytes;
extern crate rlp;
extern crate rustc_hex;
extern crate unexpected;

// In-house
extern crate ton_vm as tvm;
extern crate ton_block;
extern crate ton_types;

pub mod engines;
#[allow(deprecated)]
#[macro_use]
pub mod error;
#[cfg(any(test, feature = "test-helpers"))]
mod test_helpers;

