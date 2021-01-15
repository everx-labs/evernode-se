use aes_ctr::{
    Aes256Ctr, stream_cipher::NewStreamCipher, stream_cipher::SyncStreamCipher,
    stream_cipher::generic_array::GenericArray,
};
use bytes::{BufMut, BytesMut};
#[cfg(feature = "client")]
use config::AdnlClientConfig;
#[cfg(feature = "server")]
use config::AdnlServerConfig;
use config::KeyOption;
use curve25519_dalek::edwards::CompressedEdwardsY;
use debug::dump;
#[cfg(feature = "client")]
use ed25519_dalek::{SecretKey, PublicKey};
use error::{AdnlError, AdnlErrorKind, AdnlResult};
use rand::Rng;
#[cfg(feature = "client")]
use rand::RngCore;
use sha2::{Digest, Sha256};
use tokio::codec::{Encoder, Decoder};
use x25519_dalek::x25519;

pub struct DataCodec {
    cipher_recv: Option<Aes256Ctr>,
    cipher_send: Option<Aes256Ctr>,
    is_server: bool,
    keys: Vec<KeyOption>,
    length: Option<usize>,
}

impl DataCodec {

    #[cfg(feature = "client")]
    pub fn with_client_config(config: &AdnlClientConfig) -> Self {
        Self { 
            cipher_recv: None, 
            cipher_send: None, 
            is_server: false,
            keys: vec![config.server_key().clone()],
            length: None,
        }
    }

    #[cfg(feature = "server")]
    pub fn with_server_config(config: &AdnlServerConfig) -> Self {
        Self { 
            cipher_recv: None, 
            cipher_send: None, 
            is_server: true,
            keys: config.keys().clone(),
            length: None,
        }
    }

    #[cfg(feature = "client")]
    pub fn build_init_packet(&mut self) -> BytesMut {

        let mut nonce = [0u8; 160];
        rand::thread_rng().fill_bytes(&mut nonce);                        

        let this_pvt_key = SecretKey::generate(&mut rand::thread_rng());
        let peer_key = self.keys.get(0).expect("Server key must be configured");

        let mut ret = BytesMut::new();
        ret.reserve(256);
        ret.put_slice(peer_key.id());
        ret.put_slice(PublicKey::from(&this_pvt_key).as_bytes());
        let this_pvt_key = KeyOption::normalize_pvt_key(this_pvt_key);

        let checksum = Self::calc_checksum(&nonce);
        ret.put_slice(&checksum);
        let shared_secret = Self::calc_shared_secret(&this_pvt_key, peer_key.pub_key());
        dump("Shared Secret", &shared_secret);

        self.setup(&nonce);
        dump("Nonce", &nonce);
        Self::build_nonce_cipher(&shared_secret, &checksum).apply_keystream(&mut nonce);
        ret.put_slice(&nonce);
        ret

    }

    fn parse_init_packet(&mut self, d: &mut BytesMut) -> AdnlResult<()> {

        if d.len() != 256 {
            return Err(adnl_err!(AdnlErrorKind::StartingPacketBadLength(d.len())));
        }
        let mut shared_secret = None;
        for key in self.keys.iter() {
            if key.id().eq(&d[0..32]) {  
                shared_secret = Some(
                    Self::calc_shared_secret(key.pvt_key(), array_ref!(d, 32, 32))
                );
                break;
            }
        }          
        let shared_secret = shared_secret.ok_or(
            adnl_err!(AdnlErrorKind::StartingPacketUnknownId)
        )?;
        dump("Shared secret", &shared_secret);

        Self::build_nonce_cipher(&shared_secret, array_ref!(d, 64, 32))
            .apply_keystream(&mut d[96..]);
        dump("Nonce", &d[96..]);
        if !Sha256::digest(&d[96..]).as_slice().eq(&d[64..96]) {
            return Err(adnl_err!(AdnlErrorKind::StartingPacketBadChecksum));
        }

        self.setup(array_ref!(d, 96, 160));
        d.split_to(256);
        Ok(())

    }

    fn setup(&mut self, nonce: &[u8; 160]) {
        if self.is_server {
            // Server side
            self.cipher_recv = Self::build_cipher(&nonce[32..64], &nonce[80..96]);
            self.cipher_send = Self::build_cipher(&nonce[ 0..32], &nonce[64..80]);
        } else {
            // Client side
            self.cipher_recv = Self::build_cipher(&nonce[ 0..32], &nonce[64..80]);
            self.cipher_send = Self::build_cipher(&nonce[32..64], &nonce[80..96]);
        }
    }

    fn build_cipher(key: &[u8], ctr: &[u8]) -> Option<Aes256Ctr> {
        Some(
            Aes256Ctr::new(
                GenericArray::from_slice(key), 
                GenericArray::from_slice(ctr), 
            )
        )
    }

    fn calc_checksum(nonce: &[u8]) -> [u8; 32] {
        let mut sha = Sha256::new();
        sha.input(nonce);
        let mut ret = [0u8; 32];
        ret.copy_from_slice(sha.result_reset().as_slice()); 
        ret
    }

    fn calc_shared_secret(pvt_key: &[u8; 32], pub_key: &[u8; 32]) -> [u8; 32] {
        let node_point = CompressedEdwardsY(*pub_key)
            .decompress()
            .expect("Bad public key data")
            .to_montgomery()
            .to_bytes();
        x25519(*pvt_key, node_point)
    }

    fn build_nonce_cipher(shared_secret: &[u8; 32], checksum: &[u8; 32]) -> Aes256Ctr {
        let mut aes_key_bytes = [0; 32];
        aes_key_bytes[..16].copy_from_slice(&shared_secret[..16]);
        aes_key_bytes[16..].copy_from_slice(&checksum[16..]);
        dump("AES-Ctr Key (init)", &aes_key_bytes);
        let mut aes_ctr_bytes = [0; 16];
        aes_ctr_bytes[0..4].copy_from_slice(&checksum[..4]);
        aes_ctr_bytes[4..16].copy_from_slice(&shared_secret[20..]);
        dump("AES-Ctr Counter (init)", &aes_ctr_bytes);
        let aes_key = GenericArray::from_slice(&aes_key_bytes);
        let aes_ctr = GenericArray::from_slice(&aes_ctr_bytes);
        Aes256Ctr::new(&aes_key, &aes_ctr)
    }

}

impl Decoder for DataCodec {

    type Item = Vec<u8>;
    type Error = AdnlError;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {

        if let Some(ref mut cipher) = self.cipher_recv {
            // Encrypted channel operation
            let length = if self.length.is_none() {
                if src.len() < 4 {
                    return Ok(None);
                }
                let mut bytes = [0; 4];
                bytes.clone_from_slice(&src[..4]);
                src.split_to(4);
                cipher.apply_keystream(&mut bytes[..4]);
                u32::from_le_bytes(bytes) as usize
            } else {
                self.length.take().unwrap()
            };

            if length > src.len() {
                self.length = Some(length);
                return Ok(None);
            }

            if length < 64 {
                return Err(adnl_err!(AdnlErrorKind::BadLength(length)));
            }
            cipher.apply_keystream(&mut src[..length]);        
            if !Sha256::digest(&src[..length - 32]).as_slice().eq(&src[length - 32..length]) {
                return Err(adnl_err!(AdnlErrorKind::BadChecksum));
            }
            
            let ret = Vec::from(&src[32..length - 32]); 
            src.split_to(length);
            Ok(Some(ret))

        } else if self.is_server {
            // Setup encryption on server side
            self.parse_init_packet(src)?;
            Ok(Some(Vec::new()))
        } else {
            // It is an error to send something in clear 
            Err(adnl_err!(AdnlErrorKind::UnexpectedPacket))
        }

    }

}

impl Encoder for DataCodec {

    type Item = Vec<u8>;
    type Error = AdnlError;

    fn encode(&mut self, data: Self::Item, dst: &mut BytesMut) -> Result<(), Self::Error> {
        if let Some(ref mut cipher) = self.cipher_send {
            // Encrypted channel operation
            let start = dst.len();
            let length = data.len() + 64;
            dst.reserve(start + length + 4);
            dst.put_slice(&(length as u32).to_le_bytes());
            let mut buf: [u8; 32] = rand::thread_rng().gen();
            dst.put_slice(&buf);
            dst.put_slice(&data[..]);
            buf = Self::calc_checksum(&dst[start + 4..start + length - 28]);
            dst.put_slice(&buf);
            cipher.apply_keystream(&mut dst[start..start + length + 4]);
            Ok(())
        } else {
            // It is an error to send something in clear 
            Err(adnl_err!(AdnlErrorKind::UnexpectedPacket))
        }
    }

}