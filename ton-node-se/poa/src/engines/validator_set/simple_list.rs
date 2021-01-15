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

/// Preconfigured validator list.

use ed25519_dalek::{PublicKey};
use heapsize::HeapSizeOf;

use engines::{EpochChange};
use engines::validator_set::ValidatorSet;
use engines::authority_round::subst::{
    AuxiliaryData, BlockId, BlockNumber, Call, EthereumMachine, H256, OrdinaryHeader
};
use error::PoaError;
use ton_block::id_from_key;

pub trait PublicKeyListImport {
    fn import(json: &str) -> Result<Vec<PublicKey>, String>;
}

/// Validator set containing a known set of addresses.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SimpleList {
    validators: Vec<PublicKey>,
}                                                                                          

impl SimpleList {
    /// Create a new `SimpleList`.
    pub fn new(validators: Vec<PublicKey>) -> Self {
        SimpleList {
            validators: validators,
        }
    }
    /// Convert into inner representation.
    pub fn into_inner(self) -> Vec<PublicKey> {
        self.validators
    }
    /// Check presense of key ID in list
    pub fn contains_id(&self, id: &u64) -> bool {
        let bytes = id.to_be_bytes();
        self.validators.iter().any(|validator| validator.to_bytes()[0..8] == bytes)
    }
    /// Get key ID by index
    pub fn get_id(&self, index: usize) -> u64 {
        id_from_key(&self.validators[index])
    }
    /// Get key by index
    pub fn get_key(&self, index: usize) -> &PublicKey {
        &self.validators[index]
    }
    /// Add to
    pub fn add(&mut self, key: PublicKey) {
        let bytes = id_from_key(&key).to_be_bytes();
        if !self.validators.iter().any(|validator| validator.to_bytes()[0..8] == bytes) {
            self.validators.push(key);
        }
    }
    /// Insert
    pub fn insert(&mut self, index: usize, key: PublicKey) {
        self.validators.insert(index, key);
    }
    /// Remove by id
    pub fn remove_by_id(&mut self, id: &u64) {
        let bytes = id.to_be_bytes();
        if let Some(i) = self.validators.iter().position(|validator| validator.to_bytes()[0..8] == bytes) {
            self.validators.remove(i);
        }
    }
    /// Remove by index
    pub fn remove(&mut self, index: usize) {
        self.validators.remove(index);
    }
}

impl ::std::ops::Deref for SimpleList {
	type Target = [PublicKey];

	fn deref(&self) -> &[PublicKey] { &self.validators }
}

impl From<Vec<PublicKey>> for SimpleList {
	fn from(validators: Vec<PublicKey>) -> Self {
		SimpleList {
			validators: validators,
		}
	}
}

impl HeapSizeOf for SimpleList {
	fn heap_size_of_children(&self) -> usize {
                self.validators.len()*32*2
	}
}

impl ValidatorSet for SimpleList {
	fn default_caller(&self, _block_id: BlockId) -> Box<Call> {
		Box::new(|_, _| Err("Simple list doesn't require calls.".into()))
	}

	fn is_epoch_end(&self, first: bool, _chain_head: &OrdinaryHeader) -> Option<Vec<u8>> {
		match first {
			true => Some(Vec::new()), // allow transition to fixed list, and instantly
			false => None,
		}
	}

	fn signals_epoch_end(&self, _: bool, _: &OrdinaryHeader, _: AuxiliaryData)
		-> EpochChange<EthereumMachine>
	{
		EpochChange::No
	}

	fn epoch_set(&self, _first: bool, _: &EthereumMachine, _: BlockNumber, _: &[u8]) -> Result<(SimpleList, Option<H256>), PoaError> {
		Ok((self.clone(), None))
	}

	fn contains_with_caller(&self, _bh: &H256, key_id: &u64, _: &Call) -> bool {
		self.contains_id(key_id)
	}

	fn get_with_caller(&self, _bh: &H256, nonce: usize, _: &Call) -> PublicKey {
		let validator_n = self.validators.len();

		if validator_n == 0 {
			warn!("Cannot operate with an empty validator set.");
			panic!("Cannot operate with an empty validator set.");
		}

        info!(target: "engine", "Expected signer {} (#{}) from {} validators\n-----------------------\n", nonce % validator_n, nonce, validator_n);
        debug!(target: "engine", "Validators {:?}", self.validators);
		self.validators.get(nonce % validator_n).expect(
                    "There are validator_n authorities; taking number modulo validator_n gives number in validator_n range; qed"
                ).clone()
	}

	fn count_with_caller(&self, _bh: &H256, _: &Call) -> usize {
		self.validators.len()
	}
}

impl AsRef<dyn ValidatorSet> for SimpleList {
    fn as_ref(&self) -> &dyn ValidatorSet {
        self
    }
}

#[cfg(test)]
mod tests {

    use super::super::ValidatorSet;
    use super::{SimpleList, PublicKeyListImport};
    //use serde::Deserialize;
    use std::fs;
    use std::path::Path;
    use ed25519_dalek::{PublicKey};
    use ton_block::id_from_key;

    /// Public key list importer from JSON 
    #[allow(dead_code)] 
    #[derive(Deserialize)]
    pub struct PublicKeyListImporter {
        private_key: String,
        keys: Vec<String>,
    }

    impl PublicKeyListImport for PublicKeyListImporter{
        /// Import key values from list of files
        fn import(json: &str) -> Result<Vec<PublicKey>, String> {
            let importer: PublicKeyListImporter = 
                serde_json::from_str(json).map_err(|e| e.to_string())?;
            let mut ret = Vec::new();
            for path in importer.keys.iter() {
                let data = fs::read(Path::new(path))
                    .map_err(|_| format!("Error reading key file {}", path))?;
                ret.push(
                    PublicKey::from_bytes(&data)
                        .map_err(|_| format!("Cannot import key from {}", path))?
                );
            }
            Ok(ret)
        }    
    }

    #[test]
    fn simple_list() {
        let keys = PublicKeyListImporter::import(
            "{ 
               \"private_key\": \"../config/key01\", 
               \"keys\": [\"../config/pub01\",\"../config/pub02\",\"../config/pub03\"] 
             }"
        ).unwrap();    
        let list = SimpleList::new(keys.clone());
        let kid1 = id_from_key(&keys[0]);
        assert!(list.contains(&Default::default(), &kid1));
        assert_eq!(list.get(&Default::default(), 0), keys[0]);
        assert_eq!(list.get(&Default::default(), 1), keys[1]);
        assert_eq!(list.get(&Default::default(), 2), keys[2]);
    }

}
