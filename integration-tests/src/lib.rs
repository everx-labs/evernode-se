#![cfg(test)]

use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use failure::err_msg;
use lazy_static::lazy_static;
use serde_json::{json, Value};
use ever_block::{
    BlockProcessingStatus,
    MessageProcessingStatus,
    TransactionProcessingStatus,
    Serializable,
    Result
};
use ton_client::abi::{
    encode_message, Abi, CallSet, DeploySet, FunctionHeader, ParamsOfEncodeMessage,
    ResultOfEncodeMessage, Signer
};
use ton_client::boc::{parse_message, ParamsOfParse};
use ton_client::crypto::KeyPair;
use ton_client::net::{
    query_collection, wait_for_collection, NetworkConfig, ParamsOfQueryCollection,
    ParamsOfWaitForCollection, OrderBy, SortDirection,
};
use ton_client::processing::{
    send_message, wait_for_transaction, DecodedOutput, ResultOfProcessMessage,
};
use ton_client::processing::{ParamsOfSendMessage, ParamsOfWaitForTransaction};
use ton_client::tvm::{run_tvm, ParamsOfRunTvm, ResultOfRunTvm};
use ton_client::{ClientConfig, ClientContext};

const DEFAULT_NETWORK_ADDRESS: &str = "http://localhost";
type Keypair = ed25519_dalek::Keypair;
type Addr = String;

#[derive(Clone)]
pub(crate) struct Client {
    context: Arc<ClientContext>,
}

impl Client {
    pub fn new() -> Self {
        let config = ClientConfig {
            network: NetworkConfig {
                server_address: Some(Self::network_address()),
                ..Default::default()
            },
            ..Default::default()
        };

        Self {
            context: Arc::new(ClientContext::new(config).unwrap()),
        }
    }

    pub fn context(&self) -> Arc<ClientContext> {
        Arc::clone(&self.context)
    }

    pub fn network_address() -> String {
        std::env::var("TON_NETWORK_ADDRESS").unwrap_or(DEFAULT_NETWORK_ADDRESS.into())
    }

    pub fn generate_keypair() -> ed25519_dalek::Keypair {
        ed25519_dalek::Keypair::generate(&mut rand::thread_rng())
    }

    pub async fn call_contract(
        &self,
        addr: &str,
        abi: Abi,
        method: &str,
        params: Value,
        keys: Option<&ed25519_dalek::Keypair>,
    ) -> Result<ResultOfProcessMessage> {
        let msg = self
            .prepare_message(addr, abi.clone(), method, params, None, keys)
            .await?;

        self.send_message_and_wait(Some(abi), msg.message).await
    }

    pub async fn run_contract(
        &self,
        addr: &str,
        abi: Abi,
        method: &str,
        params: Value,
    ) -> Result<ResultOfRunTvm> {
        let message = self
            .prepare_message(addr, abi.clone(), method, params, None, None)
            .await?
            .message;

        let account = self.query_account_boc(addr).await?;

        run_tvm(
            self.context(),
            ParamsOfRunTvm {
                message,
                account,
                abi: Some(abi),
                return_updated_account: Some(true),
                ..Default::default()
            },
        )
        .await
        .map_err(|e| err_msg(format!("run failed: {:#}", e)))
    }

    pub async fn encode_message(
        &self,
        abi: Abi,
        address: Option<String>,
        deploy_set: Option<DeploySet>,
        call_set: Option<CallSet>,
        keys: Option<&ed25519_dalek::Keypair>,
    ) -> Result<ResultOfEncodeMessage> {
        let signer = if let Some(keys) = keys {
            Signer::Keys {
                keys: KeyPair::new(hex::encode(&keys.public), hex::encode(&keys.secret)),
            }
        } else {
            Signer::None
        };

        encode_message(
            self.context(),
            ParamsOfEncodeMessage {
                abi,
                address,
                deploy_set,
                call_set,
                signer,
                ..Default::default()
            },
        )
        .await
        .map_err(|e| err_msg(e.message))
    }

    pub async fn prepare_message(
        &self,
        addr: &str,
        abi: Abi,
        method: &str,
        params: Value,
        header: Option<FunctionHeader>,
        keys: Option<&ed25519_dalek::Keypair>,
    ) -> Result<ResultOfEncodeMessage> {
        let call_set = Some(CallSet {
            function_name: method.into(),
            input: Some(params),
            header: header.clone(),
            ..Default::default()
        });

        self.encode_message(abi, Some(addr.to_owned()), None, call_set, keys)
            .await
    }

    pub async fn send_message_and_wait(
        &self,
        abi: Option<Abi>,
        msg: String,
    ) -> Result<ResultOfProcessMessage> {
        let callback = |_| async move {};

        let result = send_message(
            self.context(),
            ParamsOfSendMessage {
                message: msg.clone(),
                abi: abi.clone(),
                ..Default::default()
            },
            callback,
        )
        .await?;

        Ok(wait_for_transaction(
            self.context(),
            ParamsOfWaitForTransaction {
                abi: abi.clone(),
                message: msg.clone(),
                shard_block_id: result.shard_block_id,
                send_events: true,
                ..Default::default()
            },
            callback.clone(),
        )
        .await?)
    }

    pub async fn query_account_boc(&self, addr: &str) -> Result<String> {
        let accounts = query_collection(
            self.context(),
            ParamsOfQueryCollection {
                collection: "accounts".to_string(),
                filter: Some(json!({ "id": { "eq": addr } })),
                result: "boc".to_string(),
                ..Default::default()
            },
        )
        .await
        .map_err(|e| err_msg(format!("failed to query account: {}", e)))?
        .result;

        if accounts.len() == 0 {
            return Err(err_msg(format!("account not found")));
        }
        let boc = accounts[0]["boc"].as_str();
        if boc.is_none() {
            return Err(err_msg(format!("account doesn't contain data")));
        }

        Ok(boc.unwrap().to_owned())
    }

    pub async fn wait_for_out_messages(&self, out_messages: &Vec<String>) -> Result<()> {
        for message in out_messages {
            let msg_json = parse_message(
                self.context(),
                ParamsOfParse {
                    boc: message.clone(),
                },
            )?
            .parsed;
            if msg_json["msg_type"] == 0 {
                wait_for_collection(
                    self.context(),
                    ParamsOfWaitForCollection {
                        collection: "transactions".to_owned(),
                        filter: Some(json!({
                            "in_msg": { "eq": msg_json["id"] }
                        })),
                        result: "id".to_owned(),
                        ..Default::default()
                    },
                )
                .await?;
            }
        }

        Ok(())
    }
}

pub(crate) struct Giver {
    abi: Abi,
    keypair: ed25519_dalek::Keypair,
}

impl Giver {
    pub fn new() -> Self {
        let keypair = include_str!("../../../evernode-se/contracts/giver_v3/seGiver.keys.json");
        let json: Value =
            serde_json::from_str(keypair).expect("Failed to parse giver's keypair file");
        let public = json["public"].as_str().expect("public");
        let secret = json["secret"].as_str().expect("secret");
        let public = hex::decode(public).expect("Failed to parse giver's public key");
        let secret = hex::decode(secret).expect("Failed to parse giver's secret key");
        Self {
            abi: Abi::Contract(
                serde_json::from_str(include_str!(
                    "../../../evernode-se/contracts/giver_v3/GiverV3.abi.json"
                ))
                .unwrap(),
            ),
            keypair: ed25519_dalek::Keypair {
                public: ed25519_dalek::PublicKey::from_bytes(&public).expect("public"),
                secret: ed25519_dalek::SecretKey::from_bytes(&secret).expect("secret"),
            },
        }
    }

    pub fn address() -> &'static str {
        "0:96137b99dcd65afce5a54a48dac83c0fd276432abbe3ba7f1bfb0fb795e69025"
    }

    pub fn abi(&self) -> &Abi {
        &self.abi
    }

    pub fn keypair(&self) -> &ed25519_dalek::Keypair {
        &self.keypair
    }

    pub(crate) async fn send_grams(
        &self,
        client: &Client,
        account: &str,
        value: u64,
    ) -> Result<ResultOfProcessMessage> {
        client
            .call_contract(
                Self::address(),
                self.abi().clone(),
                "sendTransaction",
                json!({
                    "dest": account,
                    "value": value,
                    "bounce": false,
                }),
                Some(self.keypair()),
            )
            .await
    }
}

pub(crate) struct Multisig {
    abi: Abi,
    keypair: ed25519_dalek::Keypair,
}

impl Multisig {
    pub fn new() -> Self {
        let keypair = include_str!("../../../evernode-se/contracts/safe_multisig/SafeMultisigWallet.keys.json");
        let json: Value =
            serde_json::from_str(keypair).expect("Failed to parse giver's keypair file");
        let public = json["public"].as_str().expect("public");
        let secret = json["secret"].as_str().expect("secret");
        let public = hex::decode(public).expect("Failed to parse giver's public key");
        let secret = hex::decode(secret).expect("Failed to parse giver's secret key");
        Self {
            abi: Abi::Contract(
                serde_json::from_str(include_str!(
                    "../../../evernode-se/contracts/safe_multisig/SafeMultisigWallet.abi.json"
                ))
                .unwrap(),
            ),
            keypair: ed25519_dalek::Keypair {
                public: ed25519_dalek::PublicKey::from_bytes(&public).expect("public"),
                secret: ed25519_dalek::SecretKey::from_bytes(&secret).expect("secret"),
            },
        }
    }

    pub fn address() -> &'static str {
        "0:d5f5cfc4b52d2eb1bd9d3a8e51707872c7ce0c174facddd0e06ae5ffd17d2fcd"
    }

    pub fn abi(&self) -> &Abi {
        &self.abi
    }

    pub fn keypair(&self) -> &ed25519_dalek::Keypair {
        &self.keypair
    }

    pub(crate) async fn send_grams(
        &self,
        client: &Client,
        account: &str,
        value: u64,
        bounce: bool,
        flags: u8,
        payload: Option<String>,
    ) -> Result<ResultOfProcessMessage> {
        client
            .call_contract(
                Self::address(),
                self.abi().clone(),
                "sendTransaction",
                json!({
                    "dest": account,
                    "value": value,
                    "bounce": bounce,
                    "flags": flags,
                    "payload": payload.unwrap_or_else(|| String::new()),
                }),
                Some(self.keypair()),
            )
            .await
    }
}

pub(crate) struct TestUtils;

impl TestUtils {
    pub fn load_contract(name: &str) -> Result<(String, Abi)> {
        let path = Path::new("contracts");
        let name = Path::new(name);
        let tvc_path = path.join(name.with_extension("tvc"));
        let abi_path = path.join(name.with_extension("abi.json"));

        let tvc = base64::encode(&std::fs::read(tvc_path)?);
        let abi = Abi::Contract(serde_json::from_str(&std::fs::read_to_string(abi_path)?)?);

        Ok((tvc, abi))
    }

    pub async fn deploy_with_giver(
        client: &Client,
        abi: Abi,
        tvc: String,
        workchain_id: i32,
        call_set: Option<CallSet>,
        keys: &ed25519_dalek::Keypair,
        value: u64,
    ) -> Result<(String, Option<DecodedOutput>)> {
        let (address, message) = client
            .encode_message(
                abi.clone(),
                None,
                Some(DeploySet {
                    tvc: Some(tvc),
                    workchain_id: Some(workchain_id),
                    ..Default::default()
                }),
                call_set,
                Some(keys),
            )
            .await
            .map(|encoded| (encoded.address, encoded.message))?;

        let giver = Giver::new();

        client
            .wait_for_out_messages(
                &giver
                    .send_grams(&client, &address, value)
                    .await?
                    .out_messages,
            )
            .await?;

        client
            .send_message_and_wait(Some(abi), message)
            .await
            .map(|send_result| (address, send_result.decoded))
    }
}

async fn deploy_msg_ordering_receiver(client: &Client, keypair: &Keypair) -> Result<(Addr, Abi)> {
    let (receiver_tvc, receiver_abi) = TestUtils::load_contract("MessageOrderingReceiver")?;
    let (receiver_addr, _output) = TestUtils::deploy_with_giver(
        client,
        receiver_abi.clone(),
        receiver_tvc,
        0,
        CallSet::some_with_function("constructor"),
        &keypair,
        10_000_000_000_000,
    )
    .await?;

    Ok((receiver_addr, receiver_abi))
}

async fn deploy_msg_ordering_sender(
    client: &Client,
    keypair: &Keypair,
    receiver_addr: Addr,
) -> Result<(Addr, Abi)> {
    let (sender_tvc, sender_abi) = TestUtils::load_contract("MessageOrderingSender")?;
    let (sender_addr, _output) = TestUtils::deploy_with_giver(
        client,
        sender_abi.clone(),
        sender_tvc,
        0,
        CallSet::some_with_function_and_input(
            "constructor",
            json!({
                "receiver": receiver_addr,
            }),
        ),
        &keypair,
        10_000_000_000_000,
    )
    .await?;

    Ok((sender_addr, sender_abi))
}

async fn execute_msg_ordering_sender(
    client: &Client,
    addr: &Addr,
    abi: Abi,
    keypair: &Keypair,
    max: u32,
) -> Result<Vec<String>> {
    let result = client
        .call_contract(
            addr,
            abi,
            "sendMessages",
            json!({
            "max": max,
            }),
            Some(&keypair),
        )
        .await?;

    assert!(result
        .decoded
        .expect("Expected sendMessages output")
        .output
        .is_none());

    Ok(result.out_messages)
}

async fn get_receiver_last_accepted(client: &Client, addr: &Addr, abi: Abi) -> Result<u32> {
    let result = client
        .run_contract(addr, abi, "getLastAccepted", json!({}))
        .await?
        .decoded
        .expect("Expected getLastAccepted output")
        .output
        .expect("Expected getLastAccepted result");

    let result = result["result"].as_str().expect("Expected string result");

    Ok(u32::from_str(result)?)
}

async fn test_internal_message_order_internal(client: &Client) -> Result<()> {
    let keypair = Client::generate_keypair();

    let (receiver_addr, receiver_abi) = deploy_msg_ordering_receiver(&client, &keypair).await?;
    let (sender_addr, sender_abi) =
        deploy_msg_ordering_sender(&client, &keypair, receiver_addr.clone()).await?;

    const MAX: u32 = 100;
    lazy_static! {
        static ref MUTEX: tokio::sync::Mutex<()> = tokio::sync::Mutex::new(());
    }
    let last_accepted = {
        let _guard = MUTEX.lock().await;

        let out_messages =
            execute_msg_ordering_sender(&client, &sender_addr, sender_abi, &keypair, MAX).await?;

        assert_eq!(out_messages.len(), MAX as usize + 1);

        client.wait_for_out_messages(&out_messages).await?;

        get_receiver_last_accepted(&client, &receiver_addr, receiver_abi).await?
    };

    assert_eq!(last_accepted, MAX);

    Ok(())
}

#[tokio::test]
async fn test_internal_message_order() -> Result<()> {
    let client = Client::new();

    test_internal_message_order_internal(&client).await
}

#[tokio::test]
async fn test_config_params_20_21() -> Result<()> {
    let client = Client::new();
    let keypair = Client::generate_keypair();

    let (tvc, abi) = TestUtils::load_contract("ConfigParamsTest")?;

    let (addr, _output) = TestUtils::deploy_with_giver(
        &client,
        abi.clone(),
        tvc,
        0,
        CallSet::some_with_function("constructor"),
        &keypair,
        10_000_000_000,
    )
    .await?;

    assert!(call_gas_to_ton(&client, &addr, abi.clone(), &keypair, -1).await > 0);
    assert!(call_gas_to_ton(&client, &addr, abi.clone(), &keypair, 0).await > 0);

    assert!(call_ton_to_gas(&client, &addr, abi.clone(), &keypair, -1).await > 0);
    assert!(call_ton_to_gas(&client, &addr, abi.clone(), &keypair, 0).await > 0);

    Ok(())
}

async fn call_gas_to_ton(
    client: &Client,
    addr: &str,
    abi: Abi,
    keypair: &ed25519_dalek::Keypair,
    workchain_id: i32,
) -> u64 {
    let method = "gasToTon";
    get_price(&extract_contract_output(
        method,
        &client
            .call_contract(
                &addr,
                abi,
                method,
                json!({
                    "gas": 1_000_000,
                    "wid": workchain_id,
                }),
                Some(&keypair),
            )
            .await,
    ))
}

async fn call_ton_to_gas(
    client: &Client,
    addr: &str,
    abi: Abi,
    keypair: &ed25519_dalek::Keypair,
    workchain_id: i32,
) -> u64 {
    let method = "tonToGas";
    get_price(&extract_contract_output(
        method,
        &client
            .call_contract(
                &addr,
                abi,
                method,
                json!({
                    "_ton": 1_000_000,
                    "wid": workchain_id,
                }),
                Some(&keypair),
            )
            .await,
    ))
}

fn extract_contract_output<'result>(
    method: &str,
    result: &'result Result<ResultOfProcessMessage>,
) -> &'result Value {
    result
        .as_ref()
        .expect(&format!("Error calling {} method", method))
        .decoded
        .as_ref()
        .expect(&format!("Expected {} output", method))
        .output
        .as_ref()
        .expect(&format!("Expected {} result", method))
}

fn get_price(value: &Value) -> u64 {
    value["price"]
        .as_str()
        .expect("Field not found: price")
        .parse()
        .expect("Error parsing price value")
}

#[tokio::test]
async fn test_ext_in_created_at() -> Result<()> {
    let client = Client::new();
    let messages = query_collection(
        client.context(),
        ParamsOfQueryCollection {
            collection: "messages".to_string(),
            filter: Some(json!({ "msg_type": { "eq": 1 } })),
            result: "created_at".to_string(),
            limit: Some(100),
            ..Default::default()
        },
    )
    .await
    .map_err(|e| err_msg(format!("failed to query messages: {}", e)))?
    .result;

    assert!(messages.len() > 0);
    for msg in messages {
        assert!(
            msg["created_at"]
                .as_u64()
                .ok_or(err_msg("created_at must be set!"))?
                > 0
        );
    }

    Ok(())
}

async fn ensure_no_non_finalized(
    client: &Client,
    collection: &str,
    finalized_status: u32,
) -> Result<()> {
    let items = query_collection(
        client.context(),
        ParamsOfQueryCollection {
            collection: collection.to_string(),
            filter: Some(json!({ "status": { "ne": finalized_status } })),
            result: "id, status, status_name".to_string(),
            ..Default::default()
        },
    )
    .await
    .map_err(|e| err_msg(format!("failed to query {}: {}", collection, e)))?
    .result;

    assert_eq!(
        items.len(),
        0,
        "Unexpected {} with status different from `Finalized`: {:?}",
        collection,
        items
    );

    Ok(())
}

#[tokio::test]
async fn test_message_statuses() -> Result<()> {
    let client = Client::new();

    test_internal_message_order_internal(&client).await?;

    ensure_no_non_finalized(
        &client,
        "messages",
        MessageProcessingStatus::Finalized as u32,
    )
    .await?;

    ensure_no_non_finalized(
        &client,
        "transactions",
        TransactionProcessingStatus::Finalized as u32,
    )
    .await?;

    ensure_no_non_finalized(&client, "blocks", BlockProcessingStatus::Finalized as u32).await?;

    Ok(())
}

async fn assert_account(
    client: &Client,
    id: &str,
    code_hash: &str,
    min_balance: u128,
) -> Result<()> {
    let accounts = query_collection(
        client.context(),
        ParamsOfQueryCollection {
            collection: "accounts".to_string(),
            filter: Some(json!({ "id": { "eq": id } })),
            result: "balance(format:DEC), code_hash".to_string(),
            ..Default::default()
        },
    )
    .await
    .map_err(|e| err_msg(format!("failed to query account: {}", e)))?
    .result;

    assert_eq!(accounts.len(), 1, "account not found: {}", id);
    let account = &accounts[0];

    assert_eq!(
        account["code_hash"], code_hash,
        "account's code_hash differs from expected"
    );
    let balance: u128 = account["balance"]
        .as_str()
        .expect("balance must be a string")
        .parse()?;
    assert!(
        balance >= min_balance,
        "account's {} balance {} is less than expected",
        id,
        balance
    );

    Ok(())
}

#[tokio::test]
async fn test_predeployed_contracts() -> Result<()> {
    let client = Client::new();

    // Giver v1
    assert_account(
        &client,
        "0:841288ed3b55d9cdafa806807f02a0ae0c169aa5edfe88a789a6482429756a94",
        "fdfab26e1359ddd0c247b0fb334c2cc3943256a263e75d33e5473567cbe2c124",
        4900000000000000000,
    )
    .await?;

    // Giver v2
    assert_account(
        &client,
        "0:ece57bcc6c530283becbbd8a3b24d3c5987cdddc3c8b7b33be6e4a6312490415",
        "ccbfc821853aa641af3813ebd477e26818b51e4ca23e5f6d34509215aa7123d9",
        4900000000000000000,
    )
    .await?;

    // Giver v3
    assert_account(
        &client,
        "0:96137b99dcd65afce5a54a48dac83c0fd276432abbe3ba7f1bfb0fb795e69025",
        "57a1e5e4304f4db2beb23117e4d85df9cb5caec127531350e73219a8b8dc8afd",
        4900000000000000000,
    )
    .await?;

    // SafeMultisigWallet
    assert_account(
        &client,
        "0:d5f5cfc4b52d2eb1bd9d3a8e51707872c7ce0c174facddd0e06ae5ffd17d2fcd",
        "80d6c47c4a25543c9b397b71716f3fae1e2c5d247174c52e2c19bd896442b105",
        999998000000000,
    )
    .await?;

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn test_my_code() {
    let client = Client::new();

    let keys = Client::generate_keypair();
    let (tvc, abi) = TestUtils::load_contract("MyCodeFail").unwrap();
    let (addr, _output) = TestUtils::deploy_with_giver(
        &client,
        abi.clone(),
        tvc,
        0,
        CallSet::some_with_function_and_input(
            "constructor",
            json!({ "pubkey": format!("0x{}", hex::encode(keys.public)) }),
        ),
        &keys,
        10_000_000_000,
    )
    .await
    .unwrap();

    let message = client
        .encode_message(
            abi.clone(),
            Some(addr),
            None,
            Some(CallSet {
                function_name: "requestCodeRefs".into(),
                input: Some(json!({})),
                ..Default::default()
            }),
            Some(&keys),
        )
        .await
        .unwrap()
        .message;

    let result = client.send_message_and_wait(Some(abi), message).await.unwrap();

    assert_eq!(
        json!({"value0": "2"}),
        result.decoded.unwrap().output.unwrap()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_time_shift() {
    let client = Client::new();

    let keys = Client::generate_keypair();
    let (tvc, abi) = TestUtils::load_contract("TimeShiftTest").unwrap();
    let (addr, _output) = TestUtils::deploy_with_giver(
        &client,
        abi.clone(),
        tvc,
        0,
        CallSet::some_with_function_and_input(
            "constructor",
            json!({ "pubkey": format!("0x{}", hex::encode(keys.public)) }),
        ),
        &keys,
        10_000_000_000,
    )
    .await
    .unwrap();

    let (tr_time1, tvm_time1) = return_time(&client, &addr, &keys, &abi).await;
    let delta = 10;
    let _ = se(&format!("increase-time?delta={}", delta)).await;
    let time_delta = se("time-delta").await.parse::<u64>().unwrap();
    assert_eq!(time_delta, 10);
    tokio::time::sleep(Duration::from_secs(1)).await;
    let (tr_time2, tvm_time2) = return_time(&client, &addr, &keys, &abi).await;
    let _ =  se("reset-time").await;
    let time_delta = se("time-delta").await.parse::<u64>().unwrap();
    assert_eq!(time_delta, 0);
    tokio::time::sleep(Duration::from_secs(delta + 1)).await;
    println!(
        "tr delta: {}, tvm delta: {}",
        tr_time2 - tr_time1,
        tvm_time2 - tvm_time1
    );
    assert!(tr_time2 - tr_time1 > delta);
    assert!(tvm_time2 - tvm_time1 > delta);
}

async fn se(command: &str) -> String {
    let se = reqwest::Client::new();
    let se_addr = Client::network_address();
    // let se_addr = "http://localhost:3000";
    let response = se
        .post(format!("{}/se/{}", se_addr, command))
        .send()
        .await
        .unwrap();
    let status = response.status().clone();
    let result = response.text().await.unwrap();
    if !status.is_success() {
        panic!("Server responded with error: {}", result);
    }
    result
}

async fn return_time(client: &Client, addr: &str, keys: &Keypair, abi: &Abi) -> (u64, u64) {
    let message = client
        .encode_message(
            abi.clone(),
            Some(addr.to_string()),
            None,
            Some(CallSet {
                function_name: "returnNow".into(),
                input: Some(json!({})),
                ..Default::default()
            }),
            Some(&keys),
        )
        .await
        .unwrap()
        .message;

    let result = client
        .send_message_and_wait(Some(abi.clone()), message)
        .await
        .unwrap();

    let tr_time = get_u64(&result.transaction, "now");
    let tvm_time = get_output_u64(&result, "value0");
    (tr_time, tvm_time)
}

fn get_output_u64(result: &ResultOfProcessMessage, name: &str) -> u64 {
    get_u64(
        &result.decoded.as_ref().unwrap().output.as_ref().unwrap(),
        name,
    )
}

fn get_u64(value: &Value, name: &str) -> u64 {
    let value = value.get(name).unwrap();
    if let Some(value) = value.as_u64() {
        value
    } else {
        value.as_str().unwrap().parse().unwrap()
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_non_sponsored_deploy() {
    let client = Client::new();

    let keys = Client::generate_keypair();
    let (tvc, abi) = TestUtils::load_contract("TimeShiftTest").unwrap();
    
    let message = client
    .encode_message(
        abi.clone(),
        None,
        Some(DeploySet {
            tvc: Some(tvc.clone()),
            ..Default::default()
        }),
        CallSet::some_with_function("constructor"),
        Some(&keys),
    )
    .await
    .unwrap();

    let err = client
        .send_message_and_wait(Some(abi.clone()), message.message.clone())
        .await
        .unwrap_err()
        .downcast::<ton_client::error::ClientError>().unwrap();

    assert_eq!(err.code, 406);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_gosh() {
    let client = Client::new();

    let keys = Client::generate_keypair();
    let (tvc, abi) = TestUtils::load_contract("test_gosh").unwrap();
    let (addr, _output) = TestUtils::deploy_with_giver(
        &client,
        abi.clone(),
        tvc,
        0,
        CallSet::some_with_function("constructor"),
        &keys,
        10_000_000_000,
    )
    .await
    .unwrap();

    let abi_json = match &abi {
        Abi::Contract(json) => json,
        _ => unreachable!(),
    };

    for function in abi_json.functions.iter().map(|f| &f.name).filter(|f| *f != "constructor") {
        let message = client
            .encode_message(
                abi.clone(),
                Some(addr.clone()),
                None,
                CallSet::some_with_function(function.as_str()),
                Some(&keys),
            )
            .await
            .unwrap()
            .message;
    
        client.send_message_and_wait(Some(abi.clone()), message).await.unwrap();
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_bounced_body() {
    let client = Client::new();

    let multisig = Multisig::new();
    let payload = ton_client::abi::encode_boc(
        client.context(),
        ton_client::abi::ParamsOfAbiEncodeBoc {
            params: vec![
                ton_client::abi::AbiParam {
                    components: vec![],
                    name: "a".to_owned(),
                    param_type: "uint256".to_owned(),
                    ..Default::default()
                },
                ton_client::abi::AbiParam {
                    components: vec![],
                    name: "b".to_owned(),
                    param_type: "uint32".to_owned(),
                    ..Default::default()
                }
            ],
            data: json!({
                "a": 123,
                "b": "456",
            }),
            ..Default::default()
        },
    ).unwrap().boc;

    let result = multisig.send_grams(
        &client,
        "0:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        500_000_000,
        true,
        0,
        Some(payload),
    ).await.unwrap();

    let mut transaction_tree = ton_client::net::query_transaction_tree(
        client.context(),
        ton_client::net::ParamsOfQueryTransactionTree {
            in_msg: result.transaction["in_msg"].as_str().unwrap().to_owned(),
            ..Default::default()
        }
    ).await.unwrap();

    let bounced_id = transaction_tree.messages.pop().unwrap().id;
    let message = &ton_client::net::query_collection(
        client.context(),
        ton_client::net::ParamsOfQueryCollection {
            collection: "messages".to_owned(),
            result: "body bounced".to_owned(),
            filter: Some(json!({ "id": { "eq": bounced_id }})),
            ..Default::default()
        }
    ).await.unwrap().result[0];

    assert_eq!(message["bounced"], Value::Bool(true));

    let bounced_body = ton_client::abi::encode_boc(
        client.context(),
        ton_client::abi::ParamsOfAbiEncodeBoc {
            params: vec![
                ton_client::abi::AbiParam {
                    components: vec![],
                    name: "a".to_owned(),
                    param_type: "int32".to_owned(),
                    ..Default::default()
                },
                ton_client::abi::AbiParam {
                    components: vec![],
                    name: "b".to_owned(),
                    param_type: "uint256".to_owned(),
                    ..Default::default()
                },
                ton_client::abi::AbiParam {
                    name: "c".to_owned(),
                    param_type: "ref(tuple)".to_owned(),
                    components: vec![
                        ton_client::abi::AbiParam {
                            components: vec![],
                            name: "a".to_owned(),
                            param_type: "uint256".to_owned(),
                            ..Default::default()
                        },
                        ton_client::abi::AbiParam {
                            components: vec![],
                            name: "b".to_owned(),
                            param_type: "uint32".to_owned(),
                            ..Default::default()
                        }
                    ],
                    ..Default::default()
                },
            ],
            data: json!({
                "a": -1,
                "b": 123,
                "c": {
                    "a": 123,
                    "b": "456",
                },
            }),
            ..Default::default()
        },
    ).unwrap().boc;

    assert_eq!(message["body"].as_str().unwrap(), bounced_body);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_copyleft() {
    let client = Client::new();

    let keys = Client::generate_keypair();
    let (tvc, abi) = TestUtils::load_contract("Copyleft").unwrap();
    let (addr, _output) = TestUtils::deploy_with_giver(
        &client,
        abi.clone(),
        tvc,
        0,
        CallSet::some_with_function("constructor"),
        &keys,
        10_000_000_000,
    )
    .await
    .unwrap();

    client.call_contract(&addr, abi, "touch", json!({}), Some(&keys)).await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn advanced_test_msg_order() {
    let client = Client::new();

    let keys = Client::generate_keypair();
    let (alpha_tvc, alpha_abi) = TestUtils::load_contract("Alpha").unwrap();
    let (beta_tvc, beta_abi) = TestUtils::load_contract("Beta").unwrap();
    let (gamma_tvc, gamma_abi) = TestUtils::load_contract("Gamma").unwrap();
    
    let (gamma_addr, _output) = TestUtils::deploy_with_giver(
        &client,
        gamma_abi.clone(),
        gamma_tvc,
        0,
        CallSet::some_with_function("constructor"),
        &keys,
        10_000_000_000,
    )
    .await
    .unwrap();

    let (beta_addr, _output) = TestUtils::deploy_with_giver(
        &client,
        beta_abi.clone(),
        beta_tvc,
        0,
        CallSet::some_with_function_and_input("constructor", json!({"gamma_addr": gamma_addr})),
        &keys,
        10_000_000_000,
    )
    .await
    .unwrap();
    
    let (alpha_addr, _output) = TestUtils::deploy_with_giver(
        &client,
        alpha_abi.clone(),
        alpha_tvc,
        0,
        CallSet::some_with_function_and_input("constructor", json!({"beta_addr": beta_addr})),
        &keys,
        1000_000_000_000,
    )
    .await
    .unwrap();

    let count = 100;

    let message = client
        .encode_message(
            alpha_abi.clone(),
            Some(alpha_addr),
            None,
            CallSet::some_with_function_and_input("massTrigger", json!({"count": count})),
            Some(&keys),
        )
        .await
        .unwrap();

    client.send_message_and_wait(Some(alpha_abi), message.message).await.unwrap();

    let result = ton_client::net::query_transaction_tree(
        client.context(),
        ton_client::net::ParamsOfQueryTransactionTree {
            in_msg: message.message_id,
            transaction_max_count: Some(0),
            ..Default::default()
        }
    ).await.unwrap();

    assert_eq!(count * 3 + 1, result.transactions.len());

    for tr in result.transactions {
        assert!(!tr.aborted);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_transaction_trace() {
    let client = Client::new();

    let keys = Client::generate_keypair();
    let giver = Giver::new();
    let result = client
        .call_contract(
            Giver::address(),
            giver.abi().clone(),
            "sendTransaction",
            json!({
                "dest": Giver::address(),
                "value": 123456,
                "bounce": false,
            }),
            Some(&keys),
        )
        .await
        .unwrap_err()
        .downcast::<ton_client::error::ClientError>().unwrap();

    let trace = query_collection(
            client.context(),
            ParamsOfQueryCollection {
                collection: "transactions".to_string(),
                filter: Some(json!({ "id": { "eq": result.data["transaction_id"] } })),
                result: "trace { step }".to_string(),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .result;

    assert!(trace[0]["trace"].is_array())
}

async fn query_last_seq_no(client: &Client) -> u64 {
    query_collection(
        client.context(),
        ParamsOfQueryCollection {
            collection: "blocks".to_string(),
            order: Some(vec![OrderBy { path: "seq_no".to_owned(), direction: SortDirection::DESC }]),
            limit: Some(1),
            result: "seq_no".to_string(),
            ..Default::default()
        },
    )
        .await
        .unwrap()
        .result[0]["seq_no"]
        .as_u64()
        .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn test_reset() {
    let client = Client::new();
    let giver = Giver::new();
    
    giver
        .send_grams(&client, Giver::address(), 500_000_000)
        .await
        .unwrap();

    let seq_no = query_last_seq_no(&client).await;

    se("reset").await;
    tokio::time::sleep(Duration::from_secs(1)).await;

    let reset_seq_no = query_last_seq_no(&client).await;

    assert!(seq_no > reset_seq_no);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_fork() {
    se("reset").await;
    tokio::time::sleep(Duration::from_secs(1)).await;

    let send_msg = || async move {
        let client = Client::new();
        let msg = ever_block::Message::with_ext_in_header(ever_block::ExternalInboundMessageHeader { 
            src: Default::default(),
            dst: "-1:0000000000000000000000000000000000000000000000000000000000000000".parse().unwrap(),
            import_fee: Default::default(),
        }).write_to_bytes().unwrap();
        let msg = base64::encode(&msg);

        let err = client.send_message_and_wait(None, msg.clone()).await.unwrap_err().downcast::<ton_client::error::ClientError>().unwrap();
        query_collection(
            client.context(),
            ParamsOfQueryCollection {
                collection: "transactions".to_string(),
                filter: Some(json!({ "id": { "eq": err.data["transaction_id"] } })),
                result: "orig_status end_status".to_string(),
                ..Default::default()
            },
        )
            .await
            .unwrap()
            .result
            .pop()
            .unwrap()
    };
    
    let transaction = send_msg().await;
    assert_eq!(transaction["orig_status"].as_u64(), Some(3));
    assert_eq!(transaction["end_status"].as_u64(), Some(3));

    se("fork?endpoint=https://mainnet.evercloud.dev/1467de01155143d5ba129ab5ea4ede1f/graphql?resetData=false").await;
    tokio::time::sleep(Duration::from_secs(1)).await;

    let transaction = send_msg().await;
    assert_eq!(transaction["orig_status"].as_u64(), Some(1));
    assert_eq!(transaction["end_status"].as_u64(), Some(1));

    se("unfork?resetData=true").await;
    tokio::time::sleep(Duration::from_secs(1)).await;

    let transaction = send_msg().await;
    assert_eq!(transaction["orig_status"].as_u64(), Some(3));
    assert_eq!(transaction["end_status"].as_u64(), Some(3));
}
