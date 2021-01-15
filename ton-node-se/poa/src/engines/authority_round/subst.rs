use bytes::Bytes;
use ed25519::signature::Verifier;
use ed25519_dalek::{PublicKey};
use hash::{self, KECCAK_NULL_RLP, KECCAK_EMPTY_LIST_RLP, keccak};
use rand::distributions::Distribution;
use rlp::{Decodable, DecoderError, Encodable, Rlp, RlpStream};
use rustc_hex::{ToHex};
use std::cmp;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::{Debug, Display, Formatter, LowerHex};
use std::io;
use std::ops::{Add, Sub};
use std::path::{Path};
use std::str::FromStr;
use std::sync::Arc;
use ton_block::{Block, GetRepresentationHash, SignedBlock};
use ton_types::{HashmapType, UInt256};

use engines::{EpochTransition, EthEngine};
use engines::block_reward::RewardKind;
use error::{BlockError, PoaError};

/// Transaction action type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
	/// Create creates new contract.
	Create,
	/// Calls contract at given address.
	/// In the case of a transfer, this is the receiver's address.'
	Call(Address),
}

impl Default for Action {
	fn default() -> Action { Action::Create }
}

impl rlp::Decodable for Action {
	fn decode(rlp: &Rlp) -> Result<Self, DecoderError> {
		if rlp.is_empty() {
			Ok(Action::Create)
		} else {
			Ok(Action::Call(rlp.as_val()?))
		}
	}
}

#[derive(Debug, PartialEq, Eq, Clone)]
/// Actions on a live block's parent block. Only committed when the live block is committed. Those actions here must
/// respect the normal blockchain reorganization rules.
pub enum AncestryAction {
	/// Mark an ancestry block as finalized.
	MarkFinalized(H256),
}

/// Transaction value
#[derive(Clone, Debug)]
pub enum ActionValue {
	/// Apparent value for transaction (not transfered)
	Apparent(U256),
	/// Value that should be transfered
	Transfer(U256),
}

/// Uniquely identifies block.
#[derive(Debug, PartialEq, Copy, Clone, Hash, Eq)]
pub enum BlockId {
	/// Block's sha3.
	/// Querying by hash is always faster.
	Hash(H256),
	/// Block number within canon blockchain.
	Number(BlockNumber),
	/// Earliest block (genesis).
	Earliest,
	/// Latest mined block.
	Latest,
}

/// The type of the call-like instruction.
#[derive(Debug, PartialEq, Clone)]
pub enum CallType {
	/// Not a CALL.
	None,
	/// CALL.
	Call,
	/// CALLCODE.
	CallCode,
	/// DELEGATECALL.
	DelegateCall,
	/// STATICCALL
	StaticCall,
}

/*
pub enum BloomInput<'a> { 
 	Raw(&'a [u8]), 
 	Hash(&'a [u8; 32]), 
}
*/ 

/// Simple vector of hashes, should be at most 256 items large, can be smaller if being used
/// for a block whose number is less than 257.
pub type LastHashes = Vec<H256>;

/*
/// A record of execution for a `LOG` operation.
#[derive(Default, Debug, Clone, PartialEq, Eq, RlpEncodable, RlpDecodable)]
pub struct LogEntry {
	/// The address of the contract executing at the point of the `LOG` operation.
	pub address: Address,
	/// The topics associated with the `LOG` operation.
	pub topics: Vec<H256>,
	/// The data associated with the `LOG` operation.
	pub data: Bytes,
}
*/

//impl LogEntry {
//	/// Calculates the bloom of this log entry.
//	pub fn bloom(&self) -> Bloom {
/*
		self.topics.iter().fold(Bloom::from(BloomInput::Raw(&self.address.0)), |mut b, t| {
			b.accrue(BloomInput::Raw(t));
			b
		})
*/
//            Bloom([0; BLOOM_SIZE])
//	}
//}

/// Transaction outcome store in the receipt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransactionOutcome {
	/// Status and state root are unknown under EIP-98 rules.
	Unknown,
	/// State root is known. Pre EIP-98 and EIP-658 rules.
	StateRoot(H256),
	/// Status code is known. EIP-658 rules.
	StatusCode(u8),
}

/// Information describing execution of a transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Receipt {
	/// The total gas used in the block following execution of the transaction.
	pub gas_used: U256,
	/// The OR-wide combination of all logs' blooms for this transaction.
//	pub log_bloom: Bloom,
	/// The logs stemming from this transaction.
//	pub logs: Vec<LogEntry>,
	/// Transaction outcome.
	pub outcome: TransactionOutcome,
}

impl Encodable for Receipt {
	fn rlp_append(&self, s: &mut RlpStream) {
		match self.outcome {
			TransactionOutcome::Unknown => {
				s.begin_list(3);
			},
			TransactionOutcome::StateRoot(ref root) => {
				s.begin_list(4);
				s.append(root);
			},
			TransactionOutcome::StatusCode(ref status_code) => {
				s.begin_list(4);
				s.append(status_code);
			},
		}
		s.append(&self.gas_used);
//		s.append(&self.log_bloom);
//		s.append_list(&self.logs);
	}
}

impl Decodable for Receipt {
	fn decode(rlp: &Rlp) -> Result<Self, DecoderError> {
		if rlp.item_count()? == 3 {
			Ok(Receipt {
				outcome: TransactionOutcome::Unknown,
				gas_used: rlp.val_at(0)?,
//				log_bloom: rlp.val_at(1)?,
//				logs: rlp.list_at(2)?,
			})
		} else {
			Ok(Receipt {
				gas_used: rlp.val_at(1)?,
//				log_bloom: rlp.val_at(2)?,
//				logs: rlp.list_at(3)?,
				outcome: {
					let first = rlp.at(0)?;
					if first.is_data() && first.data()?.len() <= 1 {
						TransactionOutcome::StatusCode(first.as_val()?)
					} else {
						TransactionOutcome::StateRoot(first.as_val()?)
					}
				}
			})
		}
	}
}

/// Semantic boolean for when a seal/signature is included.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy)]
enum Seal {
	/// The seal/signature is included.
	With,
	/// The seal/signature is not included.
	Without,
}

/// Signing error
#[derive(Debug)]
pub enum SignError {
	/// Account is not unlocked
	NotUnlocked,
	/// Account does not exist.
	NotFound,
//	/// Low-level hardware device error.
//	Hardware(HardwareError),
//	/// Low-level error from store
//	SStore(SSError),
}

/// A set of information describing an externally-originating message call
/// or contract creation operation.
#[derive(Default, Debug, Clone, PartialEq, Eq)]
pub struct Transaction {
	/// Nonce.
	pub nonce: U256,
	/// Gas price.
	pub gas_price: U256,
	/// Gas paid up front for transaction execution.
	pub gas: U256,
	/// Action, can be either call or contract create.
	pub action: Action,
	/// Transfered value.
	pub value: U256,
	/// Transaction data.
	pub data: Bytes,
}

/// Replay protection logic for v part of transaction's signature
pub mod signature {
	/// Adds chain id into v
	pub fn add_chain_replay_protection(v: u64, chain_id: Option<u64>) -> u64 {
		v + if let Some(n) = chain_id { 35 + n * 2 } else { 27 }
	}

	/// Returns refined v
	/// 0 if `v` would have been 27 under "Electrum" notation, 1 if 28 or 4 if invalid.
	pub fn check_replay_protection(v: u64) -> u8 {
		match v {
			v if v == 27 => 0,
			v if v == 28 => 1,
			v if v >= 35 => ((v - 1) % 2) as u8,
			_ => 4
		}
	}
}

impl Transaction {
	/// Specify the sender; this won't survive the serialize/deserialize process, but can be cloned.
	pub fn fake_sign(self, from: Address) -> SignedTransaction {
		SignedTransaction {
			transaction: UnverifiedTransaction {
				unsigned: self,
				r: U256::one(),
				s: U256::one(),
				v: 0,
				hash: 0.into(),
			}.compute_hash(),
			sender: from,
			public: None,
		}
	}
	/// Signs the transaction as coming from `sender`.
	pub fn sign(self, _secret: &Secret, chain_id: Option<u64>) -> SignedTransaction {
		let sig = Signature([0; 64]);
//::ethkey::sign(secret, &self.hash(chain_id))
//			.expect("data is valid and context has signing capabilities; qed");
		SignedTransaction::new(self.with_signature(sig, chain_id))
			.expect("secret is valid so it's recoverable")
	}
	/// Signs the transaction with signature.
	pub fn with_signature(self, _sig: Signature, _chain_id: Option<u64>) -> UnverifiedTransaction {
		UnverifiedTransaction {
			unsigned: self,
			r: U256::one(),
			s: U256::one(),
 			v: 0,
//			r: sig.r().into(),
//			s: sig.s().into(),
//			v: signature::add_chain_replay_protection(sig.v() as u64, chain_id),
			hash: 0.into(),
		}.compute_hash()
	}
}

/// Signed transaction information without verified signature.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct UnverifiedTransaction {
	/// Plain Transaction.
	unsigned: Transaction,
	/// The V field of the signature; the LS bit described which half of the curve our point falls
	/// in. The MS bits describe which chain this transaction is for. If 27/28, its for all chains.
	v: u64,
	/// The R field of the signature; helps describe the point on the curve.
	r: U256,
	/// The S field of the signature; helps describe the point on the curve.
	s: U256,
	/// Hash of the transaction
	hash: H256,
}

impl UnverifiedTransaction {
	/// Used to compute hash of created transactions
	fn compute_hash(self) -> UnverifiedTransaction {
//		let hash = keccak(&*self.rlp_bytes());
//		self.hash = hash;
		self
	}
	/// Checks is signature is empty.
	pub fn is_unsigned(&self) -> bool {
		self.r.is_zero() && self.s.is_zero()
	}
	/// Append object with a signature into RLP stream
	fn rlp_append_sealed_transaction(&self, s: &mut RlpStream) {
		s.begin_list(9);
//		s.append(&self.nonce);
//		s.append(&self.gas_price);
//		s.append(&self.gas);
//		s.append(&self.action);
//		s.append(&self.value);
//		s.append(&self.data);
		s.append(&self.v);
		s.append(&self.r);
		s.append(&self.s);
	}
	/// Verify basic signature params. Does not attempt sender recovery.
	pub fn verify_basic(&self, _check_low_s: bool, _chain_id: Option<u64>, _allow_empty_signature: bool) -> Result<(), PoaError> {
/*
		if check_low_s && !(allow_empty_signature && self.is_unsigned()) {
			self.check_low_s()?;
		}
		// Disallow unsigned transactions in case EIP-86 is disabled.
		if !allow_empty_signature && self.is_unsigned() {
			return Err(ethkey::Error::InvalidSignature.into());
		}
		// EIP-86: Transactions of this form MUST have gasprice = 0, nonce = 0, value = 0, and do NOT increment the nonce of account 0.
		if allow_empty_signature && self.is_unsigned() && !(self.gas_price.is_zero() && self.value.is_zero() && self.nonce.is_zero()) {
			return Err(ethkey::Error::InvalidSignature.into())
		}
		match (self.chain_id(), chain_id) {
			(None, _) => {},
			(Some(n), Some(m)) if n == m => {},
			_ => return Err(error::Error::InvalidChainId),
		};
*/
		Ok(())
	}
}

impl rlp::Decodable for UnverifiedTransaction {
	fn decode(d: &Rlp) -> Result<Self, DecoderError> {
		if d.item_count()? != 9 {
			return Err(DecoderError::RlpIncorrectListLen);
		}
		let hash = keccak(d.as_raw());
		Ok(UnverifiedTransaction {                         
			unsigned: Transaction {
				nonce: d.val_at(0)?,
				gas_price: d.val_at(1)?,
				gas: d.val_at(2)?,
				action: d.val_at(3)?,
				value: d.val_at(4)?,
				data: d.val_at(5)?,
			},
			v: d.val_at(6)?,
			r: d.val_at(7)?,
			s: d.val_at(8)?,
			hash: hash.into(),
		})
	}
}

/// A `UnverifiedTransaction` with successfully recovered `sender`.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct SignedTransaction {
	transaction: UnverifiedTransaction,
	sender: Address,
	public: Option<Public>,
}

impl SignedTransaction {
	/// Try to verify transaction and recover sender.
	pub fn new(transaction: UnverifiedTransaction) -> Result<Self, PoaError> {
		if transaction.is_unsigned() {
			Ok(SignedTransaction {
				transaction: transaction,
				sender: UNSIGNED_SENDER,
				public: None,
			})
		} else {
//			let public = transaction.recover_public()?;
//			let sender = public_to_address(&public);
			Ok(SignedTransaction {
				transaction: transaction,
				sender: UNSIGNED_SENDER,//sender,
				public: None//Some(public),
			})
		}
	}
}

impl rlp::Encodable for SignedTransaction {
	fn rlp_append(&self, s: &mut RlpStream) { self.transaction.rlp_append_sealed_transaction(s) }
}

/// Fake address for unsigned transactions as defined by EIP-86.
pub const UNSIGNED_SENDER: Address = H256([0xff; 32]);
/// System sender address for internal state updates.
pub const SYSTEM_ADDRESS: Address = H256([
    0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 
    0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 
    0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 
    0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xfe
]);

impl Display for SignError {
	fn fmt(&self, f: &mut Formatter) -> Result<(), std::fmt::Error> {
		match *self {
			SignError::NotUnlocked => write!(f, "Account is locked"),
			SignError::NotFound => write!(f, "Account does not exist"),
//			SignError::Hardware(ref e) => write!(f, "{}", e),
//			SignError::SStore(ref e) => write!(f, "{}", e),
		}
	}
}

/// Different ways of specifying validators.
#[derive(Debug, PartialEq/*, Deserialize*/)]
//#[serde(deny_unknown_fields)]
//#[serde(rename_all = "camelCase")]
pub enum ValidatorSpec {
	/// A simple list of authorities.
	List(Vec<PublicKey>),
	/// Address of a contract that indicates the list of authorities.
	SafeContract(Address),
	/// Address of a contract that indicates the list of authorities and enables reporting of theor misbehaviour using transactions.
	Contract(Address),
	/// A map of starting blocks for each validator set.
	Multi(BTreeMap<u64, ValidatorSpec>),
}

pub type Address = H256;
pub type BlockNumber = u64;
pub type Message = H256;
pub type Public = H512;
pub type Secret = H256;

#[derive(Clone, Debug)]
pub struct Password(String);

/*
impl From<ethkey::Password> for Password{ 
    fn from(x: ethkey::Password) -> Self { 
        Password(x.as_str().to_string())
    } 
} 
*/

impl From<String> for Password {
	fn from(s: String) -> Password {
		Password(s)
	}
}

impl<'a> From<&'a str> for Password {
	fn from(s: &'a str) -> Password {
		Password::from(String::from(s))
	}
}

#[derive(Clone)]
pub struct Signature(pub [u8; 64]);

impl Debug for Signature {
	fn fmt(&self, f: &mut Formatter) -> Result<(), std::fmt::Error> {
		write!(f, "{}", &self.0[..].to_hex())
	}
}

impl rlp::Decodable for Signature {
    fn decode(rlp: &Rlp) -> Result<Self, DecoderError> {
        rlp.decoder().decode_value(|bytes| match bytes.len().cmp(&64) {
            cmp::Ordering::Less => Err(DecoderError::RlpIsTooShort),
            cmp::Ordering::Greater => Err(DecoderError::RlpIsTooBig),
            cmp::Ordering::Equal => {
                let mut t = [0u8; 64];
                t.copy_from_slice(bytes);
                Ok(Signature(t))
            }
        })
    }
}

impl Default for Signature {
    #[inline]
    fn default() -> Self {
        Signature([0; 64])
    }
}

impl Display for Signature {
	fn fmt(&self, f: &mut Formatter) -> Result<(), std::fmt::Error> {
		write!(f, "{}", &self.0[..].to_hex())
	}
}

impl From<u8> for Signature {
    fn from(x: u8) -> Self {
        let mut r: [u8; 64] = [0; 64]; 
        r[0] = x;
        Signature(r) 
    }
}

impl rlp::Encodable for Signature {
    fn rlp_append(&self, s: &mut RlpStream) {
        s.encoder().encode_value(&self.0);
    }
}

impl Eq for Signature {}

impl Ord for Signature {
    fn cmp(&self, other: &Self) -> cmp::Ordering {
        for i in (0usize..64).rev() {
            if self.0[i] < other.0[i] {
                return cmp::Ordering::Less
            } else if self.0[i] > other.0[i] {
                return cmp::Ordering::Greater
            }
        }
        cmp::Ordering::Equal
    }
}

impl PartialEq for Signature {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == cmp::Ordering::Equal
    }
}

impl PartialOrd for Signature {
    fn partial_cmp(&self, other: &Self) -> Option<cmp::Ordering> {
        Some(self.cmp(other))
    }
}

pub fn verify_signed_block(key: &PublicKey, signed_block: &SignedBlock, _block_bytes: &Vec<u8>, _319offset: usize) -> Result<bool, PoaError> {
    let ret = signed_block.verify_signature(key).map_err(|_| BlockError::InvalidSeal)?;
/*  
    let results = check_signatures(&signed_block, &block_bytes[offset..], &vec!(key)).unwrap();
    for pair in results {
        println!("{} signature by 0x{:016x}", if pair.1 {"Valid"} else {"INVALID"}, pair.0);
    }   
*/
/*
    let public = recover(signature, message)?;
    let recovered_address = public_to_address(&public);
    Ok(address == &recovered_address)
*/
    Ok(ret)
}

pub fn verify_address(_address: &Address, _signature: &Signature, _message: &Message) -> Result<bool, PoaError> {
/*
	let public = recover(signature, message)?;
	let recovered_address = public_to_address(&public);
	Ok(address == &recovered_address)
*/
    Ok(true)
}

pub fn verify_signature(key: &PublicKey, signature: &Signature, message: &Message) -> Result<bool, PoaError> {
    let ds = ed25519::signature::Signature::from_bytes(&signature.0[..])
        .map_err(|_| BlockError::InvalidSeal)?;
    key.verify(&message.0[..], &ds).map_err(|_| BlockError::InvalidSeal)?;
    Ok(true)
}

/// Type alias for a function we can make calls through synchronously.
/// Returns the call result and state proof for each call.
pub type Call<'a> = dyn Fn(Address, Vec<u8>) -> Result<(Vec<u8>, Vec<Vec<u8>>), String> + 'a;

/*
const BLOOM_SIZE: usize = 256;
pub struct BloomRef<'a>(&'a [u8; BLOOM_SIZE]);

impl<'a> BloomRef<'a> { 
    pub fn contains_bloom<'b, B>(&self, bloom: B) -> bool where BloomRef<'b>: From<B> { 
	let bloom_ref: BloomRef = bloom.into(); 
	assert_eq!(self.0.len(), BLOOM_SIZE); 
	assert_eq!(bloom_ref.0.len(), BLOOM_SIZE); 
	for i in 0..BLOOM_SIZE { 
		let a = self.0[i]; 
		let b = bloom_ref.0[i]; 
		if (a & b) != b { 
			return false; 
		} 
	} 
	true 
    } 
}

impl<'a> From<&'a Bloom> for BloomRef<'a> { 
    fn from(bloom: &'a Bloom) -> Self { 
        BloomRef(&bloom.0) 
    } 
} 

#[derive(Clone)]
pub struct Bloom(pub [u8; BLOOM_SIZE]);

impl Bloom {
    pub fn accrue_bloom<'a, B>(&mut self, bloom: B) where BloomRef<'a>: From<B> { 
        let bloom_ref: BloomRef = bloom.into(); 
 	assert_eq!(self.0.len(), BLOOM_SIZE); 
	assert_eq!(bloom_ref.0.len(), BLOOM_SIZE);
        for i in 0..BLOOM_SIZE { 
            self.0[i] |= bloom_ref.0[i]; 
        } 
    } 
    pub fn contains_bloom<'a, B>(&self, bloom: B) -> bool where BloomRef<'a>: From<B> { 
        let bloom_ref: BloomRef = bloom.into(); 
	// workaround for https://github.com/rust-lang/rust/issues/43644 
	self.contains_bloom_ref(bloom_ref) 
    } 
    fn contains_bloom_ref(&self, bloom: BloomRef) -> bool { 
	let self_ref: BloomRef = self.into(); 
	self_ref.contains_bloom(bloom) 
    } 
    pub fn zero() -> Self {
        Bloom([0; BLOOM_SIZE])
    }
}

impl cmp::Ord for Bloom {
    fn cmp(&self, other: &Self) -> cmp::Ordering {
        for i in (0usize..BLOOM_SIZE).rev() {
            if self.0[i] < other.0[i] {
                return cmp::Ordering::Less
            } else if self.0[i] > other.0[i] {
                return cmp::Ordering::Greater
            }
        }
        cmp::Ordering::Equal
    }
}

impl std::cmp::Eq for Bloom {}

impl std::cmp::PartialEq for Bloom
 {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == cmp::Ordering::Equal
    }
}

impl std::cmp::PartialOrd for Bloom {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl LowerHex for Bloom {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        if f.alternate() {
	    std::write!(f, "0x")?;
	}
	for i in &self.0[..] {
            std::write!(f, "{:02x}", i)?;
	}
	Ok(())
    }
}

impl std::fmt::Debug for Bloom {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        std::write!(f, "{:#x}", self)
    }
}

impl std::default::Default for Bloom {
    #[inline]
    fn default() -> Self {
        Bloom([0; BLOOM_SIZE])
    }
}

impl rlp::Decodable for Bloom {
    fn decode(rlp: &Rlp) -> Result<Self, DecoderError> {
        rlp.decoder().decode_value(|bytes| match bytes.len().cmp(&BLOOM_SIZE) {
            cmp::Ordering::Less => Err(DecoderError::RlpIsTooShort),
            cmp::Ordering::Greater => Err(DecoderError::RlpIsTooBig),
            cmp::Ordering::Equal => {
                let mut t = [0u8; BLOOM_SIZE];
                t.copy_from_slice(bytes);
                Ok(Bloom(t))
            }
        })
    }
}

impl rlp::Encodable for Bloom {
    fn rlp_append(&self, s: &mut RlpStream) {
        s.encoder().encode_value(&self.0);
    }
}
*/

#[derive(Copy, Clone, Debug, Default, Eq, Hash, PartialEq, PartialOrd)]
pub struct H256(pub [u8; 32]);

impl H256 {
    pub fn to_vec(&self) -> Vec<u8> {
        self.0.to_vec()
    }
}

impl cmp::Ord for H256 {
    fn cmp(&self, other: &Self) -> cmp::Ordering {
        for i in (0usize..32).rev() {
            if self.0[i] < other.0[i] {
                return cmp::Ordering::Less
            } else if self.0[i] > other.0[i] {
                return cmp::Ordering::Greater
            }
        }
        cmp::Ordering::Equal
    }
}

impl From<&[u8]> for H256 {
    fn from(x: &[u8]) -> Self {
        let mut r: [u8; 32] = [0; 32]; 
        let j = cmp::min(x.len(), 32);
        for i in 0..j {
           r[i] = x[i];
        }
        H256(r) 
    }
}

impl From<u8> for H256 {
    fn from(x: u8) -> Self {
        let mut r: [u8; 32] = [0; 32]; 
        r[0] = x;
        H256(r) 
    }
}

impl From<&[u8; 32]> for H256 {
    fn from(x: &[u8; 32]) -> Self {
        H256(x.clone()) 
    }
}


impl FromStr for H256 {
    type Err = ::rustc_hex::FromHexError;
    fn from_str(input: &str) -> Result<H256, ::rustc_hex::FromHexError> {
        use ::rustc_hex::FromHex;
        let bytes: Vec<u8> = input.from_hex()?;
        if bytes.len() != 32 {
            return Err(::rustc_hex::FromHexError::InvalidHexLength);
        }
        Ok(H256::from_slice(&bytes))
    }
}

impl From<hash::H256> for H256 {
    fn from(x: hash::H256) -> Self {
        let hash::H256(ref arr) = &x;
        H256([ 
            arr[ 0], arr[ 1], arr[ 2], arr[ 3], arr[ 4], arr[ 5], arr[ 6], arr[ 7],
            arr[ 8], arr[ 9], arr[10], arr[11], arr[12], arr[13], arr[14], arr[15],
            arr[16], arr[17], arr[18], arr[19], arr[20], arr[21], arr[22], arr[23],
            arr[24], arr[25], arr[26], arr[27], arr[28], arr[29], arr[30], arr[31],
        ])
    }
}

impl LowerHex for H256 {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        if f.alternate() {
	    std::write!(f, "0x")?;
	}
	for i in &self.0[..] {
            std::write!(f, "{:02x}", i)?;
	}
	Ok(())
    }
}

impl rlp::Decodable for H256 {
    fn decode(rlp: &Rlp) -> Result<Self, DecoderError> {
        rlp.decoder().decode_value(|bytes| match bytes.len().cmp(&32) {
            cmp::Ordering::Less => Err(DecoderError::RlpIsTooShort),
            cmp::Ordering::Greater => Err(DecoderError::RlpIsTooBig),
            cmp::Ordering::Equal => {
                let mut t = [0u8; 32];
                t.copy_from_slice(bytes);
                Ok(H256(t))
            }
        })
    }
}

impl rlp::Encodable for H256 {
    fn rlp_append(&self, s: &mut RlpStream) {
        s.encoder().encode_value(&self.0);
    }
}

impl rand::distributions::Distribution<H256> for rand::distributions::Standard {
    fn sample<R: rand::Rng + ?Sized>(&self, rng: &mut R) -> H256 {
        let mut ret = H256::zero();
        for i in 0..32 {
	    ret.0[i] = rng.gen();
	}
        ret
    }
}

impl H256 {
    pub fn zero() -> Self {
        H256([0; 32])
    }
    /// Create a new hash with cryptographically random content.
    pub fn random() -> Self {
        let mut rng = rand::rngs::OsRng;
        rand::distributions::Standard.sample(&mut rng)
    }
    pub fn from_slice(src: &[u8]) -> Self {
        let mut ret = Self::zero();
        ret.0.copy_from_slice(src);
        ret
    }
}

#[derive(Clone)]
pub struct H512(pub [u8; 64]);

impl cmp::Ord for H512 {
    fn cmp(&self, other: &Self) -> cmp::Ordering {
        for i in (0usize..64).rev() {
            if self.0[i] < other.0[i] {
                return cmp::Ordering::Less
            } else if self.0[i] > other.0[i] {
                return cmp::Ordering::Greater
            }
        }
        cmp::Ordering::Equal
    }
}

impl std::cmp::Eq for H512 {}

impl std::cmp::PartialEq for H512 {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == cmp::Ordering::Equal
    }
}

impl std::cmp::PartialOrd for H512 {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl std::fmt::Debug for H512 {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        std::write!(f, "{:#x}", self)
    }
}

impl LowerHex for H512 {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        if f.alternate() {
	    std::write!(f, "0x")?;
	}
	for i in &self.0[..] {
            std::write!(f, "{:02x}", i)?;
	}
	Ok(())
    }
}

#[derive(Clone)]
pub struct H520(pub [u8; 65]);

impl cmp::Ord for H520 {
    fn cmp(&self, other: &Self) -> cmp::Ordering {
        for i in (0usize..65).rev() {
            if self.0[i] < other.0[i] {
                return cmp::Ordering::Less
            } else if self.0[i] > other.0[i] {
                return cmp::Ordering::Greater
            }
        }
        cmp::Ordering::Equal
    }
}

impl std::fmt::Debug for H520 {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        std::write!(f, "{:#x}", self)
    }
}

impl std::cmp::Eq for H520 {}

impl std::cmp::PartialEq for H520 {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == cmp::Ordering::Equal
    }
}

impl std::cmp::PartialOrd for H520 {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl std::default::Default for H520 {
    #[inline]
    fn default() -> Self {
        H520([0; 65])
    }
}

impl AsRef<[u8]> for H520 {
    #[inline]
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

impl From<Signature> for H520 {
    fn from(x: Signature) -> Self {
        let &Signature(ref arr) = &x;
        let mut r: [u8; 65] = [0; 65]; 
        for i in 0..64 {
            r[i] = arr[i]
        }
        H520(r) 
    }
}

impl From<u8> for H520 {
    fn from(x: u8) -> Self {
        let mut r: [u8; 65] = [0; 65]; 
        r[0] = x;
        H520(r) 
    }
}

impl Into<Signature> for H520 {
    fn into(self) -> Signature {
        let &H520(ref arr) = &self;
        let mut r: [u8; 64] = [0; 64]; 
        for i in 0..64 {
            r[i] = arr[i]
        }
        Signature(r) 
    }
}

impl LowerHex for H520 {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        if f.alternate() {
	    std::write!(f, "0x")?;
	}
	for i in &self.0[..] {
            std::write!(f, "{:02x}", i)?;
	}
	Ok(())
    }
}

impl rlp::Decodable for H520 {
    fn decode(rlp: &Rlp) -> Result<Self, DecoderError> {
        rlp.decoder().decode_value(|bytes| match bytes.len().cmp(&65) {
            cmp::Ordering::Less => Err(DecoderError::RlpIsTooShort),
            cmp::Ordering::Greater => Err(DecoderError::RlpIsTooBig),
            cmp::Ordering::Equal => {
                let mut t = [0u8; 65];
                t.copy_from_slice(bytes);
                Ok(H520(t))
            }
        })
    }
}

impl rlp::Encodable for H520 {
    fn rlp_append(&self, s: &mut RlpStream) {
        s.encoder().encode_value(&self.0);
    }
}

pub struct U128(pub [u64; 2]);

impl U128 {
    const MAX: U128 = U128([<u64>::max_value(); 2]);
    pub fn max_value() -> Self {
        U128::MAX
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, PartialOrd)]
pub struct U256(pub [u64; 4]);

impl U256 {
    const MAX: U256 = U256([<u64>::max_value(); 4]);
    pub fn max_value() -> Self {
        U256::MAX
    }
    pub fn is_zero(&self) -> bool {
        (self.0[0] == 0) && (self.0[1] == 0) && (self.0[2] == 0) && (self.0[3] == 0)
    }
    pub fn one() -> Self {
        let mut ret = U256([0; 4]);
        ret.0[0] = 1;
        ret
    }
    pub fn zero() -> Self {
        U256([0; 4])
    }
    #[inline]
    pub fn bits(&self) -> usize {
        let &U256(ref arr) = self;
        for i in 1..4 {
            if arr[4 - i] > 0 { return (0x40 * (4 - i + 1)) - arr[4 - i].leading_zeros() as usize; }
        }
        0x40 - arr[0].leading_zeros() as usize
    }
    /// Converts from big endian representation bytes in memory.
    pub fn from_big_endian(slice: &[u8]) -> Self {
        assert!(4 * 8 >= slice.len());
        let mut ret = [0; 4];
        unsafe {
            let ret_u8: &mut [u8; 4 * 8] = std::mem::transmute(&mut ret);
            let mut ret_ptr = ret_u8.as_mut_ptr();
            let mut slice_ptr = slice.as_ptr().offset(slice.len() as isize - 1);
            for _ in 0..slice.len() {
                *ret_ptr = *slice_ptr;
                ret_ptr = ret_ptr.offset(1);
                slice_ptr = slice_ptr.offset(-1);
            }
        }
        U256(ret)
    }
    /// Write to the slice in big-endian format.
    #[inline]
    pub fn to_big_endian(&self, bytes: &mut [u8]) {
        debug_assert!(4 * 8 == bytes.len());
        for i in 0..4 {
			bytes[8 * i..].copy_from_slice(&self.0[4 - i - 1].to_be_bytes());
        }
    }
}

impl cmp::Ord for U256 {
    fn cmp(&self, other: &Self) -> cmp::Ordering {
        for i in (0usize..4).rev() {
            if self.0[i] < other.0[i] {
                return cmp::Ordering::Less
            } else if self.0[i] > other.0[i] {
                return cmp::Ordering::Greater
            }
        }
        cmp::Ordering::Equal
    }
}

impl Add for U256 {
    type Output = U256;
    fn add(self, other: U256) -> U256{
        let mut a: u128 = 0;
        let mut r = U256([0; 4]);
        a += self.0[0] as u128 + other.0[0] as u128;
        r.0[0] = a as u64;
        a >>= 64;
        a += self.0[1] as u128 + other.0[1] as u128;
        r.0[1] = a as u64;
        a >>= 64;
        a += self.0[2] as u128 + other.0[2] as u128;
        r.0[2] = a as u64;
        a >>= 64;
        a += self.0[3] as u128 + other.0[3] as u128;
        r.0[3] = a as u64;
        r
    }
}

impl Sub for U256 {
    type Output = U256;
    fn sub(self, other: U256) -> U256{
        let mut a: i128 = 0;
        let mut r = U256([0; 4]);
        a += self.0[0] as i128 - other.0[0] as i128;
        r.0[0] = a as u64;
        a >>= 64;
        a += self.0[1] as i128 + other.0[1] as i128;
        r.0[1] = a as u64;
        a >>= 64;
        a += self.0[2] as i128 + other.0[2] as i128;
        r.0[2] = a as u64;
        a >>= 64;
        a += self.0[3] as i128 + other.0[3] as i128;
        r.0[3] = a as u64;
        r
    }
}

fn uint_full_mul_reg(n_words: usize, x: &U256, y: &U256) -> U256 {
    let mut ret = [0u64; 8];
    for i in 0..n_words {
        let mut carry = 0u64;
	let b = y.0[i];
        for j in 0..n_words {
            if (x.0[j] != 0) || (carry != 0) {
                let a = x.0[j];
                let (hi, low) = {
                    let t: u128 = a as u128 * b as u128;
                    ( (t >> 64) as u64, t as u64)
                };
                let overflow = {
                    let existing_low = &mut ret[i + j];
                    let (low, o) = low.overflowing_add(*existing_low);
                    *existing_low = low;
                    o
                };
                carry = {
                    let existing_hi = &mut ret[i + j + 1];
                    let hi = hi + overflow as u64;
                    let (hi, o0) = hi.overflowing_add(carry);
                    let (hi, o1) = hi.overflowing_add(*existing_hi);
                    *existing_hi = hi;
                    (o0 | o1) as u64
                }
            }
        }
    }
    U256([ ret[0], ret[1], ret[2], ret[3] ])
}

impl std::ops::Mul for U256 {
    type Output = U256; 
    fn mul(self, other: U256) -> U256 {
        uint_full_mul_reg(4, &self, &other)
    }
}

impl<T> std::ops::Shr<T> for U256 
    where T: Into<U256> 
{
    type Output = U256;
    fn shr(self, shift: T) -> U256 {
        let shift = shift.into().0[0] as usize;
        let U256(ref original) = self;
        let mut ret = [0u64; 4];
        let word_shift = shift / 64;
        let bit_shift = shift % 64;
        // shift
        for i in word_shift..4 {
            ret[i - word_shift] = original[i] >> bit_shift;
        }
        // Carry
        if bit_shift > 0 {
	    for i in word_shift+1..4 {
	        ret[i - word_shift - 1] += original[i] << (64 - bit_shift);
            }
	}
        U256(ret)
    }
}

impl From<u64> for U256 {
    fn from(x: u64) -> U256 {
        U256([ x, 0, 0, 0 ])
    }
}

impl From<usize> for U256 {
    fn from(x: usize) -> U256 {
        U256([ x as u64, 0, 0, 0 ])
    }
}

impl From<u128> for U256 {
    fn from(x: u128) -> U256 {
        U256([ x as u64, (x >> 64) as u64, 0, 0 ])
    }
}

impl From<U128> for U256 {
    fn from(x: U128) -> U256 {
        U256([ x.0[0], x.0[1], 0, 0 ])
    }
}

impl<'a> From<&'a [u8]> for U256 {
    fn from(bytes: &[u8]) -> U256 {
        Self::from_big_endian(bytes)
    }
}

impl<'a> From<&'a [u8; 4 * 8]> for U256 {
    fn from(bytes: &[u8; 4 * 8]) -> Self {
        bytes[..].into()
    }
}

impl Decodable for U256 {
    fn decode(rlp: &Rlp) -> Result<Self, DecoderError> {
        rlp.decoder().decode_value(|bytes| {
            if !bytes.is_empty() && bytes[0] == 0 {
                Err(DecoderError::RlpInvalidIndirection)
            } else if bytes.len() <= 32 {
                Ok(U256::from(bytes))
            } else {
                Err(DecoderError::RlpIsTooBig)
            }
        })
    }
}

impl Encodable for U256 {
    fn rlp_append(&self, s: &mut RlpStream) {
        let leading_empty_bytes = 32 - (self.bits() + 7) / 8;
	let mut buffer = [0u8; 32];
        self.to_big_endian(&mut buffer);
        s.encoder().encode_value(&buffer[leading_empty_bytes..]);
    }
}


/// Blockchain database client. Owns and manages a blockchain and a block queue.
pub trait BlockChainClient : 
Sync + Send + CallContract /*+ AccountData + BlockChain + RegistryInfo + ImportBlock
+ IoClient + BadBlocks*/ {
	/// Schedule state-altering transaction to be executed on the next pending block.
	fn transact_contract(&self, address: Address, data: Bytes) -> Result<(), PoaError>;
/*
	/// Look up the block number for the given block ID.
	fn block_number(&self, id: BlockId) -> Option<BlockNumber>;

	/// Get raw block body data by block id.
	/// Block body is an RLP list of two items: uncles and transactions.
	fn block_body(&self, id: BlockId) -> Option<encoded::Body>;

	/// Get block status by block header hash.
	fn block_status(&self, id: BlockId) -> BlockStatus;

	/// Get block total difficulty.
	fn block_total_difficulty(&self, id: BlockId) -> Option<U256>;

	/// Attempt to get address storage root at given block.
	/// May not fail on BlockId::Latest.
	fn storage_root(&self, address: &Address, id: BlockId) -> Option<H256>;

	/// Get block hash.
	fn block_hash(&self, id: BlockId) -> Option<H256>;

	/// Get address code at given block's state.
	fn code(&self, address: &Address, state: StateOrBlock) -> Option<Option<Bytes>>;

	/// Get address code at the latest block's state.
	fn latest_code(&self, address: &Address) -> Option<Bytes> {
		self.code(address, BlockId::Latest.into())
			.expect("code will return Some if given BlockId::Latest; qed")
	}

	/// Get address code hash at given block's state.

	/// Get value of the storage at given position at the given block's state.
	///
	/// May not return None if given BlockId::Latest.
	/// Returns None if and only if the block's root hash has been pruned from the DB.
	fn storage_at(&self, address: &Address, position: &H256, state: StateOrBlock) -> Option<H256>;

	/// Get value of the storage at given position at the latest block's state.
	fn latest_storage_at(&self, address: &Address, position: &H256) -> H256 {
		self.storage_at(address, position, BlockId::Latest.into())
			.expect("storage_at will return Some if given BlockId::Latest. storage_at was given BlockId::Latest. \
			Therefore storage_at has returned Some; qed")
	}

	/// Get a list of all accounts in the block `id`, if fat DB is in operation, otherwise `None`.
	/// If `after` is set the list starts with the following item.
	fn list_accounts(&self, id: BlockId, after: Option<&Address>, count: u64) -> Option<Vec<Address>>;

	/// Get a list of all storage keys in the block `id`, if fat DB is in operation, otherwise `None`.
	/// If `after` is set the list starts with the following item.
	fn list_storage(&self, id: BlockId, account: &Address, after: Option<&H256>, count: u64) -> Option<Vec<H256>>;

	/// Get transaction with given hash.
	fn transaction(&self, id: TransactionId) -> Option<LocalizedTransaction>;

	/// Get uncle with given id.
	fn uncle(&self, id: UncleId) -> Option<encoded::Header>;

	/// Get transaction receipt with given hash.
	fn transaction_receipt(&self, id: TransactionId) -> Option<LocalizedReceipt>;

	/// Get localized receipts for all transaction in given block.
	fn localized_block_receipts(&self, id: BlockId) -> Option<Vec<LocalizedReceipt>>;

	/// Get a tree route between `from` and `to`.
	/// See `BlockChain::tree_route`.
	fn tree_route(&self, from: &H256, to: &H256) -> Option<TreeRoute>;

	/// Get all possible uncle hashes for a block.
	fn find_uncles(&self, hash: &H256) -> Option<Vec<H256>>;

	/// Get latest state node
	fn state_data(&self, hash: &H256) -> Option<Bytes>;

	/// Get block receipts data by block header hash.
	fn block_receipts(&self, hash: &H256) -> Option<BlockReceipts>;

	/// Get block queue information.
	fn queue_info(&self) -> BlockQueueInfo;

	/// Returns true if block queue is empty.
	fn is_queue_empty(&self) -> bool {
		self.queue_info().is_empty()
	}

	/// Clear block queue and abort all import activity.
	fn clear_queue(&self);

	/// Get the registrar address, if it exists.
	fn additional_params(&self) -> BTreeMap<String, String>;

	/// Returns logs matching given filter. If one of the filtering block cannot be found, returns the block id that caused the error.
	fn logs(&self, filter: Filter) -> Result<Vec<LocalizedLogEntry>, BlockId>;

	/// Replays a given transaction for inspection.
	fn replay(&self, t: TransactionId, analytics: CallAnalytics) -> Result<Executed, CallError>;

	/// Replays all the transactions in a given block for inspection.
	fn replay_block_transactions(&self, block: BlockId, analytics: CallAnalytics) -> Result<Box<Iterator<Item = (H256, Executed)>>, CallError>;

	/// Returns traces matching given filter.
	fn filter_traces(&self, filter: TraceFilter) -> Option<Vec<LocalizedTrace>>;

	/// Returns trace with given id.
	fn trace(&self, trace: TraceId) -> Option<LocalizedTrace>;

	/// Returns traces created by transaction.
	fn transaction_traces(&self, trace: TransactionId) -> Option<Vec<LocalizedTrace>>;

	/// Returns traces created by transaction from block.
	fn block_traces(&self, trace: BlockId) -> Option<Vec<LocalizedTrace>>;

	/// Get last hashes starting from best block.
	fn last_hashes(&self) -> LastHashes;

	/// List all ready transactions that should be propagated to other peers.
	fn transactions_to_propagate(&self) -> Vec<Arc<VerifiedTransaction>>;

	/// Sorted list of transaction gas prices from at least last sample_size blocks.
	fn gas_price_corpus(&self, sample_size: usize) -> ::stats::Corpus<U256> {
		let mut h = self.chain_info().best_block_hash;
		let mut corpus = Vec::new();
		while corpus.is_empty() {
			for _ in 0..sample_size {
				let block = match self.block(BlockId::Hash(h)) {
					Some(block) => block,
					None => return corpus.into(),
				};

				if block.number() == 0 {
					return corpus.into();
				}
				block.transaction_views().iter().foreach(|t| corpus.push(t.gas_price()));
				h = block.parent_hash().clone();
			}
		}
		corpus.into()
	}

	/// Get the preferred chain ID to sign on
	fn signing_chain_id(&self) -> Option<u64>;

	/// Get the mode.
	fn mode(&self) -> Mode;

	/// Set the mode.
	fn set_mode(&self, mode: Mode);

	/// Get the chain spec name.
	fn spec_name(&self) -> String;

	/// Set the chain via a spec name.
	fn set_spec_name(&self, spec_name: String);

	/// Disable the client from importing blocks. This cannot be undone in this session and indicates
	/// that a subsystem has reason to believe this executable incapable of syncing the chain.
	fn disable(&self);

	/// Returns engine-related extra info for `BlockId`.
	fn block_extra_info(&self, id: BlockId) -> Option<BTreeMap<String, String>>;

	/// Returns engine-related extra info for `UncleId`.
	fn uncle_extra_info(&self, id: UncleId) -> Option<BTreeMap<String, String>>;

	/// Returns information about pruning/data availability.
	fn pruning_info(&self) -> PruningInfo;

	/// Get the address of the registry itself.
	fn registrar_address(&self) -> Option<Address>;
*/
}

/// Provides `call_contract` method
pub trait CallContract {
	/// Like `call`, but with various defaults. Designed to be used for calling contracts.
	fn call_contract(&self, id: BlockId, address: Address, data: Bytes) -> Result<Bytes, String>;
}

/// Client facilities used by internally sealing Engines.
pub trait EngineClient: Sync + Send/* + ChainInfo*/ {
	/// Attempt to cast the engine client to a full client.
	fn as_full_client(&self) -> Option<&dyn BlockChainClient>;
	/// Get raw block header data by block id.
	fn block_header(&self, id: BlockId) -> Option<OrdinaryHeader>;
	/// Get a block number by ID.
	fn block_number(&self, id: BlockId) -> Option<BlockNumber>;
	/// Broadcast a consensus message to the network.
	fn broadcast_consensus_message(&self, message: Bytes);
	/// Get the transition to the epoch the given parent hash is part of
	/// or transitions to.
	/// This will give the epoch that any children of this parent belong to.
	///
	/// The block corresponding the the parent hash must be stored already.
	fn epoch_transition_for(&self, parent_hash: H256) -> Option<EpochTransition>;
	/// Make a new block and seal it.
	fn update_sealing(&self);
/*
	/// Submit a seal for a block in the mining queue.
	fn submit_seal(&self, block_hash: H256, seal: Vec<Bytes>);
*/
}

/// A header. This contains important metadata about the block, as well as a
/// "seal" that indicates validity to a consensus engine.
pub trait Header {
	/// The author of the header.
	fn author(&self) -> &Address;
	/// Cryptographic hash of the header, including the seal.
	fn hash(&self) -> H256;
	/// The number of the header.
	fn number(&self) -> u64;
/*
	/// Cryptographic hash of the header, excluding the seal.
	fn bare_hash(&self) -> H256;
	/// Get a reference to the seal fields.
	fn seal(&self) -> &[Vec<u8>];
*/
}

/// Trait for a object that is a `ExecutedBlock`.
pub trait IsBlock {
	/// Get the `ExecutedBlock` associated with this object.
	fn block(&self) -> &ExecutedBlock;
	/// Get the final state associated with this object's block.
//	fn state(&self) -> &State<StateDB> { &self.block().state }
	/// Get all information on transactions in this block.
	fn transactions(&self) -> &[SignedTransaction] { &self.block().transactions }
/*
	/// Get the header associated with this object's block.
	fn header(&self) -> &OrdinaryHeader { &self.block().header }

	/// Get the base `Block` object associated with this.
	fn to_base(&self) -> Block {
		Block {
			header: self.header().clone(),
			transactions: self.transactions().iter().cloned().map(Into::into).collect(),
			uncles: self.uncles().to_vec(),
		}
	}

	/// Get all information on receipts in this block.
	fn receipts(&self) -> &[Receipt] { &self.block().receipts }

	/// Get all uncles in this block.
	fn uncles(&self) -> &[OrdinaryHeader] { &self.block().uncles }
*/
}

/// A "live" block is one which is in the process of the transition.
/// The state of this block can be mutated by arbitrary rules of the
/// state transition function.
pub trait LiveBlock: 'static {
	/// The block header type;
	type Header: Header;
	/// Get a reference to the header.
	fn header(&self) -> &Self::Header;
	/// Get a reference to the uncle headers. If the block type doesn't
	/// support uncles, return the empty slice.
	fn uncles(&self) -> &[Self::Header];
}

/// Machine-related types localized to a specific lifetime.
// TODO: this is a workaround for a lack of associated type constructors in the language.
pub trait LocalizedMachine<'a>: Sync + Send {
	/// Definition of auxiliary data associated to a specific block.
	type AuxiliaryData: 'a;
	/// A context providing access to the state in a controlled capacity.
	/// Generally also provides verifiable proofs.
	type StateContext: ?Sized + 'a;
}

/// Generalization of types surrounding blockchain-suitable state machines.
pub trait Machine: for<'a> LocalizedMachine<'a> {
	/// A description of needed auxiliary data.
	type AuxiliaryRequest;
	/// The block header type.
	type Header: Header;
	/// A handle to a blockchain client for this machine.
	type EngineClient: ?Sized;
	/// Errors which can occur when querying or interacting with the machine.
	type Error;
	/// Block header with metadata information.
	type ExtendedHeader: Header;
	/// The live block type.
	type LiveBlock: LiveBlock<Header=Self::Header>;
/*
	/// Actions taken on ancestry blocks when commiting a new block.
	type AncestryAction;
*/
}

/// A header with associated total score.
pub trait TotalScoredHeader: Header {
	type Value;
	/// Get the total score of this header.
	fn total_score(&self) -> Self::Value;
}

/// Trait for blocks which have a transaction type.
pub trait Transactions: LiveBlock {
	/// The transaction type.
	type Transaction;
	/// Get a reference to the transactions in this block.
	fn transactions(&self) -> &[Self::Transaction];
}

/// A state machine that uses balances.
pub trait WithBalances: Machine {
	/// Increment the balance of an account in the state of the live block.
	fn add_balance(&self, live: &mut Self::LiveBlock, address: &Address, amount: &U256) -> Result<(), Self::Error>;
/*
	/// Get the balance, in base units, associated with an account.
	/// Extracts data from the live block.
	fn balance(&self, live: &Self::LiveBlock, address: &Address) -> Result<U256, Self::Error>;
*/
}

/// A state machine that uses block rewards.
pub trait WithRewards: Machine {
	/// Note block rewards, traces each reward storing information about benefactor, amount and type
	/// of reward.
	fn note_rewards(
		&self,
		live: &mut Self::LiveBlock,
		rewards: &[(Address, RewardKind, U256)],
	) -> Result<(), Self::Error>;
}

/// Account management.
/// Responsible for unlocking accounts.
#[derive(Debug)]
pub struct AccountProvider {
}

impl AccountProvider {
	/// Inserts new account into underlying store.
	/// Does not unlock account!
	pub fn insert_account(&self, secret: Secret, _password: &Password) -> Result<Address, PoaError> {
/*
		let account = self.sstore.insert_account(SecretVaultRef::Root, secret, password)?;
		if self.blacklisted_accounts.contains(&account.address) {
			self.sstore.remove_account(&account, password)?;
			return Err(SSError::InvalidAccount.into());
		}
		Ok(account.address)
*/
            let mut r = [0u8; 32];
            for i in 0..20 {
                r[i] = secret.0[i];
            }
            Ok(H256(r))
	}
	/// Signs the message. If password is not provided the account must be unlocked.
	pub fn sign(&self, _address: Address, _password: Option<Password>, _message: Message) -> Result<Signature, SignError> {
            Ok(Signature([0; 64]))
/*
		let account = self.sstore.account_ref(&address)?;
		match self.unlocked_secrets.read().get(&account) {
			Some(secret) => {
				Ok(self.sstore.sign_with_secret(&secret, &message)?)
			},
			None => {
				let password = password.map(Ok).unwrap_or_else(|| self.password(&account))?;
				Ok(self.sstore.sign(&account, &password, &message)?)
			}
		}
*/
	}
	/// Creates not disk backed provider.
	pub fn transient_provider() -> Self {
		AccountProvider {
/*
			unlocked_secrets: RwLock::new(HashMap::new()),
			unlocked: RwLock::new(HashMap::new()),
			address_book: RwLock::new(AddressBook::transient()),
			sstore: Box::new(EthStore::open(Box::new(MemoryDirectory::default())).expect("MemoryDirectory load always succeeds; qed")),
			transient_sstore: transient_sstore(),
			hardware_store: None,
			unlock_keep_secret: false,
			blacklisted_accounts: vec![],
*/
		}
	}
}

/*
impl From<Arc<::extension::account_provider::AccountProvider>> for AccountProvider { 
    fn from(_x: Arc<::extension::account_provider::AccountProvider>) -> Self { 
        AccountProvider{ }
    } 
} 
*/

/// Auxiliary data fetcher for an Ethereum machine. In Ethereum-like machines
/// there are two kinds of auxiliary data: bodies and receipts.
#[derive(Default, Clone)]
pub struct AuxiliaryData<'a> {
	/// The full block bytes, including the header.
	pub bytes: Option<&'a [u8]>,
	/// The block receipts.
	pub receipts: Option<&'a [Receipt]>,
}

/// Request for auxiliary data of a block.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AuxiliaryRequest {
	/// Needs the receipts.
	Receipts,
/*
	/// Needs the body.
	Body,
	/// Needs both body and receipts.
	Both,
*/
}

/// An ethereum-like state machine.
pub struct EthereumMachine {
//	builtins: Arc<BTreeMap<Address, Builtin>>,
//	params: CommonParams,
//	tx_filter: Option<Arc<TransactionFilter>>,
/*
	ethash_extensions: Option<EthashExtensions>,
	schedule_rules: Option<Box<ScheduleCreationRules>>,
*/
}

impl Debug for EthereumMachine {
    fn fmt(&self, f: &mut Formatter) -> Result<(), std::fmt::Error> {
        write!(f, "")
//        write!(f, "params: {:?}\n", self.params)
    }
}

//impl From<::extension::machine::EthereumMachine> for EthereumMachine {
//    fn from (mut x: ::extension::machine::EthereumMachine) -> Self {
//        let mut map: BTreeMap<Address, Builtin> = BTreeMap::new();
/*
        let k: Vec<ethereum_types::Address> = x.builtins.keys().map(|k| k.clone()).collect();
        for key in k {
            let v = x.builtins.remove(&key).unwrap();
            map.insert(key.into(), v);
        }
*/
//        EthereumMachine { 
//            builtins: Arc::new(map),
//            params: (*x.params()).clone(),
//            tx_filter: None
//        }
//    }
//}

//impl<'a> Into<::extension::machine::EthereumMachine> for &'a EthereumMachine {
//    fn into(self) -> ::extension::machine::EthereumMachine {
//        let mut map: BTreeMap<ethereum_types::Address, Builtin> = BTreeMap::new();
/*
        for (k, v) in self.builtins.iter() {
            map.insert(k.into(), *v.clone());
        }
*/
//        ::extension::machine::EthereumMachine { 
//            builtins: Arc::new(map),
//            ethash_extensions: None,
//            params: self.params.clone(),
//            schedule_rules: None,
//            tx_filter: None
//        }
//    }
//}

/// Parity tries to round block.gas_limit to multiple of this constant
pub const PARITY_GAS_LIMIT_DETERMINANT: U256 = U256([37, 0, 0, 0]);

#[allow(dead_code)]
// Try to round gas_limit a bit so that:
// 1) it will still be in desired range
// 2) it will be a nearest (with tendency to increase) multiple of PARITY_GAS_LIMIT_DETERMINANT
fn round_block_gas_limit(gas_limit: U256, lower_limit: U256, upper_limit: U256) -> U256 {
	let increased_gas_limit = gas_limit.clone() + (PARITY_GAS_LIMIT_DETERMINANT /*- gas_limit % PARITY_GAS_LIMIT_DETERMINANT*/);
	if increased_gas_limit > upper_limit {
		let decreased_gas_limit = increased_gas_limit - PARITY_GAS_LIMIT_DETERMINANT;
		if decreased_gas_limit < lower_limit {
			gas_limit
		} else {
			decreased_gas_limit
		}
	} else {
		increased_gas_limit
	}
}

/// Represents what has to be handled by actor listening to chain events
pub trait ChainNotify : Send + Sync {
	/// fires when chain broadcasts a message
	fn broadcast(&self, _message_type: ChainMessageType) {
		// does nothing by default
	}
}

pub struct Client {
	engine: Arc<dyn EthEngine>,
}

impl Client {
    pub fn new(engine: Arc<dyn EthEngine>) -> Client {
        Client { engine }
    }
	/// Adds an actor to be notified on certain events
	pub fn add_notify(&self, _target: Arc<dyn ChainNotify>) {
//		self.notify.write().push(Arc::downgrade(&target));
	}
//	fn best_block_header(&self) -> Header {
//		self.chain.read().best_block_header()
//	}
	/// Returns engine reference.
	pub fn engine(&self) -> &dyn EthEngine {
		&*self.engine
	}
}

impl CallContract for Client {
	fn call_contract(&self, _block_id: BlockId, _address: Address, data: Bytes) -> Result<Bytes, String> {
/*     
		let state_pruned = || CallError::StatePruned.to_string();
		let state = &mut self.state_at(block_id).ok_or_else(&state_pruned)?;
		let header = self.block_header_decoded(block_id).ok_or_else(&state_pruned)?;

		let transaction = self.contract_call_tx(block_id, address, data);

		self.call(&transaction, Default::default(), state, &header)
			.map_err(|e| format!("{:?}", e))
			.map(|executed| executed.output)
*/
            Ok(data)
	}
}

impl BlockChainClient for Client {
	fn transact_contract(&self, _address: Address, _data: Bytes) -> Result<(), PoaError> {
/*
		let authoring_params = self.importer.miner.authoring_params();
		let transaction = Transaction {
			nonce: self.latest_nonce(&authoring_params.author),
			action: Action::Call(address),
			gas: self.importer.miner.sensible_gas_limit(),
			gas_price: self.importer.miner.sensible_gas_price(),
			value: U256::zero(),
			data: data,
		};
		let chain_id = self.engine.signing_chain_id(&self.latest_env_info());
		let signature = self.engine.sign(transaction.hash(chain_id).into())
			.map_err(|e| transaction::Error::InvalidSignature(e.to_string()))?;
		let signed = SignedTransaction::new(transaction.with_signature(signature.into(), chain_id))?;
		self.importer.miner.import_own_transaction(self, signed.into())
*/
            Ok(())
	}
}

impl EngineClient for Client {
	fn as_full_client(&self) -> Option<&dyn BlockChainClient> { 
            Some(self) 
        }
	fn block_header(&self, _id: BlockId) -> Option<OrdinaryHeader> {
		None
	}
	fn broadcast_consensus_message(&self, _message: Bytes) {
	}
    fn epoch_transition_for(&self, _parent_hash: H256) -> Option<EpochTransition> {
        Some(EpochTransition {
            block_hash: H256::from(1u8),
            block_number: 1,
            proof: vec![0, 1, 2, 3]
        })
    }
	fn update_sealing(&self) {
	}
	fn block_number(&self, _id: BlockId) -> Option<BlockNumber> {
		None
	}
/*
	fn submit_seal(&self, block_hash: H256, seal: Vec<Bytes>) {
		unimplemented!()
	}
*/
}

impl EthereumMachine {

    /// Empty VM
    pub fn new() -> EthereumMachine {
//        let params = CommonParams::default();
//        let filter = TransactionFilter::from_params(&params).map(Arc::new);
        EthereumMachine {
//            params: params,
//            builtins: Arc::new(BTreeMap::new()),
//            tx_filter: filter,
        }
    }

	/// The nonce with which accounts begin at given block.
	pub fn account_start_nonce(&self, block: u64) -> U256 {
/*
		let params = self.params();

		if block >= params.dust_protection_transition {
                        let t: u128 = params.nonce_cap_increment as u128 * block as u128;
                        U256::from(t)
//			U256::from(params.nonce_cap_increment) * U256::from(block)
		} else {
			params.account_start_nonce.into()
		}
*/
            U256::from(block)
	}

	/// Additional params.
	pub fn additional_params(&self) -> HashMap<String, String> {
/*
		hash_map![
			"registrar".to_owned() => format!("{:x}", self.params.registrar)
		]
*/  
            HashMap::new()
	}

/*
	/// Attempt to get a handle to a built-in contract.
	/// Only returns references to activated built-ins.
	// TODO: builtin contract routing - to do this properly, it will require removing the built-in configuration-reading logic
	// from Spec into here and removing the Spec::builtins field.
	pub fn builtin(&self, a: &Address, block_number: BlockNumber) -> Option<&Builtin> {
		self.builtins()
			.get(a)
			.and_then(|b| if b.is_active(block_number) { Some(b) } else { None })
	}

	/// Builtin-contracts for the chain..
	pub fn builtins(&self) -> &BTreeMap<Address, Builtin> {
		&*self.builtins
	}
*/

/*
	/// Returns new contract address generation scheme at given block number.
	pub fn create_address_scheme(&self, _number: BlockNumber) -> CreateContractAddress {
		CreateContractAddress::FromSenderAndNonce
	}
*/

/*
	/// Performs pre-validation of RLP decoded transaction before other processing
	pub fn decode_transaction(&self, transaction: &[u8]) -> Result<UnverifiedTransaction, ::transaction::Error> {
		let rlp = Rlp::new(&transaction);
		if rlp.as_raw().len() > self.params().max_transaction_size {
			debug!("Rejected oversized transaction of {} bytes", rlp.as_raw().len());
			return Err(::transaction::Error::TooBig)
		}
		rlp.as_val().map_err(|e| ::transaction::Error::InvalidRlp(e.to_string()))
	}

*/

//	pub fn ethash_extensions(&self) -> Option<::extension::machine::EthashExtensions> {
//            None
//        }

	/// Execute a call as the system address. Block environment information passed to the
	/// VM is modified to have its gas limit bounded at the upper limit of possible used
	/// gases including this system call, capped at the maximum value able to be
	/// represented by U256. This system call modifies the block state, but discards other
	/// information. If suicides, logs or refunds happen within the system call, they
	/// will not be executed or recorded. Gas used by this system call will not be counted
	/// on the block.
	pub fn execute_as_system(
		&self,
		_block: &mut ExecutedBlock,
		_contract_address: Address,
		_gas: U256,
		_data: Option<Vec<u8>>,
	) -> Result<Vec<u8>, PoaError> {
/*
		let (code, code_hash) = {
			let state = block.state();

			(state.code(&contract_address.clone().into())?,
			 state.code_hash(&contract_address.clone().into())?.map(|h| H256::from(h)))
		};

		self.execute_code_as_system(
			block,
			Some(contract_address),
			code,
			code_hash.into(),
			None,
			gas,
			data,
			None,
		)
*/
            Ok(Vec::new())
	}

	/// Same as execute_as_system, but execute code directly. If contract address is None, use the null sender
	/// address. If code is None, then this function has no effect. The call is executed without finalization, and does
	/// not form a transaction.
	pub fn execute_code_as_system(
		&self,
		_block: &mut ExecutedBlock,
		_contract_address: Option<Address>,
		_code: Option<Arc<Vec<u8>>>,
		_code_hash: Option<H256>,
		_value: Option<ActionValue>,
		_gas: U256,
		_data: Option<Vec<u8>>,
		_call_type: Option<CallType>,
	) -> Result<Vec<u8>, PoaError> {
/*                 
		let env_info = {
			let mut env_info = block.env_info();
			env_info.gas_limit = env_info.gas_used.saturating_add(gas.clone().into());
			env_info
		};

                let mut state = block.state_mut();
                let value: Option<vm::ActionValue> = match value {
                    Some(ActionValue::Apparent(x)) => Some(vm::ActionValue::Apparent(x.into())),
                    Some(ActionValue::Transfer(x)) => Some(vm::ActionValue::Transfer(x.into())),
                    None => None
                };
                let code_hash: Option<ethereum_types::H256> = code_hash.map(|h| h.into());

		let params = ActionParams {
			code_address: contract_address.clone().unwrap_or(UNSIGNED_SENDER.into()).into(),
			address: contract_address.unwrap_or(UNSIGNED_SENDER.into()).into(),
			sender: SYSTEM_ADDRESS,
			origin: SYSTEM_ADDRESS,
			gas: gas.into(),
			gas_price: 0.into(),
			value: value.unwrap_or(vm::ActionValue::Transfer(0.into())),
			code,
			code_hash,
			data,
			call_type: call_type.unwrap_or(CallType::Call),
			params_type: ParamsType::Separate,
		};

		let schedule = self.schedule(env_info.number);

                let t: ::extension::machine::EthereumMachine = self.into();
		let mut ex = Executive::new(&mut state, &env_info, &t, &schedule);
		let mut substate = Substate::new();

		let res = ex.call(params, &mut substate, &mut NoopTracer, &mut NoopVMTracer).map_err(|e| ::extension::engines::EngineError::FailedSystemCall(format!("{}", e)))?;
		let output = res.return_data.to_vec();

		Ok(output)
*/
            Ok(Vec::new())
	}

	/// Some intrinsic operation parameters; by default they take their value from the `spec()`'s `engine_p	arams`.
//	pub fn maximum_extra_data_size(&self) -> usize { self.params().maximum_extra_data_size }

	/// Logic to perform on a new block: updating last hashes and the DAO
	/// fork, for ethash.
	pub fn on_new_block(&self, block: &mut ExecutedBlock) -> Result<(), PoaError> {
		self.push_last_hash(block)?;
/*
		if let Some(ref ethash_params) = self.ethash_extensions {
			if block.header().number() == ethash_params.dao_hardfork_transition {
				let state = block.state_mut();
				for child in &ethash_params.dao_hardfork_accounts {
					let beneficiary = &ethash_params.dao_hardfork_beneficiary;
					state.balance(child)
						.and_then(|b| state.transfer_balance(child, beneficiary, &b, CleanupMode::NoEmpty))?;
				}
			}
		}
*/
		Ok(())
	}

/*
	/// Get the general parameters of the chain.
	pub fn params(&self) -> &CommonParams {
		&self.params
	}
*/

	/// Populate a header's fields based on its parent's header.
	/// Usually implements the chain scoring rule based on weight.
	/// The gas floor target must not be lower than the engine's minimum gas limit.
	pub fn populate_from_parent(&self, header: &mut OrdinaryHeader, parent: &OrdinaryHeader, _gas_floor_target: U256, _gas_ceil_target: U256) {
		header.set_difficulty(parent.difficulty().clone());
		let gas_limit = parent.gas_limit().clone();
		assert!(!gas_limit.is_zero(), "Gas limit should be > 0");
/*
		if let Some(ref ethash_params) = self.ethash_extensions {
			let gas_limit = {
				let bound_divisor = self.params().gas_limit_bound_divisor;
				let lower_limit = gas_limit - gas_limit / bound_divisor + 1;
				let upper_limit = gas_limit + gas_limit / bound_divisor - 1;
				let gas_limit = if gas_limit < gas_floor_target {
					let gas_limit = cmp::min(gas_floor_target, upper_limit);
					round_block_gas_limit(gas_limit, lower_limit, upper_limit)
				} else if gas_limit > gas_ceil_target {
					let gas_limit = cmp::max(gas_ceil_target, lower_limit);
					round_block_gas_limit(gas_limit, lower_limit, upper_limit)
				} else {
					let total_lower_limit = cmp::max(lower_limit, gas_floor_target);
					let total_upper_limit = cmp::min(upper_limit, gas_ceil_target);
					let gas_limit = cmp::max(gas_floor_target, cmp::min(total_upper_limit,
						lower_limit + (header.gas_used().clone() * 6u32 / 5) / bound_divisor));
					round_block_gas_limit(gas_limit, total_lower_limit, total_upper_limit)
				};
				// ensure that we are not violating protocol limits
				debug_assert!(gas_limit >= lower_limit);
				debug_assert!(gas_limit <= upper_limit);
				gas_limit
			};

			header.set_gas_limit(gas_limit);
			if header.number() >= ethash_params.dao_hardfork_transition &&
				header.number() <= ethash_params.dao_hardfork_transition + 9 {
				header.set_extra_data(b"dao-hard-fork"[..].to_owned());
			}
			return
		}
*/
//		header.set_gas_limit({
//			let bound_divisor = self.params().gas_limit_bound_divisor;
//			if gas_limit < gas_floor_target {
//				cmp::min(gas_floor_target, gas_limit /*+ gas_limit / bound_divisor*/ - U256::from(1u64))
//			} else {
//				cmp::max(gas_floor_target, gas_limit /*- gas_limit / bound_divisor*/ + U256::from(1u64))
//			}
//		});
	}

	/// Push last known block hash to the state.
	fn push_last_hash(&self, _block: &mut ExecutedBlock) -> Result<(), PoaError> {
/*
		let params = self.params();
		if block.header().number() == params.eip210_transition {
			let state = block.state_mut();
			state.init_code(&params.eip210_contract_address, params.eip210_contract_code.clone())?;
		}
		if block.header().number() >= params.eip210_transition {
			let parent_hash = block.header().parent_hash().clone();
			let _ = self.execute_as_system(
				block,
				params.eip210_contract_address.into(),
				params.eip210_contract_gas.into(),
				Some(parent_hash.to_vec()),
			)?;
		}
*/
		Ok(())
	}

	/// Regular ethereum machine.
	pub fn regular(/*params: CommonParams, builtins: BTreeMap<Address, Builtin>*/) -> EthereumMachine {
//		let tx_filter = TransactionFilter::from_params(&params).map(Arc::new);
		EthereumMachine {
//			params: params,
//			builtins: Arc::new(builtins),
//			tx_filter: tx_filter,
//			ethash_extensions: None,
//schedule_rules: None,
		}
	}

/*
	/// Get the EVM schedule for the given block number.
	pub fn schedule(&self, block_number: BlockNumber) -> Schedule {
		let mut schedule = match self.ethash_extensions {
			None => self.params.schedule(block_number),
			Some(ref ext) => {
				if block_number < ext.homestead_transition {
					Schedule::new_frontier()
				} else {
					self.params.schedule(block_number)
				}
			}
		};
		if let Some(ref rules) = self.schedule_rules {
			(rules)(&mut schedule, block_number)
		}
		schedule
	}
*/

/*
	/// The network ID that transactions should be signed with.
	pub fn signing_chain_id(&self, env_info: &EnvInfo) -> Option<u64> {
		let params = self.params();

		if env_info.number >= params.eip155_transition {
			Some(params.chain_id)
		} else {
			None
		}
	}
*/

	/// Does verification of the transaction against the parent state.
	pub fn verify_transaction<C/*: BlockInfo + CallContract*/>(&self, _t: &SignedTransaction, _parent: &OrdinaryHeader, _client: &C)
		-> Result<(), PoaError>
	{
/*
		if let Some(ref filter) = self.tx_filter.as_ref() {
			if !filter.transaction_allowed(&parent.hash().into(), parent.number() + 1, t, client) {
				return Err(transaction::Error::NotAllowed.into())
			}
		}
*/

		Ok(())
	}

	/// Does basic verification of the transaction.
	pub fn verify_transaction_basic(&self, t: &UnverifiedTransaction, _header: &OrdinaryHeader) -> Result<(), PoaError> {
		let check_low_s = true;
/*
                match self.ethash_extensions {
			Some(ref ext) => header.number() >= ext.homestead_transition,
			None => true,
		};
*/
		let chain_id = None;
/*if header.number() < self.params().validate_chain_id_transition {
			t.chain_id()
		} else if header.number() >= self.params().eip155_transition {
			Some(self.params().chain_id)
		} else {
			None
		};
*/
		t.verify_basic(check_low_s, chain_id, false)?;

		Ok(())
	}

	/// Verify a particular transaction is valid, regardless of order.
	pub fn verify_transaction_unordered(&self, t: UnverifiedTransaction, _header: &OrdinaryHeader) -> Result<SignedTransaction, PoaError> {
		Ok(SignedTransaction::new(t)?)
	}

}

impl<'a> LocalizedMachine<'a> for EthereumMachine {
	type StateContext = Call<'a>;
	type AuxiliaryData = AuxiliaryData<'a>;                                                
}

impl Machine for EthereumMachine {
	type AuxiliaryRequest = AuxiliaryRequest;
	type EngineClient = dyn EngineClient;
	type Error = PoaError;
	type ExtendedHeader = ExtendedHeader;
	type Header = OrdinaryHeader;
	type LiveBlock = ExecutedBlock;
/*
	type AncestryAction = ::common_types::ancestry_action::AncestryAction;
*/
}

impl WithBalances for EthereumMachine {
	fn add_balance(&self, _live: &mut ExecutedBlock, _address: &Address, _amount: &U256) -> Result<(), PoaError> {
//		live.state_mut().add_balance(&address.into(), &amount.into(), CleanupMode::NoEmpty).map_err(Into::into)
            Ok(())
	}
/*
	fn balance(&self, live: &ExecutedBlock, address: &Address) -> Result<U256, Error> {
		live.state().balance(address).map_err(Into::into)
	}

*/
}

impl WithRewards for EthereumMachine {
	fn note_rewards(
		&self,
		_live: &mut Self::LiveBlock,
		_rewards: &[(Address, RewardKind, U256)],
	) -> Result<(), Self::Error> {
/*
		if let Tracing::Enabled(ref mut traces) = *live.traces_mut() {
			let mut tracer = ExecutiveTracer::default();

			for &(ref address, ref reward_type, ref amount) in rewards {
				tracer.trace_reward(address.into(), amount.into(), reward_type.clone());
			}

			traces.push(tracer.drain().into());
		}
*/
		Ok(())
	}
}


/// An internal type for a block's common elements.
#[derive(Clone)]
pub struct ExecutedBlock {
	/// Executed block header.
	pub header: OrdinaryHeader,
	/// Hashes of last 256 blocks.
	pub last_hashes: Arc<LastHashes>,
	/// Transaction receipts.
	pub receipts: Vec<Receipt>,
	/// Underlaying state.
//	pub state: State<StateDB>,
	/// Transaction traces.
//	pub traces: Tracing,
	/// Executed transactions.
	pub transactions: Vec<SignedTransaction>,
	/// Hashes of already executed transactions.
	pub transactions_set: HashSet<H256>,
	/// Uncles.
	pub uncles: Vec<OrdinaryHeader>,
}

impl ExecutedBlock {

	/// Create a new block from the given `state`.
	pub fn new(/*state: State<StateDB>,*/ last_hashes: Arc<LastHashes>, /*tracing: bool*/) -> ExecutedBlock {
		ExecutedBlock {
			header: Default::default(),
			last_hashes: last_hashes,
			receipts: Default::default(),
//			state: state,
/*
			traces: if tracing {
				Tracing::enabled()
			} else {
				Tracing::Disabled
			},
*/
			transactions: Default::default(),
			transactions_set: Default::default(),
			uncles: Default::default(),
		}
	}

/*
	/// Get the environment info concerning this block.
	pub fn env_info(&self) -> EnvInfo {
		// TODO: memoise.
		EnvInfo {
			number: self.header.number(),
			author: self.header.author().clone().into(),
			timestamp: self.header.timestamp(),
			difficulty: self.header.difficulty().clone().into(),
			last_hashes: self.last_hashes.clone(),
			gas_used: self.receipts.last().map_or(ethereum_types::U256::zero(), |r| r.gas_used.into()),
			gas_limit: self.header.gas_limit().clone().into(),
		}
	}
*/

/*
	/// Get mutable access to a state.
	pub fn state_mut(&mut self) -> &mut State<StateDB> {
		&mut self.state
	}

	/// Get mutable reference to traces.
	pub fn traces_mut(&mut self) -> &mut Tracing {
		&mut self.traces
	}
*/

}

impl LiveBlock for ExecutedBlock {
	type Header = OrdinaryHeader;
	fn header(&self) -> &OrdinaryHeader {
		&self.header
	}
	fn uncles(&self) -> &[OrdinaryHeader] {
		&self.uncles
	}
}

impl IsBlock for ExecutedBlock {
	fn block(&self) -> &ExecutedBlock { self }
}

impl Transactions for ExecutedBlock {
	type Transaction = SignedTransaction;
	fn transactions(&self) -> &[SignedTransaction] {
		&self.transactions
	}
}

/// Extended block header, wrapping `Header` with finalized and total difficulty information.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtendedHeader {
	/// The actual header.
	pub header: OrdinaryHeader,
	/// Whether the block underlying this header is considered finalized.
	pub is_finalized: bool,
	/// The parent block difficulty.
	pub parent_total_difficulty: U256,
}

impl Header for ExtendedHeader {
	fn author(&self) -> &Address { self.header.author() }
	fn hash(&self) -> H256 { self.header.hash() }
	fn number(&self) -> BlockNumber { self.header.number() }
/*
	fn bare_hash(&self) -> H256 { self.header.bare_hash() }
	fn seal(&self) -> &[Vec<u8>] { self.header.seal() }
*/
}

impl TotalScoredHeader for ExtendedHeader {
	type Value = U256;
	fn total_score(&self) -> U256 { self.parent_total_difficulty.clone() + self.header.difficulty().clone() }
}

/// Just like `ClosedBlock` except that we can't reopen it and it's faster.
///
/// We actually store the post-`Engine::on_close_block` state, unlike in `ClosedBlock` where it's the pre.
#[derive(Clone)]
pub struct LockedBlock {
	block: ExecutedBlock,
}

impl LockedBlock {

	/// Provide a valid seal in order to turn this into a `SealedBlock`.
	/// This does check the validity of `seal` with the engine.
	/// Returns the `ClosedBlock` back again if the seal is no good.
	pub fn try_seal(
		self,
		engine: &dyn EthEngine,
		seal: Vec<Bytes>,
	) -> Result<SealedBlock, PoaError> {
		let mut s = self;
		s.block.header.set_seal(seal);
		s.block.header.compute_hash();
		// TODO: passing state context to avoid engines owning it?
		engine.verify_local_seal(&s.block.header)?;
                let mut block_rlp = RlpStream::new_list(1);
                block_rlp.append(&s.block.header);
		Ok(SealedBlock {
			block: block_rlp.out(), //s.block
                        empty_steps: Vec::new(),
                        signature: Vec::new()
		})
	}

	/// Provide a valid seal in order to turn this into a `SealedBlock`.
	///
	/// NOTE: This does not check the validity of `seal` with the engine.
	pub fn seal(self, _engine: &dyn EthEngine, seal: Vec<Bytes>) -> Result<SealedBlock, PoaError> {
//		let expected_seal_fields = engine.seal_fields(self.header());
		let mut s = self;
/*
		if seal.len() != expected_seal_fields {
			return Err(BlockError::InvalidSealArity(
				Mismatch { expected: expected_seal_fields, found: seal.len() }));
		}
*/
		s.block.header.set_seal(seal);
		s.block.header.compute_hash();
                let mut block_rlp = RlpStream::new_list(1);
                block_rlp.append(&s.block.header);
		Ok(SealedBlock {
			block: block_rlp.out(), //s.block,
                        empty_steps: Vec::new(),
                        signature: Vec::new()
		})
	}

}

impl IsBlock for LockedBlock {
	fn block(&self) -> &ExecutedBlock { &self.block }
}

#[derive(Clone, Debug, Default)]
pub struct UnsignedBlock {
    pub block: Block,
//    pub key: Arc<Keypair>,
    pub keyid: u64
}

impl Eq for UnsignedBlock {}

impl PartialEq for UnsignedBlock {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        self.block.eq(&other.block) 
        //&& 
        //self.key.to_bytes().iter().zip(other.key.to_bytes().iter()).all(|(a, b)| a == b)
    }
}

/// A block that has a valid seal.
///
/// The block's header has valid seal arguments. The block cannot be reversed into a `ClosedBlock` or `OpenBlock`.
//#[derive(Clone, Debug)]
pub struct SealedBlock {
    block: Vec<u8>,//ExecutedBlock,
    empty_steps: Vec<u8>,
    signature: Vec<u8>
}

/*
impl Eq for SealedBlock {}

impl PartialEq for SealedBlock {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        self.block.eq(&other.block) && 
        self.empty_steps.eq(&other.empty_steps) &&
        self.signature.eq(&other.signature)
    }
}
*/

impl SealedBlock {
    /// Get the RLP-encoding of the block.
    pub fn rlp_bytes(&self) -> Bytes {
        let mut block_rlp = RlpStream::new_list(3);
        block_rlp.append(&self.block);
        block_rlp.append(&self.empty_steps);
        block_rlp.append(&self.signature);
/*
        block_rlp.append(&self.block.header);
        block_rlp.append_list(&self.block.transactions);
        block_rlp.append_list(&self.block.uncles); */
        block_rlp.out()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TONBlock {
    Signed(SignedBlock),
    Unsigned(UnsignedBlock),
}

impl TONBlock {
    pub fn is_empty(&self) -> bool {
        match self {
            TONBlock::Signed(block) => block.block().read_extra().unwrap().read_in_msg_descr().unwrap().is_empty(),
            TONBlock::Unsigned(block) => block.block.read_extra().unwrap().read_in_msg_descr().unwrap().is_empty(),
        }
    }
    pub fn get_seq_no(&self) -> u32 {
        match self {
            TONBlock::Signed(block) => block.block().read_info().unwrap().seq_no(),
            TONBlock::Unsigned(block) => block.block.read_info().unwrap().seq_no(),
        }
    }
    pub fn get_gen_time(&self) -> u32 {
        match self {
            TONBlock::Signed(block) => block.block().read_info().unwrap().gen_utime().0,
            TONBlock::Unsigned(block) => block.block.read_info().unwrap().gen_utime().0,
        }
    }
}

/// A block header.
#[derive(Clone, Debug, Eq)]
pub struct OrdinaryHeader {
	/// Block author.
	author: Address,
	/// Block difficulty.
	difficulty: U256,
	/// Block extra data.
	extra_data: Bytes,
	/// Block gas limit.
	gas_limit: U256,
	/// Gas used for contracts execution.
	gas_used: U256,
	/// Memoized hash of that header and the seal.
	hash: Option<H256>,
	/// Block bloom.
//	log_bloom: Bloom,
	/// Block number.
	number: BlockNumber,
	/// Parent hash.
	parent_hash: H256,
	/// Block receipts root.
	receipts_root: H256,
	/// Vector of post-RLP-encoded fields.
	seal: Vec<Bytes>,
	/// State root.
	state_root: H256,
	/// Block timestamp.
	timestamp: u64,
	/// Transactions root.
	transactions_root: H256,
	/// Block uncles hash.
	uncles_hash: H256,

	pub ton_block: TONBlock,
	pub ton_bytes: Vec<u8>,
	pub ton_offset: usize
}

/// Alter value of given field, reset memoised hash if changed.
fn change_field<T>(hash: &mut Option<H256>, field: &mut T, value: T) where T: PartialEq<T> {
	if field != &value {
		*field = value;
		*hash = None;
	}
}

impl OrdinaryHeader {
	/// Create a new, default-valued, header.
	pub fn new() -> Self { Self::default() }
	/// Get the author field of the header.
	pub fn author(&self) -> &Address { &self.author }
	/// Get the author key field of the header.
	pub fn author_key_id(&self) -> &u64 { 
            match &self.ton_block {
                TONBlock::Signed(x) => match x.signatures().keys().last() {
                    Some(key) => key,
                    None => unimplemented!()
                },
                TONBlock::Unsigned(x) => &x.keyid
            }
        }
	/// Get the hash of the header excluding the seal
	pub fn bare_hash(&self) -> H256 {
		//keccak(self.rlp(Seal::Without)).into()
		let block_hash = match self.ton_block {
			TONBlock::Signed(ref block) => {
				debug!(target: "engine", "BARE_HASH: signed block");
				UInt256::from(block.block_hash().clone())
			},
			TONBlock::Unsigned(ref block) => {
				debug!(target: "engine", "BARE_HASH: unsigned block");
                block.block.hash().unwrap()
			},
		};
		debug!(target: "engine", "hash: {:?}", block_hash);
		H256::from_slice(block_hash.as_slice())
	}
	/// Get & memoize the hash of this header (keccak of the RLP with seal).
	pub fn compute_hash(&mut self) -> H256 {
		debug!(target: "engine", "ORDINARY HEADER CALC HASH:");
		let hash = self.hash();
		self.hash = Some(hash.clone());
		debug!(target: "engine", "hash: {:?}", hash);
		hash
	}
	/// Get the difficulty field of the header.
	pub fn difficulty(&self) -> &U256 { &self.difficulty }
	/// Get the gas limit field of the header.
	pub fn gas_limit(&self) -> &U256 { &self.gas_limit }
	/// Get the hash of this header (keccak of the RLP with seal).
	pub fn hash(&self) -> H256 {
		debug!(target: "engine", "ORDINARY HEADER HASH:");
		let h = self.hash.clone().unwrap_or_else(|| keccak(self.rlp(Seal::With)).into());
		debug!(target: "engine", "hash: {:?}", h);
		h
	}
	/// Get the log bloom field of the header.
//	pub fn log_bloom(&self) -> &Bloom { &self.log_bloom }
	/// Get the number field of the header.
	pub fn number(&self) -> BlockNumber { self.number }
	/// Get the parent_hash field of the header.
	pub fn parent_hash(&self) -> &H256 { &self.parent_hash }
	/// Get the receipts root field of the header.
	pub fn receipts_root(&self) -> &H256 { &self.receipts_root }
	/// Get the RLP representation of this Header.
	fn rlp(&self, with_seal: Seal) -> Bytes {
		let mut s = RlpStream::new();
		self.stream_rlp(&mut s, with_seal);
		s.out()
	}
	/// Get the seal field of the header.
	pub fn seal(&self) -> &[Bytes] { &self.seal }
	/// Set the author field of the header.
	pub fn set_author(&mut self, a: Address) {
		change_field(&mut self.hash, &mut self.author, a);
	}
	/// Set the difficulty field of the header.
	pub fn set_difficulty(&mut self, a: U256) {
		change_field(&mut self.hash, &mut self.difficulty, a);
	}
	/// Set the extra data field of the header.
	pub fn set_extra_data(&mut self, a: Bytes) {
		change_field(&mut self.hash, &mut self.extra_data, a);
	}
	/// Set the gas limit field of the header.
	pub fn set_gas_limit(&mut self, a: U256) {
		change_field(&mut self.hash, &mut self.gas_limit, a);
	}
	/// Set the gas used field of the header.
	pub fn set_gas_used(&mut self, a: U256) {
		change_field(&mut self.hash, &mut self.gas_used, a);
	}
	/// Set the log bloom field of the header.
//	pub fn set_log_bloom(&mut self, a: Bloom) {
//		change_field(&mut self.hash, &mut self.log_bloom, a);
//	}
	/// Set the number field of the header.
	pub fn set_number(&mut self, a: BlockNumber) {
		change_field(&mut self.hash, &mut self.number, a);
	}
	/// Set the number field of the header.
	pub fn set_parent_hash(&mut self, a: H256) {
		change_field(&mut self.hash, &mut self.parent_hash, a);
	}
	/// Set the receipts root field of the header.
	pub fn set_receipts_root(&mut self, a: H256) {
		change_field(&mut self.hash, &mut self.receipts_root, a);
	}
	/// Set the seal field of the header.
	pub fn set_seal(&mut self, a: Vec<Bytes>) {
		change_field(&mut self.hash, &mut self.seal, a)
	}
	/// Set the state root field of the header.
	pub fn set_state_root(&mut self, a: H256) {
		change_field(&mut self.hash, &mut self.state_root, a);
	}
	/// Set the timestamp field of the header.
	pub fn set_timestamp(&mut self, a: u64) {
		change_field(&mut self.hash, &mut self.timestamp, a);
	}
	/// Set the transactions root field of the header.
	pub fn set_transactions_root(&mut self, a: H256) {
		change_field(&mut self.hash, &mut self.transactions_root, a);
	}
	/// Set the uncles hash field of the header.
	pub fn set_uncles_hash(&mut self, a: H256) {
		change_field(&mut self.hash, &mut self.uncles_hash, a);

	}
	/// Get the state root field of the header.
	pub fn state_root(&self) -> &H256 { &self.state_root }
	/// Place this header into an RLP stream `s`, optionally `with_seal`.
	fn stream_rlp(&self, s: &mut RlpStream, with_seal: Seal) {
		if let Seal::With = with_seal {
			s.begin_list(13 + self.seal.len());
		} else {
			s.begin_list(13);
		}

		s.append(&self.parent_hash);
		s.append(&self.uncles_hash);
		s.append(&self.author);
		s.append(&self.state_root);
		s.append(&self.transactions_root);
		s.append(&self.receipts_root);
		s.append(&self.receipts_root); // instead of bloom
//		s.append(&self.log_bloom);
		s.append(&self.difficulty);
		s.append(&self.number);
		s.append(&self.gas_limit);
		s.append(&self.gas_used);
		s.append(&self.timestamp);
		s.append(&self.extra_data);

		if let Seal::With = with_seal {
			for b in &self.seal {
				s.append_raw(b, 1);
			}
		}
	}
	/// Get the timestamp field of the header.
	pub fn timestamp(&self) -> u64 { self.timestamp }
	/// Get the uncles hash field of the header.
	pub fn uncles_hash(&self) -> &H256 { &self.uncles_hash }
}

impl Header for OrdinaryHeader {
	fn author(&self) -> &Address { OrdinaryHeader::author(self) }
	fn hash(&self) -> H256 { OrdinaryHeader::hash(self) }
	fn number(&self) -> BlockNumber { OrdinaryHeader::number(self) }
/*
	fn bare_hash(&self) -> H256 { Header::bare_hash(self) }
	fn seal(&self) -> &[Vec<u8>] { Header::seal(self) }
*/
}

impl PartialEq for OrdinaryHeader {
	fn eq(&self, c: &OrdinaryHeader) -> bool {
		if let (&Some(ref h1), &Some(ref h2)) = (&self.hash, &c.hash) {
			if h1 == h2 {
				return true
			}
		}

		self.parent_hash == c.parent_hash &&
		self.timestamp == c.timestamp &&
		self.number == c.number &&
		self.author == c.author &&
		self.transactions_root == c.transactions_root &&
		self.uncles_hash == c.uncles_hash &&
		self.extra_data == c.extra_data &&
		self.state_root == c.state_root &&
		self.receipts_root == c.receipts_root &&
//		self.log_bloom == c.log_bloom &&
			self.gas_used == c.gas_used &&
		self.gas_limit == c.gas_limit &&
		self.difficulty == c.difficulty &&
		self.seal == c.seal
	}
}

impl Default for OrdinaryHeader {
	fn default() -> Self {
		OrdinaryHeader {
			author: Address::default(),
			difficulty: U256::default(),
			extra_data: vec![],
			gas_limit: U256::default(),
			gas_used: U256::default(),
			hash: None,
//			log_bloom: Bloom::default(),
			number: 0,
			parent_hash: H256::default(),
			receipts_root: KECCAK_NULL_RLP.into(),
			seal: vec![],
			state_root: KECCAK_NULL_RLP.into(),
			timestamp: 0,
			transactions_root: KECCAK_NULL_RLP.into(),
			uncles_hash: KECCAK_EMPTY_LIST_RLP.into(),
                        ton_block: TONBlock::Unsigned(UnsignedBlock::default()),
                        ton_bytes: Vec::new(),
                        ton_offset: 0
		}
	}
}

impl Decodable for OrdinaryHeader {
	fn decode(r: &Rlp) -> Result<Self, DecoderError> {
            let hash = keccak(r.as_raw()).into();
			debug!(target: "engine", "Decode header: hash {:?}", hash);
			let mut blockheader = OrdinaryHeader { 
				parent_hash: r.val_at(0)?,
				uncles_hash: r.val_at(1)?,
				author: r.val_at(2)?,
				state_root: r.val_at(3)?,
				transactions_root: r.val_at(4)?,
				receipts_root: r.val_at(5)?,
	//			log_bloom: r.val_at(6)?,
				difficulty: r.val_at(7)?,
				number: r.val_at(8)?,
				gas_limit: r.val_at(9)?,
				gas_used: r.val_at(10)?,
				timestamp: cmp::min(r.val_at::<U256>(11)?, u64::max_value().into()).0[0],
				extra_data: r.val_at(12)?,
				seal: vec![],
				hash: Some(hash),
				ton_block: TONBlock::Unsigned(UnsignedBlock::default()),
				ton_bytes: Vec::new(),
				ton_offset: 0
            };

            for i in 13..r.item_count()? {
				blockheader.seal.push(r.at(i)?.as_raw().to_vec())
			}

            Ok(blockheader)
	}
}

impl Encodable for OrdinaryHeader {
	fn rlp_append(&self, s: &mut RlpStream) {
		self.stream_rlp(s, Seal::With);
	}
}

/// Block that is ready for transactions to be added.
///
/// It's a bit like a Vec<Transaction>, except that whenever a transaction is pushed, we execute it and
/// maintain the system `state()`. We also archive execution receipts in preparation for later block creation.
pub struct OpenBlock<'x> {
	block: ExecutedBlock,
	engine: &'x dyn EthEngine,
}

impl<'x> OpenBlock<'x> {
	/// Create a new `OpenBlock` ready for transaction pushing.
	pub fn new<'a>(
		engine: &'x dyn EthEngine,
//		factories: Factories,
		_tracing: bool,
		_db: StateDB,
		parent: &OrdinaryHeader,
		last_hashes: Arc<LastHashes>,
		author: Address,
		gas_range_target: (U256, U256),
		extra_data: Bytes,
		_is_epoch_begin: bool,
		_ancestry: &mut dyn Iterator<Item=ExtendedHeader>,
	) -> Result<Self, PoaError> {
		let number = parent.number() + 1;
//		let state = State::from_existing(db, parent.state_root().clone(), engine.account_start_nonce(number).into(), factories)?;
		let mut r = OpenBlock {
			block: ExecutedBlock::new(/*state,*/ last_hashes, /*tracing*/),
			engine: engine,
		};

		r.block.header.set_parent_hash(parent.hash().into());
		r.block.header.set_number(number);
		r.block.header.set_author(author.into());
		r.block.header.set_timestamp(engine.open_block_header_timestamp(parent.timestamp()));
		r.block.header.set_extra_data(extra_data);

		let gas_floor_target = gas_range_target.0;//cmp::max(gas_range_target.0, engine.params().min_gas_limit);
		let gas_ceil_target = gas_range_target.1;//cmp::max(gas_range_target.1, gas_floor_target);

		engine.machine().populate_from_parent(&mut r.block.header, parent, gas_floor_target.into(), gas_ceil_target.into());
		engine.populate_from_parent(&mut r.block.header, parent);

		engine.machine().on_new_block(&mut r.block)?;
//		engine.on_new_block(&mut r.block, is_epoch_begin, ancestry)?; //Always OK

		Ok(r)
	}
	/// Turn this into a `LockedBlock`.
	pub fn close_and_lock(self) -> Result<LockedBlock, PoaError> {
		let mut s = self;

		s.engine.on_close_block(&mut s.block)?;
//		s.block.state.commit()?;

//		s.block.header.set_transactions_root(ordered_trie_root(s.block.transactions.iter().map(|e| e.rlp_bytes())).into());
//		let uncle_bytes = encode_list(&s.block.uncles);
//		s.block.header.set_uncles_hash(keccak(&uncle_bytes).into());
//		s.block.header.set_state_root(s.block.state.root().clone().into());
//		s.block.header.set_receipts_root(ordered_trie_root(s.block.receipts.iter().map(|r| r.rlp_bytes())).into());
//		s.block.header.set_log_bloom(s.block.receipts.iter().fold(Bloom::zero().into(), |mut b, r| {
//			b.accrue_bloom(&r.log_bloom.clone().into());
//			b
//		}));
		s.block.header.set_gas_used(s.block.receipts.last().map_or_else(U256::zero, |r| r.gas_used.clone()).into());

		Ok(LockedBlock {
			block: s.block,
		})
	}
	/// Push a transaction into the block.
	///
	/// If valid, it will be executed, and archived together with the receipt.
	pub fn push_transaction(&mut self, t: SignedTransaction, _h: Option<H256>) -> Result<&Receipt, PoaError> {
//		if self.block.transactions_set.contains(&t.hash().into()) {
//			return Err(TransactionError::AlreadyImported.into());
//		}

//		let env_info = self.env_info();
//		let outcome = self.block.state.apply(&env_info, self.engine.machine(), &t, self.block.traces.is_enabled())?;

//		self.block.transactions_set.insert(h.unwrap_or_else(||t.hash()).into());
		self.block.transactions.push(t.into());
//		if let Tracing::Enabled(ref mut traces) = self.block.traces {
//			traces.push(outcome.trace.into());
//		}
//		self.block.receipts.push(outcome.receipt);
		Ok(self.block.receipts.last().expect("receipt just pushed; qed"))
	}
}

impl<'x> IsBlock for OpenBlock<'x> {
	fn block(&self) -> &ExecutedBlock { &self.block }
}

/// Trait for an object that owns an `ExecutedBlock`
pub trait Drain {
	/// Returns `ExecutedBlock`
	fn drain(self) -> ExecutedBlock;
}

/*
impl Drain for SealedBlock {
	fn drain(self) -> ExecutedBlock {
		self.block
	}
}
*/

/// An unverified block.
#[derive(PartialEq, Debug)]
pub struct Unverified {
	/// Unverified block header.
	pub header: OrdinaryHeader,
	/// Unverified block transactions.
	pub transactions: Vec<UnverifiedTransaction>,
	/// Unverified block uncles.
	pub uncles: Vec<OrdinaryHeader>,
	/// Raw block bytes.
	pub bytes: Bytes,
}

impl Unverified {
	/// Create an `Unverified` from raw bytes.
	pub fn from_rlp(bytes: Bytes) -> Result<Self, ::rlp::DecoderError> {
//		use rlp::Rlp;
		let (header, transactions, uncles) = {
			let rlp = Rlp::new(&bytes);
			let header = rlp.val_at(0)?;
			let transactions = rlp.list_at(1)?;
			let uncles = rlp.list_at(2)?;
			(header, transactions, uncles)
		};
		Ok(Unverified {
			header,
			transactions,
			uncles,
			bytes,
		})
	}
}

/// Wrapper for trusted rlp, which is expected to be valid, for use in views
/// When created with view!, records the file and line where it was created for debugging
pub struct ViewRlp<'a> {
	/// Wrapped Rlp, expected to be valid
	pub rlp: Rlp<'a>,
	file: &'a str,
	line: u32,
}

impl<'a, 'view> ViewRlp<'a> where 'a : 'view {
	pub fn new(bytes: &'a [u8], file: &'a str, line: u32) -> Self {
		ViewRlp {
			rlp: Rlp::new(bytes),
			file,
			line
		}
	}
	fn expect_valid_rlp<T>(&self, r: Result<T, DecoderError>) -> T {
		r.unwrap_or_else(|e| panic!(
			"View rlp is trusted and should be valid. Constructed in {} on line {}: {}",
			self.file,
			self.line,
			e
		))
	}
	/// Returns decoded value at the given index, panics not present or valid at that index
	pub fn val_at<T>(&self, index: usize) -> T where T : Decodable {
		self.expect_valid_rlp(self.rlp.val_at(index))
	}
}

/// View onto block rlp.
pub struct BlockView<'a> {
	rlp: ViewRlp<'a>
}

impl<'a> BlockView<'a> {
	pub fn new(rlp: ViewRlp<'a>) -> BlockView<'a> {
		BlockView {
			rlp: rlp
		}
	}
	/// Create new Header object from header rlp.
	pub fn header(&self) -> OrdinaryHeader {
		self.rlp.val_at(0)
	}
}

/// Database backing `BlockChain`.
pub trait BlockChainDB: Send + Sync {
/*
	/// Generic key value store.
	fn key_value(&self) -> &Arc<KeyValueDB>;

	/// Header blooms database.
	fn blooms(&self) -> &blooms_db::Database;

	/// Trace blooms database.
	fn trace_blooms(&self) -> &blooms_db::Database;

	/// Restore the DB from the given path
	fn restore(&self, new_db: &str) -> Result<(), EthcoreError> {
		// First, close the Blooms databases
		self.blooms().close()?;
		self.trace_blooms().close()?;

		// Restore the key_value DB
		self.key_value().restore(new_db)?;

		// Re-open the Blooms databases
		self.blooms().reopen()?;
		self.trace_blooms().reopen()?;
		Ok(())
	}
*/
}

/// Generic database handler. This trait contains one function `open`. When called, it opens database with a
/// predefined config.
pub trait BlockChainDBHandler: Send + Sync {
	/// Open the predefined key-value database.
	fn open(&self, path: &Path) -> io::Result<Arc<dyn BlockChainDB>>;
}

/// Structure providing fast access to blockchain data.
///
/// **Does not do input data verification.**
pub struct BlockChain {
/*
	// All locks must be captured in the order declared here.
	best_block: RwLock<BestBlock>,
	// Stores best block of the first uninterrupted sequence of blocks. `None` if there are no gaps.
	// Only updated with `insert_unordered_block`.
	best_ancient_block: RwLock<Option<BestAncientBlock>>,
	// Stores the last block of the last sequence of blocks. `None` if there are no gaps.
	// This is calculated on start and does not get updated.
	first_block: Option<H256>,

	// block cache
	block_headers: RwLock<HashMap<H256, encoded::Header>>,
	block_bodies: RwLock<HashMap<H256, encoded::Body>>,

	// extra caches
	block_details: RwLock<HashMap<H256, BlockDetails>>,
	block_hashes: RwLock<HashMap<BlockNumber, H256>>,
	transaction_addresses: RwLock<HashMap<H256, TransactionAddress>>,
	block_receipts: RwLock<HashMap<H256, BlockReceipts>>,

	db: Arc<dyn BlockChainDB>,

	cache_man: Mutex<CacheManager<CacheId>>,

	pending_best_ancient_block: RwLock<Option<Option<BestAncientBlock>>>,
	pending_best_block: RwLock<Option<BestBlock>>,
	pending_block_hashes: RwLock<HashMap<BlockNumber, H256>>,
	pending_block_details: RwLock<HashMap<H256, BlockDetails>>,
	pending_transaction_addresses: RwLock<HashMap<H256, Option<TransactionAddress>>>,
*/
}

impl BlockChain {
	/// Create new instance of blockchain from given Genesis.
	pub fn new(_config: BlockChainConfig, _genesis: &[u8], _db: Arc<dyn BlockChainDB>) -> BlockChain {
		 BlockChain {
/*
			first_block: None,
			best_block: RwLock::new(BestBlock {
				// BestBlock will be overwritten anyway.
				header: Default::default(),
				total_difficulty: Default::default(),
				block: encoded::Block::new(genesis.into()),
			}),
			best_ancient_block: RwLock::new(None),
			block_headers: RwLock::new(HashMap::new()),
			block_bodies: RwLock::new(HashMap::new()),
			block_details: RwLock::new(HashMap::new()),
			block_hashes: RwLock::new(HashMap::new()),
			transaction_addresses: RwLock::new(HashMap::new()),
			block_receipts: RwLock::new(HashMap::new()),
			db: db.clone(),
			cache_man: Mutex::new(cache_man),
			pending_best_ancient_block: RwLock::new(None),
			pending_best_block: RwLock::new(None),
			pending_block_hashes: RwLock::new(HashMap::new()),
			pending_block_details: RwLock::new(HashMap::new()),
			pending_transaction_addresses: RwLock::new(HashMap::new()),
*/
		}
        }
	/// Inserts the block into backing cache database.
	/// Expects the block to be valid and already verified.
	/// If the block is already known, does nothing.
	pub fn insert_block(&self, /*batch: &mut DBTransaction,*/ _block: EncodedBlock, _receipts: Vec<Receipt>/*, extras: ExtrasInsert*/) /*-> ImportRoute */ {
/*

let parent_hash =
{
let v = block.header_view();
		v.parent_hash()
};
		let best_hash = self.best_block_hash();


		let route = self.tree_route(best_hash, parent_hash).expect("forks are only kept when it has common ancestors; tree route from best to prospective's parent always exists; qed");


		self.insert_block_with_route(batch, block, receipts, route, extras)
*/
	}

}

/// Blockchain configuration.
//#[derive(Debug, PartialEq, Clone)]
pub struct BlockChainConfig {
	/// Preferred cache size in bytes.
	pub pref_cache_size: usize,
	/// Maximum cache size in bytes.
	pub max_cache_size: usize,
}

impl Default for BlockChainConfig {
	fn default() -> Self {
		BlockChainConfig {
			pref_cache_size: 1 << 14,
			max_cache_size: 1 << 20,
		}
	}
}

/// Owning block view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncodedBlock(Vec<u8>);

impl EncodedBlock {
	/// Create a new owning block view. The raw bytes passed in must be an rlp-encoded block.
	pub fn new(raw: Vec<u8>) -> Self { EncodedBlock(raw) }
}

/// Representation of the entire state of all accounts in the system.
///
/// `State` can work together with `StateDB` to share account cache.
///
/// Local cache contains changes made locally and changes accumulated
/// locally from previous commits. Global cache reflects the database
/// state and never contains any changes.
///
/// Cache items contains account data, or the flag that account does not exist
/// and modification state (see `AccountState`)
///
/// Account data can be in the following cache states:
/// * In global but not local - something that was queried from the database,
/// but never modified
/// * In local but not global - something that was just added (e.g. new account)
/// * In both with the same value - something that was changed to a new value,
/// but changed back to a previous block in the same block (same State instance)
/// * In both with different values - something that was overwritten with a
/// new value.
///
/// All read-only state queries check local cache/modifications first,
/// then global state cache. If data is not found in any of the caches
/// it is loaded from the DB to the local cache.
///
/// **** IMPORTANT *************************************************************
/// All the modifications to the account data must set the `Dirty` state in the
/// `AccountEntry`. This is done in `require` and `require_or_from`. So just
/// use that.
/// ****************************************************************************
///
/// Upon destruction all the local cache data propagated into the global cache.
/// Propagated items might be rejected if current state is non-canonical.
///
/// State checkpointing.
///
/// A new checkpoint can be created with `checkpoint()`. checkpoints can be
/// created in a hierarchy.
/// When a checkpoint is active all changes are applied directly into
/// `cache` and the original value is copied into an active checkpoint.
/// Reverting a checkpoint with `revert_to_checkpoint` involves copying
/// original values from the latest checkpoint back into `cache`. The code
/// takes care not to overwrite cached storage while doing that.
/// checkpoint can be discarded with `discard_checkpoint`. All of the orignal
/// backed-up values are moved into a parent checkpoint (if any).
///
#[allow(dead_code)]
pub struct State<B> {
	db: B,
	root: H256,
/*
	cache: RefCell<HashMap<Address, AccountEntry>>,
	// The original account is preserved in
	checkpoints: RefCell<Vec<HashMap<Address, Option<AccountEntry>>>>,
	account_start_nonce: U256,
	factories: Factories,
*/
}

impl<B> State<B> {
	/// Creates new state with empty state root
	/// Used for tests.
	pub fn new(db: B, _account_start_nonce: U256/*, factories: Factories*/) -> State<B> {
		let root = H256([0; 32]);
/*
		{
			// init trie and reset root to null
			let _ = factories.trie.create(db.as_hashdb_mut(), &mut root);
		}
*/
		State {
			db: db,
			root: root,
/*
			cache: RefCell::new(HashMap::new()),
			checkpoints: RefCell::new(Vec::new()),
			account_start_nonce: account_start_nonce,
			factories: factories,
*/
		}
	}
}

/// State database abstraction.
/// Manages shared global state cache which reflects the canonical
/// state as it is on the disk. All the entries in the cache are clean.
/// A clone of `StateDB` may be created as canonical or not.
/// For canonical clones local cache is accumulated and applied
/// in `sync_cache`
/// For non-canonical clones local cache is dropped.
///
/// Global cache propagation.
/// After a `State` object has been committed to the trie it
/// propagates its local cache into the `StateDB` local cache
/// using `add_to_account_cache` function.
/// Then, after the block has been added to the chain the local cache in the
/// `StateDB` is propagated into the global cache.
#[derive(Clone)]
pub struct StateDB {
/*
	/// Backing database.
	db: Box<JournalDB>,
	/// Shared canonical state cache.
	account_cache: Arc<Mutex<AccountCache>>,
	/// DB Code cache. Maps code hashes to shared bytes.
	code_cache: Arc<Mutex<MemoryLruCache<H256, Arc<Vec<u8>>>>>,
	/// Local dirty cache.
	local_cache: Vec<CacheQueueItem>,
	/// Shared account bloom. Does not handle chain reorganizations.
	account_bloom: Arc<Mutex<Bloom>>,
	cache_size: usize,
	/// Hash of the block on top of which this instance was created or
	/// `None` if cache is disabled
	parent_hash: Option<H256>,
	/// Hash of the committing block or `None` if not committed yet.
	commit_hash: Option<H256>,
	/// Number of the committing block or `None` if not committed yet.
	commit_number: Option<BlockNumber>,
*/
}

impl StateDB {
	/// Create a new instance wrapping `JournalDB` and the maximum allowed size
	/// of the LRU cache in bytes. Actual used memory may (read: will) be higher due to bookkeeping.
	// TODO: make the cache size actually accurate by moving the account storage cache
	// into the `AccountCache` structure as its own `LruCache<(Address, H256), H256>`.
	pub fn new(/*db: Box<JournalDB>,*/ _cache_size: usize) -> StateDB {
            StateDB { }
/*
		let bloom = Self::load_bloom(&**db.backing());
		let acc_cache_size = cache_size * ACCOUNT_CACHE_RATIO / 100;
		let code_cache_size = cache_size - acc_cache_size;
		let cache_items = acc_cache_size / ::std::mem::size_of::<Option<Account>>();

		StateDB {
			db: db,
			account_cache: Arc::new(Mutex::new(AccountCache {
				accounts: LruCache::new(cache_items),
				modifications: VecDeque::new(),
			})),
			code_cache: Arc::new(Mutex::new(MemoryLruCache::new(code_cache_size))),
			local_cache: Vec::new(),
			account_bloom: Arc::new(Mutex::new(bloom)),
			cache_size: cache_size,
			parent_hash: None,
			commit_hash: None,
			commit_number: None,
		}
*/
	}
}

/// Messages to broadcast via chain
pub enum ChainMessageType {
	/// Consensus message
	Consensus(Vec<u8>),
	/// Message with private transaction
	PrivateTransaction(H256, Vec<u8>),
	/// Message with signed private transaction
	SignedPrivateTransaction(H256, Vec<u8>),
}
