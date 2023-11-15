use ton_block::Deserializable;
use ton_executor::BlockchainConfig;

use crate::{error::{NodeError, NodeResult}, config::ForkConfig};

use super::ExternalAccountsProvider;

pub struct ForkProvider {
    url: String,
    secret: Option<String>,
    client: reqwest::Client,
}

impl ForkProvider {
    pub fn new(config: ForkConfig) -> Self {
        Self {
            url: config.endpoint,
            secret: config.auth,
            client: reqwest::Client::new(),
        }
    }

    fn try_extract_graphql_error(value: &serde_json::Value) -> Option<String> {
        let errors = if let Some(payload) = value.get("payload") {
            payload.get("errors")
        } else {
            value.get("errors")
        };

        errors.map(|err| err.to_string())
    }

    fn query(&self, query: &str, variables: serde_json::Value) -> NodeResult<serde_json::Value> {
        let mut request = self
            .client
            .request(reqwest::Method::POST, &self.url)
            .header("content-type", "application/json")
            .body(serde_json::json!({
                "query": query,
                "variables": variables
            }).to_string());

        if let Some(secret) = &self.secret {
            request = request.header("Authorization", secret);
        }

        let response = request
            .send()
            .map_err(|err| NodeError::ForkEndpointFetchError(err.to_string()))?
            .text()
            .map_err(|err| NodeError::ForkEndpointFetchError(err.to_string()))?;

        let response = serde_json::from_str(&response)
            .map_err(|err| NodeError::ForkEndpointFetchError(err.to_string()))?;

        if let Some(err) = Self::try_extract_graphql_error(&response) {
            return Err(NodeError::ForkEndpointFetchError(err));
        }

        Ok(response)
    }

    pub fn get_blockchain_config(&self) -> NodeResult<(BlockchainConfig, i32)> {
        let response = self.query(
            "query{blockchain{key_blocks(last:1){edges{node{boc}}}}}",
            Default::default(),
        )?;

        let boc = response
            .pointer("/data/blockchain/key_blocks/edges/0/node/boc")
            .map(|val| val.as_str())
            .flatten()
            .ok_or_else(|| NodeError::ForkEndpointFetchError("No key block found".to_owned()))?;
        
        let block = ton_block::Block::construct_from_base64(boc)
            .map_err(|err| NodeError::ForkEndpointFetchError(err.to_string()))?;

        let mc_extra = block
            .read_extra()
            .and_then(|extra| extra.read_custom())
            .map_err(|err| NodeError::ForkEndpointFetchError(err.to_string()))?
            .ok_or_else(|| NodeError::ForkEndpointFetchError("Key block is not master block".to_owned()))?;
        let config = mc_extra
            .config()
            .ok_or_else(|| NodeError::ForkEndpointFetchError("Key block without config".to_owned()))?;

        Ok((
            BlockchainConfig::with_config(config.clone())
                .map_err(|err| NodeError::ForkEndpointFetchError(err.to_string()))?,
            block.global_id(),
        ))
    }
}

impl ExternalAccountsProvider for ForkProvider {
    fn get_account(
        &self,
        address: ton_block::MsgAddressInt,
    ) -> NodeResult<Option<ton_block::ShardAccount>> {
        let response = self.query(
            "query account($address:String!){blockchain{account(address:$address){info{boc}}}}",
            serde_json::json!({
                "address": address.to_string(),
            }))?;

        let boc = response
            .pointer("/data/blockchain/account/info/boc")
            .map(|val| val.as_str())
            .flatten();

        match boc {
            Some(boc) => {
                let bytes = base64::decode(boc)
                    .map_err(|err| NodeError::ForkEndpointFetchError(err.to_string()))?;
                let cell = ton_types::read_single_root_boc(&bytes)
                    .map_err(|err| NodeError::ForkEndpointFetchError(err.to_string()))?;
                Ok(Some(ton_block::ShardAccount::with_account_root(
                    cell,
                    ton_types::UInt256::default(),
                    0,
                )))
            }
            None => Ok(None),
        }
    }
}

#[test]
fn test_account_provider() {
    let provider = ForkProvider::new(
        ForkConfig {
            endpoint: "https://mainnet.evercloud.dev/1467de01155143d5ba129ab5ea4ede1f/graphql".to_owned(),
            auth:  None,
        }
    );

    dbg!(provider
        .get_account(
            ton_block::MsgAddressInt::with_standart(None, -1, ton_types::UInt256::default().into())
                .unwrap()
        )
        .unwrap());
}
