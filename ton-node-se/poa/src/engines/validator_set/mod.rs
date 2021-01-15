// Copyright 2015-2018 Parity Technologies (UK) Ltd.
// This file is part of Parity.

// Parity is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.

// Parity is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.

// You should have received a copy of the GNU General Public License
// along with Parity.  If not, see <http://www.gnu.org/licenses/>.

/// Validator lists.

#[cfg(test)]
mod test;
mod simple_list;
mod safe_contract;
mod contract;
mod multi;

use bytes::Bytes;
use ed25519_dalek::PublicKey;
use std::sync::Weak;

use error::PoaError;
use engines::{EpochChange};
use engines::authority_round::subst::{
   Address, AuxiliaryData, BlockId, BlockNumber, Call, EngineClient, EthereumMachine, H256, OrdinaryHeader,
   ValidatorSpec
};

#[cfg(test)]
pub use self::test::TestSet;
pub use self::simple_list::{PublicKeyListImport, SimpleList};
use self::contract::ValidatorContract;
use self::safe_contract::ValidatorSafeContract;
use self::multi::Multi;
use super::SystemCall;

/// Creates a validator set from spec.
pub fn new_validator_set(spec: ValidatorSpec) -> Box<dyn ValidatorSet> {
	match spec {
		ValidatorSpec::List(list) => Box::new(SimpleList::new(list.into_iter().map(Into::into).collect())),
		ValidatorSpec::SafeContract(address) => Box::new(ValidatorSafeContract::new(address.into())),
		ValidatorSpec::Contract(address) => Box::new(ValidatorContract::new(address.into())),
		ValidatorSpec::Multi(sequence) => Box::new(
			Multi::new(sequence.into_iter().map(|(block, set)| (block.into(), new_validator_set(set))).collect())
		),
	}
}

/// A validator set.
pub trait ValidatorSet: Send + Sync + 'static {
	/// Get the default "Call" helper, for use in general operation.
	// TODO [keorn]: this is a hack intended to migrate off of
	// a strict dependency on state always being available.
	fn default_caller(&self, block_id: BlockId) -> Box<Call>;

	/// Checks if a given address is a validator,
	/// using underlying, default call mechanism.
	fn contains(&self, parent: &H256, key_id: &u64) -> bool {
		let default = self.default_caller(BlockId::Hash(parent.clone()));
		self.contains_with_caller(parent, key_id, &*default)
	}

	/// Draws an validator nonce modulo number of validators.
	fn get(&self, parent: &H256, nonce: usize) -> PublicKey {
		let default = self.default_caller(BlockId::Hash(parent.clone()));
		self.get_with_caller(parent, nonce, &*default)
	}

	/// Returns the current number of validators.
	fn count(&self, parent: &H256) -> usize {
		let default = self.default_caller(BlockId::Hash(parent.clone()));
		self.count_with_caller(parent, &*default)
	}

	/// Signalling that a new epoch has begun.
	///
	/// All calls here will be from the `SYSTEM_ADDRESS`: 2^160 - 2
	/// and will have an effect on the block's state.
	/// The caller provided here may not generate proofs.
	///
	/// `first` is true if this is the first block in the set.
	fn on_epoch_begin(&self, _first: bool, _header: &OrdinaryHeader, _call: &mut SystemCall) -> Result<(), PoaError> {
		Ok(())
	}

	/// Extract genesis epoch data from the genesis state and header.
	fn genesis_epoch_data(&self, _header: &OrdinaryHeader, _call: &Call) -> Result<Vec<u8>, String> { Ok(Vec::new()) }

	/// Whether this block is the last one in its epoch.
	///
	/// Indicates that the validator set changed at the given block in a manner
	/// that doesn't require finality.
	///
	/// `first` is true if this is the first block in the set.
	fn is_epoch_end(&self, first: bool, chain_head: &OrdinaryHeader) -> Option<Vec<u8>>;

	/// Whether the given block signals the end of an epoch, but change won't take effect
	/// until finality.
	///
	/// Engine should set `first` only if the header is genesis. Multiplexing validator
	/// sets can set `first` to internal changes.
	fn signals_epoch_end(
		&self,
		first: bool,
		header: &OrdinaryHeader,
		aux: AuxiliaryData,
	) -> EpochChange<EthereumMachine>;

	/// Recover the validator set from the given proof, the block number, and
	/// whether this header is first in its set.
	///
	/// May fail if the given header doesn't kick off an epoch or
	/// the proof is invalid.
	///
	/// Returns the set, along with a flag indicating whether finality of a specific
	/// hash should be proven.
	fn epoch_set(&self, first: bool, machine: &EthereumMachine, number: BlockNumber, proof: &[u8])
		-> Result<(SimpleList, Option<H256>), PoaError>;

	/// Checks if a given address is a validator, with the given function
	/// for executing synchronous calls to contracts.
	fn contains_with_caller(&self, parent_block_hash: &H256, key_id: &u64, caller: &Call) -> bool;

	/// Draws an validator nonce modulo number of validators.
	fn get_with_caller(&self, parent_block_hash: &H256, nonce: usize, caller: &Call) -> PublicKey;

	/// Returns the current number of validators.
	fn count_with_caller(&self, parent_block_hash: &H256, caller: &Call) -> usize;

	/// Notifies about malicious behaviour.
	fn report_malicious(&self, _validator: &Address, _set_block: BlockNumber, _block: BlockNumber, _proof: Bytes) {}
	/// Notifies about benign misbehaviour.
	fn report_benign(&self, _validator: &u64, _set_block: BlockNumber, _block: BlockNumber) {}
	/// Allows blockchain state access.
	fn register_client(&self, _client: Weak<dyn EngineClient>) {}
}

/*
impl ValidatorSet for Vec<PublicKey> {
	fn default_caller(&self, _block_id: BlockId) -> Box<Call> {
		Box::new(|_, _| Err("Keys list doesn't require calls.".into()))
	}
	fn is_epoch_end(&self, first: bool, _chain_head: &OrdinaryHeader) -> Option<Vec<u8>> {
		match first {
			true => Some(Vec::new()), // allow transition to fixed list, and instantly
			false => None,
		}
	}
	fn signals_epoch_end(&self, _: bool, _: &OrdinaryHeader, _: AuxiliaryData)
		-> ::extension::engines::EpochChange<EthereumMachine>
	{
		::extension::engines::EpochChange::No
	}
	fn epoch_set(&self, _first: bool, _: &EthereumMachine, _: BlockNumber, _: &[u8]) -> Result<(Vec<PublicKey>, Option<H256>), Error> {
		Ok((self.clone(), None))
	}
	fn contains_with_caller(&self, _bh: &H256, address: &Address, _: &Call) -> bool {
		self.validators.contains(address)
	}
	fn get_with_caller(&self, _bh: &H256, nonce: usize, _: &Call) -> Address {
		let validator_n = self.validators.len();
		if validator_n == 0 {
			warn!("Cannot operate with an empty validator set.");
			panic!("Cannot operate with an empty validator set.");
		}
		self.validators.get(nonce % validator_n).expect("There are validator_n authorities; taking number modulo validator_n gives number in validator_n range; qed").clone()
	}
	fn count_with_caller(&self, _bh: &H256, _: &Call) -> usize {
		self.len()
	}
}
*/
