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
/// It can also report validators for misbehaviour with two levels: `reportMalicious` and `reportBenign`.

use bytes::Bytes;
use ed25519_dalek::{PublicKey};
use parking_lot::RwLock;
use std::sync::Weak;

use engines::{EpochChange};
use engines::validator_set::{ValidatorSet, SimpleList, SystemCall};
use engines::validator_set::safe_contract::ValidatorSafeContract;
use error::PoaError;

use engines::authority_round::subst::{
    Address, AuxiliaryData, BlockId, BlockNumber, Call, EngineClient, EthereumMachine, H256, OrdinaryHeader
};

//use_contract!(validator_report, "src/extension/res/contracts/validator_report.json");

#[allow(dead_code)]
/// A validator contract with reporting.
pub struct ValidatorContract {
	contract_address: Address,
	validators: ValidatorSafeContract,
	client: RwLock<Option<Weak<dyn EngineClient>>>, // TODO [keorn]: remove
}

impl ValidatorContract {
	pub fn new(contract_address: Address) -> Self {
		ValidatorContract {
			contract_address: contract_address.clone(),
			validators: ValidatorSafeContract::new(contract_address.clone()),
			client: RwLock::new(None),
		}
	}
}

impl ValidatorContract {
        #[allow(dead_code)]
	fn transact(&self, data: Bytes) -> Result<(), String> {
		let client = self.client.read().as_ref()
			.and_then(Weak::upgrade)
			.ok_or_else(|| "No client!")?;

		match client.as_full_client() {
			Some(c) => {
				c.transact_contract(self.contract_address.clone().into(), data)
					.map_err(|e| format!("Transaction import error: {}", e))?;
				Ok(())
			},
			None => Err("No full client!".into()),
		}
	}
}

impl ValidatorSet for ValidatorContract {
	fn default_caller(&self, id: BlockId) -> Box<Call> {
		self.validators.default_caller(id)
	}

	fn on_epoch_begin(&self, first: bool, header: &OrdinaryHeader, call: &mut SystemCall) -> Result<(), PoaError> {
		self.validators.on_epoch_begin(first, header, call)
	}

	fn genesis_epoch_data(&self, header: &OrdinaryHeader, call: &Call) -> Result<Vec<u8>, String> {
		self.validators.genesis_epoch_data(header, call)
	}

	fn is_epoch_end(&self, first: bool, chain_head: &OrdinaryHeader) -> Option<Vec<u8>> {
		self.validators.is_epoch_end(first, chain_head)
	}

	fn signals_epoch_end(
		&self,
		first: bool,
		header: &OrdinaryHeader,
		aux: AuxiliaryData,
	) -> EpochChange<EthereumMachine> {
		self.validators.signals_epoch_end(first, header, aux)
	}

	fn epoch_set(&self, first: bool, machine: &EthereumMachine, number: BlockNumber, proof: &[u8]) -> Result<(SimpleList, Option<H256>), PoaError> {
		self.validators.epoch_set(first, machine, number, proof)
	}

	fn contains_with_caller(&self, bh: &H256, key_id: &u64, caller: &Call) -> bool {
		self.validators.contains_with_caller(bh, key_id, caller)
	}

	fn get_with_caller(&self, bh: &H256, nonce: usize, caller: &Call) -> PublicKey {
		self.validators.get_with_caller(bh, nonce, caller)
	}

	fn count_with_caller(&self, bh: &H256, caller: &Call) -> usize {
		self.validators.count_with_caller(bh, caller)
	}

	fn report_malicious(&self, _address: &Address, _set_block: BlockNumber, _block: BlockNumber, _proof: Bytes) {
/*
		let data = validator_report::functions::report_malicious::encode_input(address.clone(), block, proof);
		match self.transact(data) {
			Ok(_) => warn!(target: "engine", "Reported malicious validator {:?}", address),
			Err(s) => warn!(target: "engine", "Validator {:?} could not be reported {}", address, s),
		}
*/
	}

	fn report_benign(&self, _key_id: &u64, _set_block: BlockNumber, _block: BlockNumber) {
/*
		let data = validator_report::functions::report_benign::encode_input(address.clone(), block);
		match self.transact(data) {
			Ok(_) => warn!(target: "engine", "Reported benign validator misbehaviour {:?}", address),
			Err(s) => warn!(target: "engine", "Validator {:?} could not be reported {}", address, s),
		}
*/
	}

	fn register_client(&self, client: Weak<dyn EngineClient>) {
		self.validators.register_client(client.clone());
		*self.client.write() = Some(client);
	}
}

/*
#[cfg(test)]
mod tests {

    use ed25519_dalek::{Keypair, KEYPAIR_LENGTH, PublicKey};
	use hash::keccak;
	use rlp::encode;
    use std::fs;
    use std::path::Path;
        use std::str::FromStr;
	use std::sync::Arc;

	use test_helpers::generate_dummy_client_with_spec_and_accounts_ar;
	use super::super::{ValidatorSet, new_validator_set};
    use ton_block::id_from_key;
	use super::ValidatorContract;

        use engines::{
            AuthorityRound, AuthorityRoundParams
        };
        use engines::authority_round::subst::{
            AccountProvider, Address, EthereumMachine, H256, H520, OrdinaryHeader, U256,
            ValidatorSpec
        };

    fn new_validator_contract() -> Arc<AuthorityRound> {
	AuthorityRound::new(
            AuthorityRoundParams {
                step_duration: 1,
                start_step: Some(2),
                validators: new_validator_set(
                    ValidatorSpec::Contract(
                        Address::from_str("0x0000000000000000000000000000000000000005").unwrap()
                    )
                ),
                validate_score_transition: 0,
                validate_step_transition: 0,
                immediate_transitions: true,
                block_reward: U256::from(0u64),
                block_reward_contract_transition: 0,
                block_reward_contract: None,
                maximum_uncle_count_transition: 0,
                maximum_uncle_count: 0,
                empty_steps_transition: 0,
                maximum_empty_steps: 0,
                ton_key: Keypair::from_bytes(&[0; KEYPAIR_LENGTH]).unwrap()
            },
            EthereumMachine::new()
        ).unwrap()	
    }

    #[ignore]
    #[test]
    fn fetches_validators() {
        let client = generate_dummy_client_with_spec_and_accounts_ar(new_validator_contract, None);
        let vc = Arc::new(ValidatorContract::new("0000000000000000000000000000000000000005".parse::<Address>().unwrap().into()));
        vc.register_client(Arc::downgrade(&client) as _);
        let last_hash = H256::from(0);//client.best_block_header().hash();
        let dat1 = fs::read(Path::new("./pub01")).expect("Error reading key file ./pub01");
        let pub1 = PublicKey::from_bytes(&dat1).unwrap();
        let kid1 = id_from_key(&pub1);
        let dat2 = fs::read(Path::new("./pub02")).expect("Error reading key file ./pub02");
        let pub2 = PublicKey::from_bytes(&dat2).unwrap();
        let kid2 = id_from_key(&pub2);
        assert!(vc.contains(&last_hash.into(), &kid1));
        assert!(vc.contains(&last_hash.into(), &kid2));
    }

        #[ignore]
	#[test]
	fn reports_validators() {

		let tap = Arc::new(AccountProvider::transient_provider());
		let v1 = tap.insert_account(keccak("1").into(), &"".into()).unwrap();
		let client = generate_dummy_client_with_spec_and_accounts_ar(new_validator_contract, Some(tap.clone()));
		client.engine().register_client(Arc::downgrade(&client) as _);
		let _validator_contract = "0000000000000000000000000000000000000005".parse::<Address>().unwrap();

		// Make sure reporting can be done.
//		client.miner().set_gas_range_target((1_000_000.into(), 1_000_000.into()));
//		client.miner().set_author(v1, Some("".into())).unwrap();

		// Check a block that is a bit in future, reject it but don't report the validator.
		let mut header = OrdinaryHeader::default();
		let seal = vec![encode(&4u8), encode(&H520::default().as_ref())];//&(&H520::default() as &[u8]))];
		header.set_seal(seal);
		header.set_author(v1.clone().into());
		header.set_number(2);
//		header.set_parent_hash(client.chain_info().best_block_hash.into());
		assert!(client.engine().verify_block_external(&header).is_err());
		client.engine().step();
//		assert_eq!(client.chain_info().best_block_number, 0);

println!("C");

		// Now create one that is more in future. That one should be rejected and validator should be reported.
		let mut header = OrdinaryHeader::default();
		let seal = vec![encode(&8u8), encode(&H520::default().as_ref())];//&(&H520::default() as &[u8]))];
		header.set_seal(seal);
		header.set_author(v1.clone().into());
		header.set_number(2);
//		header.set_parent_hash(client.chain_info().best_block_hash.into());

println!("C1");
		// `reportBenign` when the designated proposer releases block from the future (bad clock).
		assert!(client.engine().verify_block_basic(&header).is_err());
println!("C1-1");
		// Seal a block.
		client.engine().step();

println!("C2");
//		assert_eq!(client.chain_info().best_block_number, 1);
		// Check if the unresponsive validator is `disliked`.
//		assert_eq!(
//			client.call_contract(BlockId::Latest, validator_contract, "d8f2e0bf".from_hex().unwrap()).unwrap().to_hex(),
//			"0000000000000000000000007d577a597b2742b498cb5cf0c26cdcd726d39e6e"
//		);
println!("C3");

		// Simulate a misbehaving validator by handling a double proposal.
//		let header = client.best_block_header();
//		assert!(client.engine().verify_block_family(&header.clone().into(), &header.clone().into()).is_err());

println!("D");

		// Seal a block.
		client.engine().step();
		client.engine().step();
//		assert_eq!(client.chain_info().best_block_number, 2);

println!("E");

		// Check if misbehaving validator was removed.
//		client.transact_contract(Default::default(), Default::default()).unwrap();
		client.engine().step();
		client.engine().step();
//		assert_eq!(client.chain_info().best_block_number, 2);

println!("F");

	}
}

*/