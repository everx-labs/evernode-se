use ed25519_dalek::{SecretKey, ExpandedSecretKey};
#[cfg(feature = "server")]
use ed25519_dalek::PublicKey;
#[cfg(any(feature = "server",feature = "client"))]
use serde::Deserialize;
#[cfg(any(feature = "server", feature = "client"))]
use sha2::{Digest, Sha256};
use std::convert::TryInto;
#[cfg(any(feature = "server",feature = "client"))]
use std::net::SocketAddr;
#[cfg(feature = "server")]
use std::time::Duration;

/// Server key option
#[derive(Clone, Debug)]
pub struct KeyOption {
    id: [u8; 32],
    pub_key: Option<[u8; 32]>,
    pvt_key: Option<[u8; 32]>
}

impl KeyOption {

    /// Construct from public key (client scenario)
    #[cfg(feature = "client")]
    pub fn from_public_key(type_id: i32, key: &str) -> Self {
        let key = base64::decode(key).expect("Cannot decode server public key");
        if key.len() != 32 {
            panic!("Bad server public key");
        } 
        let mut pub_key = [0u8; 32];
        pub_key.clone_from_slice(&key[..32]);
        KeyOption {
            id: Self::calc_id(type_id, &pub_key),
            pub_key: Some(pub_key),
            pvt_key: None
        }
    }

    /// Construct from private key (server scenario)
    #[cfg(feature = "server")]
    pub fn from_private_key(type_id: i32, key: &str) -> Self {
        let key = base64::decode(key).expect("Cannot decode server private key");
        if key.len() != 32 {
            panic!("Bad server private key");
        } 
        let pvt_key = SecretKey::from_bytes(&key[..32]).expect("Bad server private key");
        let exp_key = ExpandedSecretKey::from(&pvt_key);
        let pub_key = PublicKey::from(&exp_key).to_bytes();
        KeyOption {
            id: Self::calc_id(type_id, &pub_key),
            pub_key: None,
            pvt_key: Some(Self::normalize_pvt_key(pvt_key))
        }
    }

    /// Get key id 
    pub fn id(&self) -> &[u8; 32] {
        &self.id
    }

    /// Get public key
    pub fn pub_key(&self) -> &[u8; 32] {
        self.pub_key.as_ref().expect("Public key must be set")
    }

    /// Get private key
    pub fn pvt_key(&self) -> &[u8; 32] {
        self.pvt_key.as_ref().expect("Private key must be set")
    }

    /// Get normalized private key
    pub fn normalize_pvt_key(key: SecretKey) -> [u8; 32] {
        ExpandedSecretKey::from(&key)
            .to_bytes()[..32]
            .try_into()
            .expect("Bad private key data")
    }

    #[cfg(any(feature = "server", feature = "client"))]
    fn calc_id(type_id: i32, pub_key: &[u8; 32]) -> [u8; 32] {
        let mut sha = Sha256::new();
        sha.input(&type_id.to_le_bytes());
        sha.input(pub_key);
        let mut ret = [0u8; 32];
        ret.copy_from_slice(sha.result_reset().as_slice());         
        ret
    }

}

/// ADNL client configuration
#[cfg(feature = "client")]
#[derive(Clone, Debug)]
pub struct AdnlClientConfig {
    server_address: SocketAddr,
    server_key: KeyOption
}

#[cfg(feature = "client")]
#[derive(Deserialize)]
struct AdnlClientConfigJson {
    server_address: String,
    server_type_id: i32,
    server_pub_key: String
}

#[cfg(feature = "client")]
impl AdnlClientConfig {

    /// Costructs new configuration from JSON data
    pub fn from_json(json: &str) -> Self {
        let json_config: AdnlClientConfigJson = serde_json::from_str(json)
            .expect("Bad client configuration format");
        AdnlClientConfig {
            server_address: json_config.server_address.parse().expect("Bad address to connect to"),
            server_key: KeyOption::from_public_key(
                json_config.server_type_id, 
                &json_config.server_pub_key
            )
        }
    }

    /// Get server address 
    pub fn server_address(&self) -> &SocketAddr {
        &self.server_address
    }

    /// Get server key
    pub fn server_key(&self) -> &KeyOption {
        &self.server_key
    }

}

/// ADNL server configuration
#[cfg(feature = "server")]
#[derive(Clone, Debug)]
pub struct AdnlServerConfig {
    address: SocketAddr,
    keys: Vec<KeyOption>,
    read_timeout: Option<Duration>,
    write_timeout: Option<Duration>
}

#[cfg(feature = "server")]
#[allow(dead_code)] // Because of pub_key
#[derive(Deserialize)]
struct AdnlServerKeyOptionJson {
    type_id: i32,
    pub_key: String,
    pvt_key: String,
}

#[cfg(feature = "server")]
#[derive(Deserialize)]
pub struct AdnlServerConfigJson {
    address: String,
    keys: Vec<AdnlServerKeyOptionJson>,
    read_timeout: Option<Duration>,
    write_timeout: Option<Duration>
}

#[cfg(feature = "server")]
impl AdnlServerConfig {

    /// Costructs new configuration from JSON-based config
    pub fn from_json_config(json_config: &AdnlServerConfigJson) -> Self {
        let mut keys = Vec::new();
        for key in json_config.keys.iter() {
            keys.push(KeyOption::from_private_key(key.type_id, &key.pvt_key))
        }
        AdnlServerConfig {
            address: json_config.address.parse().expect("Bad address to listen on"),
            keys,
            read_timeout: json_config.read_timeout,
            write_timeout: json_config.write_timeout
        }
    }

    /// Costructs new configuration with address
    pub fn with_address_and_keys(address: &str, keys: Vec<KeyOption>) -> Self {
        AdnlServerConfig {
            address: address.parse().expect("Bad address to listen on"),
            keys,
            write_timeout: None,
            read_timeout: None
        }
    }

    /// Get address to listen on
    pub fn address(&self) -> &SocketAddr {
        &self.address
    }

    /// Get available key options
    pub fn keys(&self) -> &Vec<KeyOption> {
        &self.keys
    }

    pub fn read_timeout(&self) -> Option<Duration> {
        self.read_timeout
    }

    pub fn write_timeout(&self) -> Option<Duration> {
        self.write_timeout
    }

}
