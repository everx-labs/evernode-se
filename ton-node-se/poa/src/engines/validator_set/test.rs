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

/// Used for Engine testing.

use bytes::Bytes;
use ed25519_dalek::PublicKey;
use heapsize::HeapSizeOf;
use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};

use super::{ValidatorSet, SimpleList};

use engines::{EpochChange};
use engines::authority_round::subst::{
    Address, AuxiliaryData, BlockId, BlockNumber, Call, EthereumMachine, H256, OrdinaryHeader
};
use error::PoaError;

/// Set used for testing with a single validator.
pub struct TestSet {
	validator: SimpleList,
	last_malicious: Arc<AtomicUsize>,
	last_benign: Arc<AtomicUsize>,
}

impl TestSet {
	pub fn new(last_malicious: Arc<AtomicUsize>, last_benign: Arc<AtomicUsize>) -> Self {
            let dat1 = fs::read(Path::new("../config/pub01")).expect("Error reading key file ../config/pub01");
            let pub1 = PublicKey::from_bytes(&dat1).unwrap();
		TestSet {
			validator: SimpleList::new(vec![pub1]),
			last_malicious: last_malicious,
			last_benign: last_benign,
		}
	}
}

impl HeapSizeOf for TestSet {
	fn heap_size_of_children(&self) -> usize {
		self.validator.heap_size_of_children()
	}
}

impl ValidatorSet for TestSet {
	fn default_caller(&self, _block_id: BlockId) -> Box<Call> {
		Box::new(|_, _| Err("Test set doesn't require calls.".into()))
	}

	fn is_epoch_end(&self, _first: bool, _chain_head: &OrdinaryHeader) -> Option<Vec<u8>> { None }

	fn signals_epoch_end(&self, _: bool, _: &OrdinaryHeader, _: AuxiliaryData)
		-> EpochChange<EthereumMachine>
	{
		EpochChange::No
	}

	fn epoch_set(&self, _: bool, _: &EthereumMachine, _: BlockNumber, _: &[u8]) -> Result<(SimpleList, Option<H256>), PoaError> {
		Ok((self.validator.clone(), None))
	}

	fn contains_with_caller(&self, bh: &H256, key_id: &u64, _: &Call) -> bool {
		self.validator.contains(bh, key_id)
	}

	fn get_with_caller(&self, bh: &H256, nonce: usize, _: &Call) -> PublicKey {
		self.validator.get(bh, nonce)
	}

	fn count_with_caller(&self, _bh: &H256, _: &Call) -> usize {
		1
	}

	fn report_malicious(&self, _validator: &Address, _set_block: BlockNumber, block: BlockNumber, _proof: Bytes) {
		self.last_malicious.store(block as usize, AtomicOrdering::SeqCst)
	}

	fn report_benign(&self, _validator: &u64, _set_block: BlockNumber, block: BlockNumber) {
		self.last_benign.store(block as usize, AtomicOrdering::SeqCst)
	}
}
