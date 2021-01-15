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

/// Validator set maintained in a contract, updated using `getValidators` method.

use bytes::Bytes;
use ed25519_dalek::{PublicKey};
//use hash::keccak;
use kvdb::DBValue;
use memory_cache::MemoryLruCache;
use parking_lot::RwLock;
use rlp::{Rlp, RlpStream};
use std::sync::{Weak, Arc};
//use unexpected::Mismatch;

use engines::{EngineError, EpochChange, Proof, StateDependentProof, SystemCall};
use engines::validator_set::ValidatorSet;
use engines::validator_set::simple_list::SimpleList;
use error::{/*BlockError,*/ PoaError};

use engines::authority_round::subst::{
    Address, AuxiliaryData, AuxiliaryRequest, BlockId, BlockNumber, Call, EngineClient, 
    EthereumMachine, H256, OrdinaryHeader, Receipt 
};

//use_contract!(validator_set, "src/extension/res/contracts/validator_set.json");

const MEMOIZE_CAPACITY: usize = 500;

// TODO: ethabi should be able to generate this.
/*
const EVENT_NAME: &'static [u8] = &*b"InitiateChange(bytes32,address[])";

lazy_static! {
	static ref EVENT_NAME_HASH: H256 = keccak(EVENT_NAME).into();
}
*/

// state-dependent proofs for the safe contract:
// only "first" proofs are such.
struct StateProof {
	contract_address: Address,
	header: OrdinaryHeader,
}

impl StateDependentProof<EthereumMachine> for StateProof {
	fn generate_proof(&self, caller: &Call) -> Result<Vec<u8>, String> {
		prove_initial(self.contract_address.clone(), &self.header, caller)
	}

	fn check_proof(&self, machine: &EthereumMachine, proof: &[u8]) -> Result<(), String> {
		let (header, state_items) = decode_first_proof(&Rlp::new(proof))
			.map_err(|e| format!("proof incorrectly encoded: {}", e))?;
		if &header != &self.header {
			return Err("wrong header in proof".into());
		}

		check_first_proof(machine, self.contract_address.clone(), header, &state_items).map(|_| ())
	}
}

/// The validator contract should have the following interface:
pub struct ValidatorSafeContract {
	contract_address: Address,
	validators: RwLock<MemoryLruCache<H256, SimpleList>>,
	client: RwLock<Option<Weak<dyn EngineClient>>>, // TODO [keorn]: remove
}

#[allow(dead_code)]
// first proof is just a state proof call of `getValidators` at header's state.
fn encode_first_proof(header: &OrdinaryHeader, state_items: &[Vec<u8>]) -> Bytes {
	let mut stream = RlpStream::new_list(2);
	stream.append(header).begin_list(state_items.len());
	for item in state_items {
		stream.append(item);
	}

	stream.out()
}

// check a first proof: fetch the validator set at the given block.
fn check_first_proof(_machine: &EthereumMachine, _contract_address: Address, _old_header: OrdinaryHeader, _state_items: &[DBValue])
	-> Result<Vec<PublicKey>, String>	
{

	// TODO: match client contract_call_tx more cleanly without duplication.
//	const PROVIDED_GAS: u64 = 50_000_000;

/*
	let env_info = ::vm::EnvInfo {
		number: old_header.number(),
		author: old_header.author().into(),
		difficulty: old_header.difficulty().into(),
		gas_limit: PROVIDED_GAS.into(),
		timestamp: old_header.timestamp(),
		last_hashes: {
			// this will break if we don't inclue all 256 last hashes.
			let mut last_hashes: Vec<_> = (0..256).map(|_| H256::default()).collect();
			last_hashes[255] = old_header.parent_hash().clone();
			Arc::new(last_hashes.iter().map(|h| h.into()).collect())
		},
		gas_used: 0.into(),
	};
*/

/*
	// check state proof using given machine.
	let number = old_header.number();
	let (data, decoder) = validator_set::functions::get_validators::call();

	let from = Address::default();
	let tx = Transaction {
		nonce: machine.account_start_nonce(number).into(),
		action: Action::Call(contract_address.into()),
		gas: PROVIDED_GAS.into(),
		gas_price: U256::default().into(),
		value: U256::default().into(),
		data,
	}.fake_sign(from.into());

	let res = ::extension::state::check_proof(
		state_items,
		 old_header.state_root().into(),
		&tx,
		machine,
		&env_info,
	);

	match res {
		::extension::state::ProvedExecution::BadProof => Err("Bad proof".into()),
		::extension::state::ProvedExecution::Failed(e) => Err(format!("Failed call: {}", e)),
		::extension::state::ProvedExecution::Complete(e) => {
                      decoder.decode(&e.output).map_err(|e| e.to_string()).map(|h| h.iter().map(|h| h.into()).collect())
                },
	}
*/

    Err("Bad proof".into())
}

fn decode_first_proof(rlp: &Rlp) -> Result<(OrdinaryHeader, Vec<DBValue>), PoaError> {
	let header = rlp.val_at(0)?;
	let state_items = rlp.at(1)?.iter().map(|x| {
		let mut val = DBValue::new();
		val.append_slice(x.data()?);
		Ok(val)
	}).collect::<Result<_, PoaError>>()?;

	Ok((header, state_items))
}

// inter-contract proofs are a header and receipts.
// checking will involve ensuring that the receipts match the header and
// extracting the validator set from the receipts.
fn encode_proof(header: &OrdinaryHeader, receipts: &[Receipt]) -> Bytes {
	let mut stream = RlpStream::new_list(2);
	stream.append(header).append_list(receipts);
	stream.drain()
}

fn decode_proof(rlp: &Rlp) -> Result<(OrdinaryHeader, Vec<Receipt>), PoaError> {
	Ok((rlp.val_at(0)?, rlp.list_at(1)?))
}

// given a provider and caller, generate proof. this will just be a state proof
// of `getValidators`.
fn prove_initial(_contract_address: Address, _header: &OrdinaryHeader, _caller: &Call) -> Result<Vec<u8>, String> {
	use std::cell::RefCell;

	let epoch_proof = RefCell::new(None);
/*
	let validators = {
		let (data, decoder) = validator_set::functions::get_validators::call();
		let (value, proof) = caller(contract_address, data)?;
		*epoch_proof.borrow_mut() = Some(encode_first_proof(header, &proof));
		decoder.decode(&value).map_err(|e| e.to_string())?
	};
*/

	let proof = epoch_proof.into_inner().expect("epoch_proof always set after call; qed");

/*
	trace!(target: "engine", "obtained proof for initial set: {} validators, {} bytes",
		validators.len(), proof.len());

	info!(target: "engine", "Signal for switch to contract-based validator set.");
	info!(target: "engine", "Initial contract validators: {:?}", validators);
*/

	Ok(proof)
}

impl ValidatorSafeContract {
	pub fn new(contract_address: Address) -> Self {
		ValidatorSafeContract {
			contract_address,
			validators: RwLock::new(MemoryLruCache::new(MEMOIZE_CAPACITY)),
			client: RwLock::new(None),
		}
	}

	/// Queries the state and gets the set of validators.
	fn get_list(&self, _caller: &Call) -> Option<SimpleList> {
		let _contract_address = self.contract_address.clone();

//		let (data, decoder) = validator_set::functions::get_validators::call();
//		let value = caller(contract_address, data).and_then(|x| decoder.decode(&x.0).map_err(|e| e.to_string()));
                let value = Vec::new();
//                value.push(Address::from(0));
//                value.push(Address::from(1));
                let value : Result<Vec<PublicKey>, String> = Ok(value);

		match value {
			Ok(new) => {
				debug!(target: "engine", "Set of validators obtained: {:?}", new);
				Some(SimpleList::new(new))//.iter().map(|h| h.into()).collect()))
			},
			Err(s) => {
				debug!(target: "engine", "Set of validators could not be updated: {}", s);
				None
			},
		}
	}

	// Whether the header matches the expected bloom.
	//
	// The expected log should have 3 topics:
	//   1. ETHABI-encoded log name.
	//   2. the block's parent hash.
	//   3. the "nonce": n for the nth transition in history.
	//
	// We can only search for the first 2, since we don't have the third
	// just yet.
	//
	// The parent hash is included to prevent
	// malicious actors from brute forcing other logs that would
	// produce the same bloom.
	//
	// The log data is an array of all new validator addresses.
/*
	fn expected_bloom(&self, header: &OrdinaryHeader) -> Bloom {
		let topics = vec![EVENT_NAME_HASH.clone(), header.parent_hash().clone()];

		debug!(target: "engine", "Expected topics for header {:?}: {:?}",
			header.hash(), topics);

		LogEntry {
			address: self.contract_address.clone().into(),
			topics: topics,
			data: Vec::new(), // irrelevant for bloom.
		}.bloom().into()
	}
*/

	// check receipts for log event. bloom should be `expected_bloom` for the
	// header the receipts correspond to.
	fn extract_from_event(&self, /*bloom: Bloom,*/ _header: &OrdinaryHeader, _receipts: &[Receipt]) -> Option<SimpleList> {
/*
		let check_log = |log: &LogEntry| {
			log.address == self.contract_address.clone().into() &&
				log.topics.len() == 2 &&
				log.topics[0] == EVENT_NAME_HASH.clone().into() &&
				log.topics[1] == *header.parent_hash()
		};

		//// iterate in reverse because only the _last_ change in a given
		//// block actually has any effect.
		//// the contract should only increment the nonce once.
                let bloom: ethereum_types::Bloom = bloom.into();
		let mut decoded_events = receipts.iter()
			.rev()
			.filter(|r| r.log_bloom.contains_bloom(&bloom))
			.flat_map(|r| r.logs.iter())
			.filter(move |l| check_log(l))
			.filter_map(|log| {
                            validator_set::events::initiate_change::parse_log((log.topics.clone(), log.data.clone()).into()).ok()
			});

		// only last log is taken into account
		match decoded_events.next() {
			None => None,
			Some(matched_event) => Some(SimpleList::new(matched_event.new_set.iter().map(|h| h.into()).collect()))
		}
*/
            None
	}
}

impl ValidatorSet for ValidatorSafeContract {
	fn default_caller(&self, id: BlockId) -> Box<Call> {
		let client = self.client.read().clone();
		Box::new(move |addr, data| client.as_ref()
			.and_then(Weak::upgrade)
			.ok_or_else(|| "No client!".into())
			.and_then(|c| {
				match c.as_full_client() {
					Some(c) => c.call_contract(id, addr.into(), data),
					None => Err("No full client!".into()),
				}
			})
			.map(|out| (out, Vec::new()))) // generate no proofs in general
	}

	fn on_epoch_begin(&self, _first: bool, _header: &OrdinaryHeader, _caller: &mut SystemCall) -> Result<(), PoaError> {
/*
		let data = validator_set::functions::finalize_change::encode_input();
		caller(self.contract_address.clone(), data)
			.map(|_| ())
			.map_err(::extension::engines::EngineError::FailedSystemCall)
			.map_err(Into::into)
*/
            Ok(())
	}

	fn genesis_epoch_data(&self, header: &OrdinaryHeader, call: &Call) -> Result<Vec<u8>, String> {
		prove_initial(self.contract_address.clone(), header, call)
	}

	fn is_epoch_end(&self, _first: bool, _chain_head: &OrdinaryHeader) -> Option<Vec<u8>> {
		None // no immediate transitions to contract.
	}

	fn signals_epoch_end(&self, first: bool, header: &OrdinaryHeader, aux: AuxiliaryData)
		-> EpochChange<EthereumMachine>
	{
		let receipts = aux.receipts;

		// transition to the first block of a contract requires finality but has no log event.
		if first {
			debug!(target: "engine", "signalling transition to fresh contract.");
			let state_proof = Arc::new(StateProof {
				contract_address: self.contract_address.clone(),
				header: header.clone(),
			});
			return EpochChange::Yes(Proof::WithState(state_proof as Arc<_>));
		}

		// otherwise, we're checking for logs.
//		let bloom = self.expected_bloom(header);
//		let header_bloom = header.log_bloom();
//		if &bloom & header_bloom != bloom { return ::extension::engines::EpochChange::No }

		trace!(target: "engine", "detected epoch change event bloom");

		match receipts {
			None => EpochChange::Unsure(AuxiliaryRequest::Receipts),
			Some(receipts) => match self.extract_from_event(/*bloom,*/ header, receipts) {
				None => EpochChange::No,
				Some(list) => {
					info!(target: "engine", "Signal for transition within contract. New list: {:?}",
						&*list);

					let proof = encode_proof(&header, receipts);
					EpochChange::Yes(Proof::Known(proof))
				}
			},
		}
	}

	fn epoch_set(&self, first: bool, machine: &EthereumMachine, _number: BlockNumber, proof: &[u8])
		-> Result<(SimpleList, Option<H256>), PoaError>
	{
		let rlp = Rlp::new(proof);

		if first {
			trace!(target: "engine", "Recovering initial epoch set");

			let (old_header, state_items) = decode_first_proof(&rlp)?;
			let number = old_header.number();
			let old_hash = old_header.hash();
			let addresses = check_first_proof(machine, self.contract_address.clone(), old_header, &state_items)
				.map_err(EngineError::InsufficientProof)?;

			trace!(target: "engine", "extracted epoch set at #{}: {} addresses",
				number, addresses.len());

			Ok((SimpleList::new(addresses), Some(old_hash)))
		} else {
			let (old_header, receipts) = decode_proof(&rlp)?;

			// ensure receipts match header.
			// TODO: optimize? these were just decoded.
/*
			let found_root = ::triehash::ordered_trie_root(
				receipts.iter().map(::rlp::encode)
			).into();
			if &found_root != old_header.receipts_root() {
				return Err(BlockError::InvalidReceiptsRoot(
					Mismatch { expected: old_header.receipts_root().clone(), found: found_root }
				).into());
			}
*/

//			let bloom = self.expected_bloom(&old_header);

			match self.extract_from_event(/*bloom,*/ &old_header, &receipts) {
				Some(list) => Ok((list, Some(old_header.hash()))),
				None => Err(EngineError::InsufficientProof("No log event in proof.".into()).into()),
			}
		}
	}

	fn contains_with_caller(&self, block_hash: &H256, key_id: &u64, caller: &Call) -> bool {
		let mut guard = self.validators.write();
		let maybe_existing = guard
			.get_mut(block_hash)
			.map(|list| list.contains(block_hash, key_id));
		maybe_existing
			.unwrap_or_else(|| self
				.get_list(caller)
				.map_or(false, |list| {
					let contains = list.contains(block_hash, key_id);
					guard.insert(block_hash.clone(), list);
					contains
				 }))
	}

	fn get_with_caller(&self, block_hash: &H256, nonce: usize, caller: &Call) -> PublicKey {
		let mut guard = self.validators.write();
		let maybe_existing = guard
			.get_mut(block_hash)
			.map(|list| list.get(block_hash, nonce));
		maybe_existing
			.unwrap_or_else(|| self
				.get_list(caller)
				.map_or_else(Default::default, |list| {
					let address = list.get(block_hash, nonce);
					guard.insert(block_hash.clone(), list);
					address
				 }))
	}

	fn count_with_caller(&self, block_hash: &H256, caller: &Call) -> usize {
		let mut guard = self.validators.write();
		let maybe_existing = guard
			.get_mut(block_hash)
			.map(|list| list.count(block_hash));
		maybe_existing
			.unwrap_or_else(|| self
				.get_list(caller)
				.map_or_else(usize::max_value, |list| {
					let address = list.count(block_hash);
					guard.insert(block_hash.clone(), list);
					address
				 }))
	}

	fn register_client(&self, client: Weak<dyn EngineClient>) {
		trace!(target: "engine", "Setting up contract caller.");
		*self.client.write() = Some(client);
	}
}

#[cfg(test)]
mod tests {

    use ed25519_dalek::PublicKey;
    use hash::keccak;
    use rustc_hex::FromHex;
    use std::fs;
    use std::path::Path;
    use std::convert::From;
    use std::str::FromStr;
    use std::sync::Arc;
    use test_helpers::{
        generate_dummy_client_with_spec_and_accounts_ba, generate_dummy_client_with_spec_and_data_ba
    };
    use super::super::{ValidatorSet};
    use super::{ValidatorSafeContract};
    use ton_block::id_from_key;

    use engines::{
        BasicAuthority, BasicAuthorityParams, EpochChange, Proof
    };
    use engines::authority_round::subst::{
        AccountProvider, Action, Address, AuxiliaryRequest, EngineClient, EthereumMachine, H256, 
        OrdinaryHeader, Secret, Transaction, ValidatorSpec
    };

    fn new_validator_safe_contract() -> BasicAuthority {
        BasicAuthority::new(
            BasicAuthorityParams {
                validators: ValidatorSpec::SafeContract(
                    Address::from_str("0x0000000000000000000000000000000000000005").unwrap()
                )
            },
            EthereumMachine::new()
        )
    }

    #[ignore]
    #[test]
    fn fetches_validators() {
        let client = generate_dummy_client_with_spec_and_accounts_ba(new_validator_safe_contract, None);
        let vc = Arc::new(ValidatorSafeContract::new("0000000000000000000000000000000000000005".parse::<Address>().unwrap().into()));
        vc.register_client(Arc::downgrade(&client) as _);
        let last_hash = H256::from(0);//client.best_block_header().hash();
        let dat1 = fs::read(Path::new("../config/pub01")).expect("Error reading key file ../config/pub01");
        let pub1 = PublicKey::from_bytes(&dat1).unwrap();
        let kid1 = id_from_key(&pub1);
        let dat2 = fs::read(Path::new("../config/pub02")).expect("Error reading key file ../config/pub02");
        let pub2 = PublicKey::from_bytes(&dat2).unwrap();
        let kid2 = id_from_key(&pub2);
        assert!(vc.contains(&last_hash.into(), &kid1));
        assert!(vc.contains(&last_hash.into(), &kid2));
    }

        #[ignore]
	#[test]
	fn knows_validators() {
		let tap = Arc::new(AccountProvider::transient_provider());
		let s0: Secret = keccak("1").into();
		let _v0 = tap.insert_account(s0.clone(), &"".into()).unwrap();
		let _v1 = tap.insert_account(keccak("0").into(), &"".into()).unwrap();
		let chain_id = 0;//new_validator_safe_contract().chain_id();
		let client = generate_dummy_client_with_spec_and_accounts_ba(new_validator_safe_contract, Some(tap));
		client.engine().register_client(Arc::downgrade(&client) as _);
		let validator_contract = "0000000000000000000000000000000000000005".parse::<Address>().unwrap();

//		client.miner().set_author(v1, Some("".into())).unwrap();
		// Remove "1" validator.
		let _tx = Transaction {
			nonce: 0u64.into(),
			gas_price: 0u64.into(),
			gas: 500_000u64.into(),
			action: Action::Call(validator_contract.clone()),
			value: 0u64.into(),
			data: "bfc708a000000000000000000000000082a978b3f5962a5b0957d9ee9eef472ee55b42f1".from_hex().unwrap(),
		}.sign(&s0, Some(chain_id));
//		client.miner().import_own_transaction(client.as_ref(), tx.into()).unwrap();
		EngineClient::update_sealing(&*client);
//		assert_eq!(client.chain_info().best_block_number, 1);
		// Add "1" validator back in.
		let _tx = Transaction {
			nonce: 1u64.into(),
			gas_price: 0u64.into(),
			gas: 500_000u64.into(),
			action: Action::Call(validator_contract.clone()),
			value: 0u64.into(),
			data: "4d238c8e00000000000000000000000082a978b3f5962a5b0957d9ee9eef472ee55b42f1".from_hex().unwrap(),
		}.sign(&s0, Some(chain_id));
//		client.miner().import_own_transaction(client.as_ref(), tx.into()).unwrap();
		EngineClient::update_sealing(&*client);
		// The transaction is not yet included so still unable to seal.
//		assert_eq!(client.chain_info().best_block_number, 1);

		// Switch to the validator that is still there.
//		client.miner().set_author(v0, Some("".into())).unwrap();
		EngineClient::update_sealing(&*client);
//		assert_eq!(client.chain_info().best_block_number, 2);
		// Switch back to the added validator, since the state is updated.
//		client.miner().set_author(v1, Some("".into())).unwrap();
		let _tx = Transaction {
			nonce: 2u64.into(),
			gas_price: 0u64.into(),
			gas: 21000u64.into(),
			action: Action::Call(Address::default().into()),
			value: 0u64.into(),
			data: Vec::new(),
		}.sign(&s0, Some(chain_id));
//		client.miner().import_own_transaction(client.as_ref(), tx.into()).unwrap();
		EngineClient::update_sealing(&*client);
		// Able to seal again.
//		assert_eq!(client.chain_info().best_block_number, 3);

		// Check syncing.
		let sync_client = generate_dummy_client_with_spec_and_data_ba(new_validator_safe_contract, 0, 0, &[]);
		sync_client.engine().register_client(Arc::downgrade(&sync_client) as _);
//		for i in 1..4 {
//			sync_client.import_block(Unverified::from_rlp(client.block(BlockId::Number(i)).unwrap().into_inner()).unwrap()).unwrap();
//		}
//		sync_client.flush_queue();
//		assert_eq!(sync_client.chain_info().best_block_number, 3);
	}

        #[ignore]
	#[test]
	fn detects_bloom() {

		let client = generate_dummy_client_with_spec_and_accounts_ba(new_validator_safe_contract, None);
		let engine = client.engine().clone();
		let _validator_contract = "0000000000000000000000000000000000000005".parse::<Address>().unwrap();

		let last_hash = H256::from(0u8);//client.best_block_header().hash();
		let mut new_header = OrdinaryHeader::default();
		new_header.set_parent_hash(last_hash.into());
		new_header.set_number(1); // so the validator set looks for a log.

		// first, try without the parent hash.
/*
		let mut event = LogEntry {
			address: validator_contract,
			topics: vec![EVENT_NAME_HASH.clone().into()],
			data: Vec::new(),
		};
*/

//		new_header.set_log_bloom(event.bloom().into());
		match engine.signals_epoch_end(&new_header.clone().into(), Default::default()) {
			EpochChange::No => {},
			_ => panic!("Expected bloom to be unrecognized."),
		};

		// with the last hash, it should need the receipts.
//		event.topics.push(last_hash);
//		new_header.set_log_bloom(event.bloom().clone().into());

		match engine.signals_epoch_end(&new_header.clone().into(), Default::default()) {
			EpochChange::Unsure(AuxiliaryRequest::Receipts) => {},
			_ => panic!("Expected bloom to be recognized."),
		};
	}

        #[ignore]
	#[test]
	fn initial_contract_is_signal() {

		let client = generate_dummy_client_with_spec_and_accounts_ba(new_validator_safe_contract, None);
		let engine = client.engine().clone();

		let mut new_header = OrdinaryHeader::default();
		new_header.set_number(0); // so the validator set doesn't look for a log

		match engine.signals_epoch_end(&new_header, Default::default()) {
			EpochChange::Yes(Proof::WithState(_)) => {},
			_ => panic!("Expected state to be required to prove initial signal"),
		};
	}
}
