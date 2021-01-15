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

//! Finality proof generation and checking.

use ed25519_dalek::{PublicKey};
use std::collections::{VecDeque};
use std::collections::hash_map::{HashMap, Entry};
use engines::authority_round::subst::{H256};
use engines::validator_set::SimpleList;
use std::io::Cursor;
use std::io::{ Read, Write };
use std::fs::{File};
use std::path::Path;
use ton_types::types::ByteOrderRead;

/// Error indicating unknown validator.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct UnknownValidator;

/// Rolling finality checker for authority round consensus.
/// Stores a chain of unfinalized hashes that can be pushed onto.
#[derive(Debug, PartialEq)]
pub struct RollingFinality {
	headers: VecDeque<(H256, Vec<u64>)>,
	signers: SimpleList,
	sign_count: HashMap<u64, usize>,
	last_pushed: Option<H256>,
}

impl RollingFinality {

	/// Create a blank finality checker under the given validator set.
	pub fn blank(signers: Vec<PublicKey>) -> Self {
		RollingFinality {
			headers: VecDeque::new(),
			signers: SimpleList::new(signers),
			sign_count: HashMap::new(),
			last_pushed: None,
		}
	}

    pub fn add_signer(&mut self, signer: PublicKey) {
        self.signers.add(signer)
    }

    pub fn remove_signer(&mut self, signer: &u64) {
        self.signers.remove_by_id(signer)
    }

	/// Extract unfinalized subchain from ancestry iterator.
	/// Clears the current subchain.
	///
	/// Fails if any provided signature isn't part of the signers set.
	pub fn build_ancestry_subchain<I>(&mut self, iterable: I) -> Result<(), UnknownValidator>
		where I: IntoIterator<Item=(H256, Vec<u64>)>
	{
		self.clear();
		for (hash, signers) in iterable {

                    self.check_signers(&signers)?;

			if self.last_pushed.is_none() { self.last_pushed = Some(hash.clone()) }

			// break when we've got our first finalized block.
			{
				let current_signed = self.sign_count.len();

				let new_signers = signers.iter().filter(|s| !self.sign_count.contains_key(s)).count();
				let would_be_finalized = (current_signed + new_signers) * 2 > self.signers.len();

				if would_be_finalized {
					trace!(target: "finality", "Encountered already finalized block {:?}", hash.clone());
					break
				}

				for signer in signers.iter() {
					*self.sign_count.entry(signer.clone()).or_insert(0) += 1;
				}
			}

			self.headers.push_front((hash, signers));
		}

		trace!(target: "finality", "Rolling finality state: {:?}", self.headers);
		Ok(())
	}

	/// Clear the finality status, but keeps the validator set.
	pub fn clear(&mut self) {
		self.headers.clear();
		self.sign_count.clear();
		self.last_pushed = None;
	}

	/// Returns the last pushed hash.
	pub fn subchain_head(&self) -> Option<H256> {
		self.last_pushed.clone()
	}

	/// Get an iterator over stored hashes in order.
	#[cfg(test)]
	pub fn unfinalized_hashes(&self) -> impl Iterator<Item=&H256> {
		self.headers.iter().map(|(h, _)| h)
	}


    pub fn save(&self, file_name: &str) -> Result<(), std::io::Error> {
        let mut file_info = File::create(file_name)?;
        let data = self.serialize_info();
        file_info.write_all(&data)?;
        file_info.flush()?;
        Ok(())
    }

    pub fn load(&mut self, file_name: &str) -> Result<(), std::io::Error> {
        if Path::new(file_name).exists() {
            let mut file_info = File::open(file_name)?;
            let mut data = Vec::new();
            file_info.read_to_end(&mut data)?;
            self.deserialize_info(data)?;
        }
        Ok(())
    }

    /// serialize block hashes info 
    pub fn serialize_info(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        //serialize sign_count
        let len = self.sign_count.len();
        buf.extend_from_slice(&(len as u32).to_le_bytes());
        for (sign, count) in self.sign_count.iter() {
            buf.extend_from_slice(&(*sign as u64).to_le_bytes());
            buf.extend_from_slice(&(*count as u64).to_le_bytes());
        }

        //serialize headers
        let len = self.headers.len();
        buf.extend_from_slice(&(len as u32).to_le_bytes());
        for h in self.headers.iter() {
            let (hash, validators) = h.clone();
            buf.append(&mut hash.0.to_vec());
            let keys_count = validators.len();
            buf.extend_from_slice(&(keys_count as u32).to_le_bytes());
            for v in validators.iter() {
                buf.extend_from_slice(&(*v as u64).to_le_bytes());
            }
        }

        buf
    }

    /// deserialize block hashes info 
    pub fn deserialize_info(&mut self, data: Vec<u8>) -> Result<(), std::io::Error> {
        let mut rdr = Cursor::new(data);
        // deserialize sing_count
        let len = rdr.read_le_u32()?;
        for _ in 0..len {
            let sign = rdr.read_le_u64()?;
            let count = rdr.read_le_u64()? as usize;
            self.sign_count.insert(sign, count);
        }

        // deserialize headers
        let len = rdr.read_le_u32()?;
        for _ in 0..len {
            let hash = rdr.read_u256()?;
            let keys_count = rdr.read_le_u32()?;
            let mut keys: Vec<u64> = vec![];
            for _ in 0..keys_count {
                keys.push(rdr.read_le_u64()?);
            }
            self.headers.push_back((H256(hash), keys));
        }
        Ok(())
    }

	/// Get the validator set.
	pub fn validators(&self) -> &SimpleList { &self.signers }

    /// Remove last validator from list
    pub fn remove_last(&mut self) -> Option<(H256, Vec<u64>)> {
        self.headers.pop_back()
    }

    /// Push a hash onto the rolling finality checker (implying `subchain_head` == head.parent)
    ///
    /// Fails if `signer` isn't a member of the active validator set.
    /// Returns a list of all newly finalized headers.
    // TODO: optimize with smallvec.
    pub fn push_hash(&mut self, head: H256, signers: Vec<u64>) -> Result<Vec<H256>, UnknownValidator> {

        self.check_signers(&signers)?;

		for signer in signers.iter() {
			*self.sign_count.entry(signer.clone()).or_insert(0) += 1;
		}

		self.headers.push_back((head.clone(), signers));

		let mut newly_finalized = Vec::new();

		while self.sign_count.len() * 2 > self.signers.len() {
			let (hash, signers) = self.headers.pop_front()
				.expect("headers length always greater than sign count length; qed");

			newly_finalized.push(hash);

			for signer in signers {
				match self.sign_count.entry(signer) {
					Entry::Occupied(mut entry) => {
						// decrement count for this signer and purge on zero.
						*entry.get_mut() -= 1;

						if *entry.get() == 0 {
							entry.remove();
						}
					}
					Entry::Vacant(_) => panic!("all hashes in `header` should have entries in `sign_count` for their signers; qed"),
				}
			}
		}

		trace!(target: "finality", "Blocks finalized by {:?}: {:?}", head, newly_finalized);

		self.last_pushed = Some(head);
		Ok(newly_finalized)
	}

    fn check_signers(&self, signers: &Vec<u64>) -> Result<(), UnknownValidator> {
        for s in signers.iter() {
            if !self.signers.contains_id(s) {
                return Err(UnknownValidator) 
            }
        } 
        Ok(())
    }

}

#[cfg(test)]
mod tests {

    use ed25519_dalek::PublicKey;
    use std::fs;
    use std::path::Path;
    use ton_block::id_from_key;

	use super::RollingFinality;
        use engines::authority_round::subst::{H256};	

    #[test]
    fn test_serialation() {
        let vec = (0..7).map(|_| {
            let pvt_key = ed25519_dalek::SecretKey::generate(&mut rand::thread_rng());
            ed25519_dalek::PublicKey::from(&pvt_key)
        }).collect::<Vec<ed25519_dalek::PublicKey>>();
        let mut bytes = [0u8; 8];
        bytes.copy_from_slice(&vec[0].as_bytes()[0..8]);
        let v1 = u64::from_be_bytes(bytes);
        bytes.copy_from_slice(&vec[1].as_bytes()[0..8]);
        let v2 = u64::from_be_bytes(bytes);
        bytes.copy_from_slice(&vec[2].as_bytes()[0..8]);
        let v3 = u64::from_be_bytes(bytes);
        let mut rf = RollingFinality::blank(vec);
        rf.push_hash(H256([0;32]), vec![v1]).unwrap();
        rf.push_hash(H256([1;32]), vec![v2]).unwrap();
        rf.push_hash(H256([2;32]), vec![v1]).unwrap();
        rf.push_hash(H256([4;32]), vec![v3]).unwrap();
        rf.push_hash(H256([5;32]), vec![v3]).unwrap();

        let data = rf.serialize_info();

        println!("{:?}", data);

        let mut rf2 = RollingFinality::blank(vec![]);
        
        rf2.deserialize_info(data).unwrap();

        assert_eq!(rf.headers, rf2.headers);
    }


    fn get_keys(n: usize) -> (Vec<PublicKey>, Vec<u64>) {
        let mut keys = Vec::new();
        let mut kids = Vec::new();
        for i in 0..n {
            let name = format!("../config/pub{:02}", i+1);
            let data = fs::read(Path::new(&name))
                .expect(&format!("Error reading key file {}", name));
            let key = PublicKey::from_bytes(&data).unwrap();
            kids.push(id_from_key(&key));
            keys.push(key);
        }
        (keys, kids)
    }
 
    #[test]
    fn rejects_unknown_signers() {
        let (signers, key_ids) = get_keys(3);
        let mut finality = RollingFinality::blank(signers);
        assert!(finality.push_hash(H256::random(), vec![key_ids[0], 0xAA]).is_err());
    }

    #[test]
    fn finalize_multiple() {

        let (signers, key_ids) = get_keys(6);
        let mut finality = RollingFinality::blank(signers);
        let hashes: Vec<_> = (0..7).map(|_| H256::random()).collect();
        
        // 3 / 6 signers is < 51% so no finality.
        for (i, hash) in hashes.iter().take(6).cloned().enumerate() {
            let i = i % 3;
            assert!(finality.push_hash(hash, vec![key_ids[i]]).unwrap().len() == 0);
        }

        // after pushing a block signed by a fourth validator, the first four
        // blocks of the unverified chain become verified.
        assert_eq!(
            finality.push_hash(hashes[6].clone(), vec![key_ids[4]]).unwrap(),
            vec![hashes[0].clone(), hashes[1].clone(), hashes[2].clone(), hashes[3].clone()]
        );

    }

    #[test]
    fn finalize_multiple_signers() {
        let (signers, key_ids) = get_keys(6);
        let mut finality = RollingFinality::blank(signers);
        let hash = H256::random();
        // after pushing a block signed by four validators, it becomes verified right away.
        assert_eq!(finality.push_hash(hash.clone(), key_ids).unwrap(), vec![hash]);
    }

    #[test]
    fn from_ancestry() {
        let (signers, key_ids) = get_keys(6);
        let hashes: Vec<_> = (0..12).map(
            |i| (H256::random(), vec![key_ids[i % 6]])
        ).collect();
        let mut finality = RollingFinality::blank(signers);
        finality.build_ancestry_subchain(hashes.iter().rev().cloned()).unwrap();
        assert_eq!(finality.unfinalized_hashes().count(), 3);
        assert_eq!(finality.subchain_head(), Some(hashes[11].clone().0));
    }

    #[test]
    fn from_ancestry_multiple_signers() {
        let (signers, key_ids) = get_keys(6);
        let hashes: Vec<_> = (0..12).map(
            |i| {
                (H256::random(), vec![key_ids[i % 6], key_ids[(i + 1) % 6], key_ids[(i + 2) % 6]])
            }
        ).collect();
        let mut finality = RollingFinality::blank(signers);
        finality.build_ancestry_subchain(hashes.iter().rev().cloned()).unwrap();
        // only the last hash has < 51% of authorities' signatures
        assert_eq!(finality.unfinalized_hashes().count(), 1);
        assert_eq!(finality.unfinalized_hashes().next(), Some(&hashes[11].clone().0));
        assert_eq!(finality.subchain_head(), Some(hashes[11].clone().0));
    }

}
