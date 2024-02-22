use super::*;
use common::block_on;
use crypto::privkey::key_pair_from_seed;
use ethkey::{Generator, Random};
use mm2_core::mm_ctx::{MmArc, MmCtxBuilder};
use mm2_test_helpers::{for_tests::{eth_jst_testnet_conf, eth_testnet_conf, ETH_DEV_NODES, ETH_DEV_SWAP_CONTRACT,
                                   ETH_DEV_TOKEN_CONTRACT},
                       get_passphrase};

lazy_static! {
    static ref ETH_DISTRIBUTOR: EthCoin = eth_distributor();
    static ref JST_DISTRIBUTOR: EthCoin = jst_distributor();
    static ref MM_CTX: MmArc = MmCtxBuilder::new().into_mm_arc();
}

pub fn eth_distributor() -> EthCoin {
    let req = json!({
        "method": "enable",
        "coin": "ETH",
        "urls": ETH_DEV_NODES,
        "swap_contract_address": ETH_DEV_SWAP_CONTRACT,
    });
    let seed = get_passphrase!(".env.client", "ALICE_PASSPHRASE").unwrap();
    let keypair = key_pair_from_seed(&seed).unwrap();
    let priv_key_policy = PrivKeyBuildPolicy::IguanaPrivKey(keypair.private().secret);
    block_on(eth_coin_from_conf_and_request(
        &MM_CTX,
        "ETH",
        &eth_testnet_conf(),
        &req,
        CoinProtocol::ETH,
        priv_key_policy,
    ))
    .unwrap()
}

pub fn jst_distributor() -> EthCoin {
    let req = json!({
        "method": "enable",
        "coin": "ETH",
        "urls": ETH_DEV_NODES,
        "swap_contract_address": ETH_DEV_SWAP_CONTRACT,
    });
    let seed = get_passphrase!(".env.seed", "BOB_PASSPHRASE").unwrap();
    let keypair = key_pair_from_seed(&seed).unwrap();
    let priv_key_policy = PrivKeyBuildPolicy::IguanaPrivKey(keypair.private().secret);
    block_on(eth_coin_from_conf_and_request(
        &MM_CTX,
        "ETH",
        &eth_testnet_conf(),
        &req,
        CoinProtocol::ERC20 {
            platform: "ETH".to_string(),
            contract_address: ETH_DEV_TOKEN_CONTRACT.to_string(),
        },
        priv_key_policy,
    ))
    .unwrap()
}

pub(crate) fn eth_coin_for_test(
    coin_type: EthCoinType,
    urls: &[&str],
    fallback_swap_contract: Option<Address>,
) -> (MmArc, EthCoin) {
    let key_pair = KeyPair::from_secret_slice(
        &hex::decode("809465b17d0a4ddb3e4c69e8f23c2cabad868f51f8bed5c765ad1d6516c3306f").unwrap(),
    )
    .unwrap();
    eth_coin_from_keypair(coin_type, urls, fallback_swap_contract, key_pair)
}

#[allow(dead_code)]
pub(crate) fn random_eth_coin_for_test(
    coin_type: EthCoinType,
    urls: &[&str],
    fallback_swap_contract: Option<Address>,
) -> (MmArc, EthCoin) {
    let key_pair = Random.generate().unwrap();
    fill_eth(key_pair.address(), 0.001);
    eth_coin_from_keypair(coin_type, urls, fallback_swap_contract, key_pair)
}

fn eth_coin_from_keypair(
    coin_type: EthCoinType,
    urls: &[&str],
    fallback_swap_contract: Option<Address>,
    key_pair: KeyPair,
) -> (MmArc, EthCoin) {
    let mut nodes = vec![];
    for url in urls.iter() {
        nodes.push(HttpTransportNode {
            uri: url.parse().unwrap(),
            gui_auth: false,
        });
    }
    drop_mutability!(nodes);

    let transport = Web3Transport::with_nodes(nodes);
    let web3 = Web3::new(transport);
    let conf = json!({
        "coins":[
            eth_testnet_conf(),
            eth_jst_testnet_conf()
        ]
    });
    let ctx = MmCtxBuilder::new().with_conf(conf).into_mm_arc();
    let ticker = match coin_type {
        EthCoinType::Eth => "ETH".to_string(),
        EthCoinType::Erc20 { .. } => "JST".to_string(),
    };
    let my_address = key_pair.address();

    let eth_coin = EthCoin(Arc::new(EthCoinImpl {
        coin_type,
        decimals: 18,
        gas_station_url: None,
        gas_station_decimals: ETH_GAS_STATION_DECIMALS,
        history_sync_state: Mutex::new(HistorySyncState::NotEnabled),
        gas_station_policy: GasStationPricePolicy::MeanAverageFast,
        sign_message_prefix: Some(String::from("Ethereum Signed Message:\n")),
        priv_key_policy: key_pair.into(),
        derivation_method: Arc::new(DerivationMethod::SingleAddress(my_address)),
        swap_contract_address: Address::from_str(ETH_DEV_SWAP_CONTRACT).unwrap(),
        fallback_swap_contract,
        contract_supports_watchers: false,
        ticker,
        web3_instances: vec![Web3Instance {
            web3: web3.clone(),
            is_parity: false,
        }],
        web3,
        ctx: ctx.weak(),
        required_confirmations: 1.into(),
        chain_id: None,
        trezor_coin: None,
        logs_block_range: DEFAULT_LOGS_BLOCK_RANGE,
        nonce_lock: new_nonce_lock(),
        erc20_tokens_infos: Default::default(),
        abortable_system: AbortableQueue::default(),
    }));
    (ctx, eth_coin)
}

#[allow(dead_code)]
pub(crate) fn fill_eth(to_addr: Address, amount: f64) {
    let wei_per_eth: u64 = 1_000_000_000_000_000_000;
    let amount_in_wei = (amount * wei_per_eth as f64) as u64;
    ETH_DISTRIBUTOR
        .send_to_address(to_addr, amount_in_wei.into())
        .wait()
        .unwrap();
}

#[allow(dead_code)]
pub(crate) fn fill_jst(to_addr: Address, amount: f64) {
    let wei_per_jst: u64 = 1_000_000_000_000_000_000;
    let amount_in_wei = (amount * wei_per_jst as f64) as u64;
    JST_DISTRIBUTOR
        .send_to_address(to_addr, amount_in_wei.into())
        .wait()
        .unwrap();
}
