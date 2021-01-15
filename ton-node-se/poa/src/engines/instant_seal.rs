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

use engines::{Engine, Seal};
use engines::authority_round::subst::{Machine, Transactions, TotalScoredHeader};

/// `InstantSeal` params.
#[derive(Default, Debug, PartialEq)]
pub struct InstantSealParams {
	/// Whether to use millisecond timestamp
	pub millisecond_timestamp: bool,
}

/*
impl From<::ethjson::spec::InstantSealParams> for InstantSealParams {
	fn from(p: ::ethjson::spec::InstantSealParams) -> Self {
		InstantSealParams {
			millisecond_timestamp: p.millisecond_timestamp,
		}
	}
}
*/

/// An engine which does not provide any consensus mechanism, just seals blocks internally.
/// Only seals blocks which have transactions.
pub struct InstantSeal<M> {
	params: InstantSealParams,
	machine: M,
}

impl<M> InstantSeal<M> {
	/// Returns new instance of InstantSeal over the given state machine.
	pub fn new(params: InstantSealParams, machine: M) -> Self {
		InstantSeal {
			params, machine,
		}
	}
}

impl<M: Machine> Engine<M> for InstantSeal<M>
  where M::LiveBlock: Transactions,
        M::ExtendedHeader: TotalScoredHeader,
        <M::ExtendedHeader as TotalScoredHeader>::Value: Ord
{
	fn name(&self) -> &str {
		"InstantSeal"
	}

	fn machine(&self) -> &M { &self.machine }

	fn seals_internally(&self) -> Option<bool> { Some(true) }

	fn generate_seal(&self, block: &M::LiveBlock, _parent: &M::Header) -> (Seal, Vec<u128>) {
	    if block.transactions().is_empty() { 
                (Seal::None, Vec::new())
            } else { 
                (Seal::Regular(Vec::new()), Vec::new()) 
            }
	}

	fn verify_local_seal(&self, _header: &M::Header) -> Result<(), M::Error> {
		Ok(())
	}

	fn open_block_header_timestamp(&self, parent_timestamp: u64) -> u64 {
		use std::{time, cmp};

		let dur = time::SystemTime::now().duration_since(time::UNIX_EPOCH).unwrap_or_default();
		let mut now = dur.as_secs();
		if self.params.millisecond_timestamp {
			now = now * 1000 + dur.subsec_millis() as u64;
		}
		cmp::max(now, parent_timestamp)
	}

	fn is_timestamp_valid(&self, header_timestamp: u64, parent_timestamp: u64) -> bool {
		header_timestamp >= parent_timestamp
	}

	fn fork_choice(&self, new: &M::ExtendedHeader, current: &M::ExtendedHeader) -> super::ForkChoice {
		super::total_difficulty_fork_choice(new, current)
	}
}

#[cfg(test)]
mod tests {

        use std::str::FromStr;
	use std::sync::Arc;

	use test_helpers::get_temp_state_db;
	use engines::{Engine, Seal};       
        use engines::authority_round::subst::{
            Address, EthereumMachine, H256, H520, IsBlock, OpenBlock, OrdinaryHeader, U256
        };

    fn new_instant() -> super::InstantSeal<EthereumMachine> {
        super::InstantSeal::new(
            super::InstantSealParams {
                millisecond_timestamp: false,
            },
            EthereumMachine::new()
        )
    }

    fn genesis_header() -> OrdinaryHeader {
        let mut header: OrdinaryHeader = Default::default();
        header.set_parent_hash(H256::from_str("0x0000000000000000000000000000000000000000000000000000000000000000").unwrap());
        header.set_timestamp(0);
        header.set_number(0);
        header.set_author(H256::from_str("0x0000000000000000000000000000000000000000").unwrap());
// header.set_transactions_root(self.transactions_root.clone());
// header.set_uncles_hash(keccak(RlpStream::new_list(0).out()));
// header.set_extra_data(self.extra_data.clone());
// header.set_state_root(self.state_root());
// header.set_receipts_root(self.receipts_root.clone());
// header.set_log_bloom(Bloom::default());
// header.set_gas_used(self.gas_used.clone());
        header.set_gas_limit(U256::from(0x7A1222u64));
        header.set_difficulty(U256::from(0x20000u64));
/*
        header.set_seal({
			let r = Rlp::new(&self.seal_rlp);
			r.iter().map(|f| f.as_raw().to_vec()).collect()
	});
*/
//      trace!(target: "spec", "Header hash is {}", header.hash());
        header
    }

        #[ignore]
	#[test]
	fn instant_can_seal() {
		let engine = new_instant();
		let db = get_temp_state_db();//spec.ensure_db_good(get_temp_state_db(), &Default::default()).unwrap();
		let genesis_header = genesis_header();
		let last_hashes = Arc::new(vec![genesis_header.hash()]);
		let b = OpenBlock::new(&engine, /* Default::default(), */ false, db, &genesis_header, last_hashes, 
                    Address::default().into(), (3141562u64.into(), 31415620u64.into()), 
                    vec![], false, &mut Vec::new().into_iter()).unwrap();
		let b = b.close_and_lock().unwrap();
		if let (Seal::Regular(seal), _) = engine.generate_seal(b.block(), &genesis_header.into()) {
			assert!(b.try_seal(&engine, seal).is_ok());
		}
	}

	#[test]
	fn instant_cant_verify() {
		let engine = new_instant();
		let mut header: OrdinaryHeader = OrdinaryHeader::default();
		assert!(engine.verify_block_basic(&header).is_ok());
		header.set_seal(vec![::rlp::encode(&H520::default())]);
		assert!(engine.verify_block_unordered(&header).is_ok());
	}
}
