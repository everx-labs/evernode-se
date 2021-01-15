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

//! A blockchain engine that supports a basic, non-BFT proof-of-authority.

use std::sync::{Weak, Arc};
use parking_lot::RwLock;

use engines::{Engine, Seal, ConstructedVerifier};
use engines::signer::EngineSigner;
use engines::validator_set::{ValidatorSet, SimpleList, new_validator_set};
use error::{BlockError, PoaError};

use engines::authority_round::subst::{
    AccountProvider, Address, AuxiliaryData, Call, EngineClient, EthereumMachine, 
    ExecutedBlock, ExtendedHeader, H256, H520, LiveBlock, OrdinaryHeader, Password, Signature, 
    TONBlock, ValidatorSpec
};

/// `BasicAuthority` params.
#[derive(Debug, PartialEq)]
pub struct BasicAuthorityParams {
	/// Valid signatories.
	pub validators: ValidatorSpec,
}

/*
impl From<ethjson::spec::BasicAuthorityParams> for BasicAuthorityParams {
	fn from(p: ethjson::spec::BasicAuthorityParams) -> Self {
		BasicAuthorityParams {
			validators: p.validators,
		}
	}
}
*/

struct EpochVerifier {
	list: SimpleList,
}

impl super::EpochVerifier<EthereumMachine> for EpochVerifier {
	fn verify_light(&self, header: &OrdinaryHeader) -> Result<(), PoaError> {
		verify_external(header, &self.list)
	}
}

fn verify_external(header: &OrdinaryHeader, validators: &dyn ValidatorSet) -> Result<(), PoaError> {
	use rlp::Rlp;

	// Check if the signature belongs to a validator, can depend on parent state.
	let _sig = Rlp::new(&header.seal()[0]).as_val::<H520>()?;
        let signer = match &header.ton_block {
            TONBlock::Signed(x) => x.signatures().keys().last(),
            TONBlock::Unsigned(_) => return Err(BlockError::InvalidSeal.into())
        };
//	let signer = ethkey::public_to_address(&ethkey::recover(&sig.into(), &header.bare_hash().into())?);
//	if *header.author() != signer {
//		return Err(EngineError::NotAuthorized(header.author().clone()).into())
//	}

	if let Some(signer) = signer {
		if validators.contains(header.parent_hash(), &signer) {
			return Ok(())
		}
	}
	Err(BlockError::InvalidSeal.into())
}

/// Engine using `BasicAuthority`, trivial proof-of-authority consensus.
pub struct BasicAuthority {
	machine: EthereumMachine,                              	
	signer: RwLock<EngineSigner>,
	validators: Box<dyn ValidatorSet>,
}

impl std::fmt::Debug for BasicAuthority {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> Result<(), std::fmt::Error> {
        write!(f, "machine: {:?}, signer: {:?} \n", self.machine, self.signer)
    }
}

impl BasicAuthority {
	/// Create a new instance of BasicAuthority engine
	pub fn new(our_params: BasicAuthorityParams, machine: EthereumMachine) -> Self {
		BasicAuthority {
			machine: machine,
			signer: Default::default(),
			validators: new_validator_set(our_params.validators),
		}
	}
}

impl Engine<EthereumMachine> for BasicAuthority {
	fn name(&self) -> &str { "BasicAuthority" }

	fn machine(&self) -> &EthereumMachine { &self.machine }

	// One field - the signature
	fn seal_fields(&self, _header: &OrdinaryHeader) -> usize { 1 }

	fn seals_internally(&self) -> Option<bool> {
		Some(self.signer.read().is_some())
	}

	/// Attempt to seal the block internally.
	fn generate_seal(&self, block: &ExecutedBlock, _parent: &OrdinaryHeader) -> (Seal, Vec<u128>) {
		let header = block.header();
		let author = header.author_key_id();
		if self.validators.contains(header.parent_hash(), author) {
			// account should be pernamently unlocked, otherwise sealing will fail
			if let Ok(signature) = self.sign(header.bare_hash()) {
				return (
                                    Seal::Regular(vec![::rlp::encode(&H520::from(signature).as_ref())]),
                                    Vec::new()
                                ); //&(&H520::from(signature) as &[u8]))]);
			} else {
				trace!(target: "basicauthority", "generate_seal: FAIL: accounts secret key unavailable");
			}
		}
		(Seal::None, Vec::new())
	}

	fn verify_local_seal(&self, _header: &OrdinaryHeader) -> Result<(), PoaError> {
		Ok(())
	}

	fn verify_block_external(&self, header: &OrdinaryHeader) -> Result<(), PoaError> {
            verify_external(header, &*self.validators)
	}

	fn genesis_epoch_data(&self, header: &OrdinaryHeader, call: &Call) -> Result<Vec<u8>, String> {
		self.validators.genesis_epoch_data(header, call)
	}

	#[cfg(not(test))]
	fn signals_epoch_end(&self, _header: &OrdinaryHeader, _auxiliary: AuxiliaryData)
		-> super::EpochChange<EthereumMachine>
	{
		// don't bother signalling even though a contract might try.
		super::EpochChange::No
	}

	#[cfg(test)]
	fn signals_epoch_end(&self, header: &OrdinaryHeader, auxiliary: AuxiliaryData)
		-> super::EpochChange<EthereumMachine>
	{
		// in test mode, always signal even though they don't be finalized.
		let first = header.number() == 0;
		self.validators.signals_epoch_end(first, header, auxiliary)
	}

	fn is_epoch_end(
		&self,
		chain_head: &OrdinaryHeader,
		_finalized: &[H256],
		_chain: &super::Headers<OrdinaryHeader>,
		_transition_store: &super::PendingTransitionStore,
	) -> Option<Vec<u8>> {
		let first = chain_head.number() == 0;

		// finality never occurs so only apply immediate transitions.
		self.validators.is_epoch_end(first, chain_head)
	}

	fn is_epoch_end_light(
		&self,
		chain_head: &OrdinaryHeader,
		chain: &super::Headers<OrdinaryHeader>,
		transition_store: &super::PendingTransitionStore,
	) -> Option<Vec<u8>> {
		self.is_epoch_end(chain_head, &[], chain, transition_store)
	}

	fn epoch_verifier<'a>(&self, header: &OrdinaryHeader, proof: &'a [u8]) -> ConstructedVerifier<'a, EthereumMachine> {
		let first = header.number() == 0;

		match self.validators.epoch_set(first, &self.machine, header.number(), proof) {
			Ok((list, finalize)) => {
				let verifier = Box::new(EpochVerifier { list: list });

				// our epoch verifier will ensure no unverified verifier is ever verified.
				match finalize {
					Some(finalize) => ConstructedVerifier::Unconfirmed(verifier, proof, finalize),
					None => ConstructedVerifier::Trusted(verifier),
				}
			}
			Err(e) => ConstructedVerifier::Err(e),
		}
	}

	fn register_client(&self, client: Weak<dyn EngineClient>) {
		self.validators.register_client(client);
	}

	fn set_signer(&self, ap: Arc<AccountProvider>, address: Address, password: Password) {
		self.signer.write().set(ap, address, password);
	}

	fn sign(&self, hash: H256) -> Result<Signature, PoaError> {
		Ok(self.signer.read().sign(hash)?)
	}

/*
	fn snapshot_components(&self) -> Option<Box<::extension::snapshot::SnapshotComponents>> {
		None
	}
*/

	fn fork_choice(&self, new: &ExtendedHeader, current: &ExtendedHeader) -> super::ForkChoice {
		super::total_difficulty_fork_choice(new, current)
	}
}

#[cfg(test)]
mod tests {

    use ed25519_dalek::PublicKey;
	use hash::keccak;
        use std::fs;
        use std::path::Path;
        use std::str::FromStr;
	use std::sync::Arc;
//	use tempdir::TempDir;
	use test_helpers::get_temp_state_db;

	use engines::{Engine, Seal};    
        use engines::authority_round::subst::{
            AccountProvider, EthereumMachine, H256, H520, IsBlock, OpenBlock, OrdinaryHeader, 
            ValidatorSpec, U256
        };

	/// Create a new test chain spec with `BasicAuthority` consensus engine.
	fn new_test_authority() -> super::BasicAuthority {
            let dat1 = fs::read(Path::new("../config/pub01")).expect("Error reading key file ./config/pub01");
            let pub1 = PublicKey::from_bytes(&dat1).unwrap();
//		let bytes: &[u8] = include_bytes!("../res/basic_authority.json");
//		let tempdir = TempDir::new("").unwrap();
//		Spec::load(&tempdir.path(), bytes).expect("invalid chain spec")
            super::BasicAuthority::new(
                super::BasicAuthorityParams {
                    validators: ValidatorSpec::List(vec![pub1])
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
        header.set_gas_limit(U256::from(0x2fefd8u64));
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
	fn has_valid_metadata() {
		let engine = new_test_authority();
		assert!(!engine.name().is_empty());
	}

/*
	#[test]
	fn can_return_schedule() {
		let engine = new_test_authority();
		let schedule = engine.schedule(10000000);
		assert!(schedule.stack_limit > 0);
	}
*/

        #[ignore]
	#[test]
	fn can_do_signature_verification_fail() {
		let engine = new_test_authority();
		let mut header: OrdinaryHeader = OrdinaryHeader::default();
		header.set_seal(vec![::rlp::encode(&H520::default())]);

		let verify_result = engine.verify_block_external(&header);
		assert!(verify_result.is_err());
	}

        #[ignore]
	#[test]
	fn can_generate_seal() {
		let tap = AccountProvider::transient_provider();
		let addr = tap.insert_account(keccak("").into(), &"".into()).unwrap();

		let engine = new_test_authority();
		engine.set_signer(Arc::new(tap), addr.clone().into(), "".into());
		let genesis_header = genesis_header();
		let db = get_temp_state_db();//spec.ensure_db_good(get_temp_state_db(), &Default::default()).unwrap();
		let last_hashes = Arc::new(vec![genesis_header.hash()]);
		let b = OpenBlock::new(&engine, /* Default::default(),*/ false, db, &genesis_header, 
                    last_hashes, addr.into(), (3141562u64.into(), 31415620u64.into()), 
                    vec![], false, &mut Vec::new().into_iter()).unwrap();
		let b = b.close_and_lock().unwrap();
		if let (Seal::Regular(seal), _) = engine.generate_seal(b.block(), &genesis_header.into()) {
			assert!(b.try_seal(&engine, seal).is_ok());
		}
	}

        #[ignore]
	#[test]
	fn seals_internally() {
		let tap = AccountProvider::transient_provider();
		let authority = tap.insert_account(keccak("").into(), &"".into()).unwrap();
		let engine = new_test_authority();
		assert!(!engine.seals_internally().unwrap());
		engine.set_signer(Arc::new(tap), authority.into(), "".into());
		assert!(engine.seals_internally().unwrap());
	}
}
