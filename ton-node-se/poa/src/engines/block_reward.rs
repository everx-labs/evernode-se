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

//! A module with types for declaring block rewards and a client interface for interacting with a
//! block reward contract.

use hash::keccak;
use std::sync::Arc;
use engines::{SystemOrCodeCall, SystemOrCodeCallKind};
use error::PoaError;

//use_contract!(block_reward_contract, "src/extension/res/contracts/block_reward.json");

use engines::authority_round::subst::{
    Address, BlockNumber, Machine, U256, WithBalances, WithRewards
};

/// The kind of block reward.
/// Depending on the consensus engine the allocated block reward might have
/// different semantics which could lead e.g. to different reward values.
#[derive(PartialEq, Eq, Clone, Copy, Debug)]
pub enum RewardKind {
	/// Reward attributed to the block author.
	Author,
	/// Reward attributed to the author(s) of empty step(s) included in the block (AuthorityRound engine).
	EmptyStep,
	/// Reward attributed by an external protocol (e.g. block reward contract).
	External,
	/// Reward attributed to the block uncle(s) with given difference.
	Uncle(u8),
}

impl RewardKind {
	/// Create `RewardKind::Uncle` from given current block number and uncle block number.
	pub fn uncle(number: BlockNumber, uncle: BlockNumber) -> Self {
		RewardKind::Uncle(if number > uncle && number - uncle <= u8::max_value().into() { (number - uncle) as u8 } else { 0 })
	}
}

impl From<RewardKind> for u16 {
	fn from(reward_kind: RewardKind) -> Self {
		match reward_kind {
			RewardKind::Author => 0,
			RewardKind::EmptyStep => 2,
			RewardKind::External => 3,

			RewardKind::Uncle(depth) => 100 + depth as u16,
		}
	}
}

/*
impl Into<trace::RewardType> for RewardKind {
	fn into(self) -> trace::RewardType {
		match self {
			RewardKind::Author => trace::RewardType::Block,
			RewardKind::Uncle(_) => trace::RewardType::Uncle,
			RewardKind::EmptyStep => trace::RewardType::EmptyStep,
			RewardKind::External => trace::RewardType::External,
		}
	}
}
*/

/// A client for the block reward contract.
#[derive(PartialEq, Debug)]
pub struct BlockRewardContract {
	kind: SystemOrCodeCallKind,
}

impl BlockRewardContract {
	/// Create a new block reward contract client targeting the system call kind.
	pub fn new(kind: SystemOrCodeCallKind) -> BlockRewardContract {
		BlockRewardContract {
			kind,
		}
	}

	/// Create a new block reward contract client targeting the contract address.
	pub fn new_from_address(address: Address) -> BlockRewardContract {
		Self::new(SystemOrCodeCallKind::Address(address))
	}

	/// Create a new block reward contract client targeting the given code.
	pub fn new_from_code(code: Arc<Vec<u8>>) -> BlockRewardContract {
		let code_hash = keccak(&code[..]).into();

		Self::new(SystemOrCodeCallKind::Code(code, code_hash))
	}

	/// Calls the block reward contract with the given beneficiaries list (and associated reward kind)
	/// and returns the reward allocation (address - value). The block reward contract *must* be
	/// called by the system address so the `caller` must ensure that (e.g. using
	/// `machine.execute_as_system`).
	pub fn reward(
		&self,
		beneficiaries: &[(Address, RewardKind)],
		_caller: &mut SystemOrCodeCall,
	) -> Result<Vec<(Address, U256)>, PoaError> {
/*
		let input = block_reward_contract::functions::reward::encode_input(
			beneficiaries.iter().map(|&(ref address, _)| H160::from(address.clone())),
			beneficiaries.iter().map(|&(_, ref reward_kind)| u16::from(*reward_kind)),
		);

		let output = caller(self.kind.clone(), input)
			.map_err(Into::into)
			.map_err(::extension::engines::EngineError::FailedSystemCall)?;

		// since this is a non-constant call we can't use ethabi's function output
		// deserialization, sadness ensues.
		let types = &[
			ParamType::Array(Box::new(ParamType::Address)),
			ParamType::Array(Box::new(ParamType::Uint(256))),
		];

		let tokens = ethabi::decode(types, &output)
			.map_err(|err| err.to_string())
			.map_err(::extension::engines::EngineError::FailedSystemCall)?;

		assert!(tokens.len() == 2);

		let addresses = tokens[0].clone().to_array().expect("type checked by ethabi::decode; qed");
		let rewards = tokens[1].clone().to_array().expect("type checked by ethabi::decode; qed");

		if addresses.len() != rewards.len() {
			return Err(::extension::engines::EngineError::FailedSystemCall(
				"invalid data returned by reward contract: both arrays must have the same size".into()
			).into());
		}

		let addresses = addresses.into_iter().map(|t| t.to_address().expect("type checked by ethabi::decode; qed"));
		let rewards = rewards.into_iter().map(|t| t.to_uint().expect("type checked by ethabi::decode; qed"));

		Ok(addresses.zip(rewards).map(|(h, u)| (h.into(), u.into())).collect())
*/
            let mut ret = Vec::new();
            for b in beneficiaries {
                ret.push((b.0.clone(), U256::from(0u64)))
            }
            Ok(ret)
	}
}

/// Applies the given block rewards, i.e. adds the given balance to each beneficiary' address.
/// If tracing is enabled the operations are recorded.
pub fn apply_block_rewards<M: Machine + WithBalances + WithRewards>(
	rewards: &[(Address, RewardKind, U256)],
	block: &mut M::LiveBlock,
	machine: &M,
) -> Result<(), M::Error> {
	for &(ref author, _, ref block_reward) in rewards {
		machine.add_balance(block, author, block_reward)?;
	}

	let rewards: Vec<_> = rewards.into_iter().map(|&(ref a, k, ref r)| (a.clone(), k.into(), r.clone())).collect();
	machine.note_rewards(block, &rewards)
}

/*
#[cfg(test)]
mod test {

    use ed25519_dalek::{Keypair, KEYPAIR_LENGTH, PublicKey};
    use std::fs;
    use std::path::Path;
        use std::str::FromStr;
        use std::sync::Arc;

	use super::{BlockRewardContract, RewardKind};
	use test_helpers::generate_dummy_client_with_spec_and_accounts_ar;

        use engines::authority_round::{AuthorityRound, AuthorityRoundParams};
	use engines::validator_set::{new_validator_set};
        use engines::authority_round::subst::{
            Address, EthereumMachine, H256, U256, ValidatorSpec
        };

    fn new_test_machine() -> EthereumMachine {
        EthereumMachine::new()
    }

    fn new_test_round_block_reward_contract() -> Arc<AuthorityRound> {
        let dat1 = fs::read(Path::new("./pub01")).expect("Error reading key file ./pub01");
        let pub1 = PublicKey::from_bytes(&dat1).unwrap();
        let dat2 = fs::read(Path::new("./pub02")).expect("Error reading key file ./pub02");
        let pub2 = PublicKey::from_bytes(&dat2).unwrap();
        AuthorityRound::new(
            AuthorityRoundParams {
                step_duration: 1,
                start_step: Some(2),
                validators: new_validator_set(
                    ValidatorSpec::List(vec![pub1, pub2])
                ),
                validate_score_transition: 0,
                validate_step_transition: 0,
                immediate_transitions: true,
                block_reward: U256::from(10u64),
                block_reward_contract_transition: 0,
                block_reward_contract: Some(BlockRewardContract::new_from_address(
                    Address::from_str("0x0000000000000000000000000000000000000042").unwrap()
                )),
                maximum_uncle_count_transition: 0,
                maximum_uncle_count: 0,
                empty_steps_transition: 1,
                maximum_empty_steps: 2,
                ton_key: Keypair::from_bytes(&[0; KEYPAIR_LENGTH]).unwrap()
            },
            EthereumMachine::new()
        ).unwrap()
    }

        #[ignore]
	#[test]
	fn block_reward_contract() {
		let _client = generate_dummy_client_with_spec_and_accounts_ar(
			new_test_round_block_reward_contract,
			None,
		);

		let _machine = new_test_machine();

		// the spec has a block reward contract defined at the given address
		let block_reward_contract = BlockRewardContract::new_from_address(
			Address::from_str("0000000000000000000000000000000000000042").unwrap().into(),
		);

		let mut call = |_to, _data| {
			let mut block = client.prepare_open_block(
				"0000000000000000000000000000000000000001".into(),
				(3141562.into(), 31415620.into()),
				vec![],
			).unwrap();

			let result = match to {
				SystemOrCodeCallKind::Address(to) => {
					machine.execute_as_system(
						block.block_mut(),
						to.into(),
						U256::max_value().into(),
						Some(data),
					)
				},
				_ => panic!("Test reward contract is created by an address, we never reach this branch."),
			};

			result.map_err(|e| format!("{}", e))
                    Ok(vec![0u8])
		};

		// if no beneficiaries are given no rewards are attributed
		assert!(block_reward_contract.reward(&vec![], &mut call).unwrap().is_empty());

		// the contract rewards (1000 + kind) for each benefactor
		let beneficiaries = vec![
			(H256::from_str("0000000000000000000000000000000000000033").unwrap().into(), RewardKind::Author),
			(H256::from_str("0000000000000000000000000000000000000034").unwrap().into(), RewardKind::Uncle(1)),
			(H256::from_str("0000000000000000000000000000000000000035").unwrap().into(), RewardKind::EmptyStep),
		];

		let rewards = block_reward_contract.reward(&beneficiaries, &mut call).unwrap();
		let expected = vec![
			(H256::from_str("0000000000000000000000000000000000000033").unwrap().into(), U256::from(1000u64)),
			(H256::from_str("0000000000000000000000000000000000000034").unwrap().into(), U256::from(1101u64)),
			(H256::from_str("0000000000000000000000000000000000000035").unwrap().into(), U256::from(1002u64)),
		];

		assert_eq!(expected, rewards);

	}
}
*/