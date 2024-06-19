use super::docker_tests_common::{random_secp256k1_secret, ERC1155_TEST_ABI, ERC721_TEST_ABI, GETH_ACCOUNT,
                                 GETH_ERC1155_CONTRACT, GETH_ERC20_CONTRACT, GETH_ERC721_CONTRACT,
                                 GETH_NFT_SWAP_CONTRACT, GETH_NONCE_LOCK, GETH_RPC_URL, GETH_SWAP_CONTRACT,
                                 GETH_WATCHERS_SWAP_CONTRACT, GETH_WEB3, MM_CTX};
use bitcrypto::{dhash160, sha256};
use coins::eth::{checksum_address, eth_addr_to_hex, eth_coin_from_conf_and_request, EthCoin, ERC20_ABI};
use coins::nft::nft_structs::{Chain, ContractType, NftInfo};
use coins::{CoinProtocol, CoinWithDerivationMethod, ConfirmPaymentInput, DerivationMethod, Eip1559Ops,
            FoundSwapTxSpend, MakerNftSwapOpsV2, MarketCoinOps, NftSwapInfo, ParseCoinAssocTypes, PrivKeyBuildPolicy,
            RefundPaymentArgs, SearchForSwapTxSpendInput, SendNftMakerPaymentArgs, SendPaymentArgs,
            SpendNftMakerPaymentArgs, SpendPaymentArgs, SwapOps, SwapTxFeePolicy, SwapTxTypeWithSecretHash, ToBytes,
            Transaction, ValidateNftMakerPaymentArgs};
use common::{block_on, now_sec};
use crypto::Secp256k1Secret;
use ethereum_types::U256;
use futures01::Future;
use mm2_number::{BigDecimal, BigUint};
use mm2_test_helpers::for_tests::{erc20_dev_conf, eth_dev_conf, nft_dev_conf};
use std::thread;
use std::time::Duration;
use web3::contract::{Contract, Options};
use web3::ethabi::Token;
use web3::types::{Address, TransactionRequest, H256};

/// # Safety
///
/// GETH_ACCOUNT is set once during initialization before tests start
pub fn geth_account() -> Address { unsafe { GETH_ACCOUNT } }

/// # Safety
///
/// GETH_SWAP_CONTRACT is set once during initialization before tests start
pub fn swap_contract() -> Address { unsafe { GETH_SWAP_CONTRACT } }

/// # Safety
///
/// GETH_NFT_SWAP_CONTRACT is set once during initialization before tests start
pub fn nft_swap_contract() -> Address { unsafe { GETH_NFT_SWAP_CONTRACT } }

/// # Safety
///
/// GETH_WATCHERS_SWAP_CONTRACT is set once during initialization before tests start
pub fn watchers_swap_contract() -> Address { unsafe { GETH_WATCHERS_SWAP_CONTRACT } }

/// # Safety
///
/// GETH_ERC20_CONTRACT is set once during initialization before tests start
pub fn erc20_contract() -> Address { unsafe { GETH_ERC20_CONTRACT } }

/// Return ERC20 dev token contract address in checksum format
pub fn erc20_contract_checksum() -> String { checksum_address(&format!("{:02x}", erc20_contract())) }

/// # Safety
///
/// GETH_ERC721_CONTRACT is set once during initialization before tests start
pub fn erc721_contract() -> Address { unsafe { GETH_ERC721_CONTRACT } }

/// # Safety
///
/// GETH_ERC1155_CONTRACT is set once during initialization before tests start
pub fn erc1155_contract() -> Address { unsafe { GETH_ERC1155_CONTRACT } }

fn wait_for_confirmation(tx_hash: H256) {
    thread::sleep(Duration::from_millis(2000));
    loop {
        match block_on(GETH_WEB3.eth().transaction_receipt(tx_hash)) {
            Ok(Some(r)) => match r.block_hash {
                Some(_) => break,
                None => thread::sleep(Duration::from_millis(100)),
            },
            _ => {
                thread::sleep(Duration::from_millis(100));
            },
        }
    }
}

pub fn fill_eth(to_addr: Address, amount: U256) {
    let _guard = GETH_NONCE_LOCK.lock().unwrap();
    let tx_request = TransactionRequest {
        from: geth_account(),
        to: Some(to_addr),
        gas: None,
        gas_price: None,
        value: Some(amount),
        data: None,
        nonce: None,
        condition: None,
        transaction_type: None,
        access_list: None,
        max_fee_per_gas: None,
        max_priority_fee_per_gas: None,
    };
    let tx_hash = block_on(GETH_WEB3.eth().send_transaction(tx_request)).unwrap();
    wait_for_confirmation(tx_hash);
}

fn fill_erc20(to_addr: Address, amount: U256) {
    let _guard = GETH_NONCE_LOCK.lock().unwrap();
    let erc20_contract = Contract::from_json(GETH_WEB3.eth(), erc20_contract(), ERC20_ABI.as_bytes()).unwrap();

    let tx_hash = block_on(erc20_contract.call(
        "transfer",
        (Token::Address(to_addr), Token::Uint(amount)),
        geth_account(),
        Options::default(),
    ))
    .unwrap();
    wait_for_confirmation(tx_hash);
}

pub(crate) fn mint_erc721(to_addr: Address, token_id: U256) {
    let _guard = GETH_NONCE_LOCK.lock().unwrap();
    let erc721_contract = Contract::from_json(GETH_WEB3.eth(), erc721_contract(), ERC721_TEST_ABI.as_bytes()).unwrap();

    let options = Options {
        gas: Some(U256::from(150_000)),
        ..Options::default()
    };

    let tx_hash = block_on(erc721_contract.call(
        "mint",
        (Token::Address(to_addr), Token::Uint(token_id)),
        geth_account(),
        options,
    ))
    .unwrap();
    wait_for_confirmation(tx_hash);

    let owner: Address =
        block_on(erc721_contract.query("ownerOf", Token::Uint(token_id), None, Options::default(), None)).unwrap();

    assert_eq!(
        owner, to_addr,
        "The ownership of the tokenID {:?} does not match the expected address {:?}.",
        token_id, to_addr
    );
}

fn erc712_owner(token_id: U256) -> Address {
    let _guard = GETH_NONCE_LOCK.lock().unwrap();
    let erc721_contract = Contract::from_json(GETH_WEB3.eth(), erc721_contract(), ERC721_TEST_ABI.as_bytes()).unwrap();
    block_on(erc721_contract.query("ownerOf", Token::Uint(token_id), None, Options::default(), None)).unwrap()
}

pub(crate) fn mint_erc1155(to_addr: Address, token_id: U256, amount: U256) {
    let _guard = GETH_NONCE_LOCK.lock().unwrap();
    let erc1155_contract =
        Contract::from_json(GETH_WEB3.eth(), erc1155_contract(), ERC1155_TEST_ABI.as_bytes()).unwrap();

    let tx_hash = block_on(erc1155_contract.call(
        "mint",
        (
            Token::Address(to_addr),
            Token::Uint(token_id),
            Token::Uint(amount),
            Token::Bytes("".into()),
        ),
        geth_account(),
        Options::default(),
    ))
    .unwrap();
    wait_for_confirmation(tx_hash);

    // Check the balance of the token for the to_addr
    let balance: U256 = block_on(erc1155_contract.query(
        "balanceOf",
        (Token::Address(to_addr), Token::Uint(token_id)),
        None,
        Options::default(),
        None,
    ))
    .unwrap();

    assert_eq!(
        balance, amount,
        "The balance of tokenId {:?} for address {:?} does not match the expected amount {:?}.",
        token_id, to_addr, amount
    );
}

fn erc1155_balance(wallet_addr: Address, token_id: U256) -> U256 {
    let _guard = GETH_NONCE_LOCK.lock().unwrap();
    let erc1155_contract =
        Contract::from_json(GETH_WEB3.eth(), erc1155_contract(), ERC1155_TEST_ABI.as_bytes()).unwrap();
    block_on(erc1155_contract.query(
        "balanceOf",
        (Token::Address(wallet_addr), Token::Uint(token_id)),
        None,
        Options::default(),
        None,
    ))
    .unwrap()
}

pub(crate) async fn fill_erc1155_info(eth_coin: &EthCoin, tokens_id: u32, amount: u32) {
    let nft_infos_lock = eth_coin.nfts_infos.clone();
    let mut nft_infos = nft_infos_lock.lock().await;

    let erc1155_nft_info = NftInfo {
        token_address: erc1155_contract(),
        token_id: BigUint::from(tokens_id),
        chain: Chain::Eth,
        contract_type: ContractType::Erc1155,
        amount: BigDecimal::from(amount),
    };
    let erc1155_address_str = eth_addr_to_hex(&erc1155_contract());
    let erc1155_key = format!("{},{}", erc1155_address_str, tokens_id);
    nft_infos.insert(erc1155_key, erc1155_nft_info);
}

pub(crate) async fn fill_erc721_info(eth_coin: &EthCoin, tokens_id: u32) {
    let nft_infos_lock = eth_coin.nfts_infos.clone();
    let mut nft_infos = nft_infos_lock.lock().await;

    let erc721_nft_info = NftInfo {
        token_address: erc721_contract(),
        token_id: BigUint::from(tokens_id),
        chain: Chain::Eth,
        contract_type: ContractType::Erc721,
        amount: BigDecimal::from(1),
    };
    let erc721_address_str = eth_addr_to_hex(&erc721_contract());
    let erc721_key = format!("{},{}", erc721_address_str, tokens_id);
    nft_infos.insert(erc721_key, erc721_nft_info);
}

/// Creates ETH protocol coin supplied with 100 ETH
pub fn eth_coin_with_random_privkey_using_urls(swap_contract_address: Address, urls: &[&str]) -> EthCoin {
    let eth_conf = eth_dev_conf();
    let req = json!({
        "method": "enable",
        "coin": "ETH",
        "swap_contract_address": swap_contract_address,
        "urls": urls,
    });

    let secret = random_secp256k1_secret();
    let eth_coin = block_on(eth_coin_from_conf_and_request(
        &MM_CTX,
        "ETH",
        &eth_conf,
        &req,
        CoinProtocol::ETH,
        PrivKeyBuildPolicy::IguanaPrivKey(secret),
    ))
    .unwrap();

    let my_address = match eth_coin.derivation_method() {
        DerivationMethod::SingleAddress(addr) => *addr,
        _ => panic!("Expected single address"),
    };

    // 100 ETH
    fill_eth(my_address, U256::from(10).pow(U256::from(20)));

    eth_coin
}

/// Creates ETH protocol coin supplied with 100 ETH, using the default GETH_RPC_URL
pub fn eth_coin_with_random_privkey(swap_contract_address: Address) -> EthCoin {
    eth_coin_with_random_privkey_using_urls(swap_contract_address, &[GETH_RPC_URL])
}

/// Creates ERC20 protocol coin supplied with 1 ETH and 100 token
pub fn erc20_coin_with_random_privkey(swap_contract_address: Address) -> EthCoin {
    let erc20_conf = erc20_dev_conf(&erc20_contract_checksum());
    let req = json!({
        "method": "enable",
        "coin": "ERC20DEV",
        "swap_contract_address": swap_contract_address,
        "urls": [GETH_RPC_URL],
    });

    let erc20_coin = block_on(eth_coin_from_conf_and_request(
        &MM_CTX,
        "ERC20DEV",
        &erc20_conf,
        &req,
        CoinProtocol::ERC20 {
            platform: "ETH".to_string(),
            contract_address: checksum_address(&format!("{:02x}", erc20_contract())),
        },
        PrivKeyBuildPolicy::IguanaPrivKey(random_secp256k1_secret()),
    ))
    .unwrap();

    let my_address = match erc20_coin.derivation_method() {
        DerivationMethod::SingleAddress(addr) => *addr,
        _ => panic!("Expected single address"),
    };

    // 1 ETH
    fill_eth(my_address, U256::from(10).pow(U256::from(18)));
    // 100 tokens (it has 8 decimals)
    fill_erc20(my_address, U256::from(10000000000u64));

    erc20_coin
}

pub enum TestNftType {
    Erc1155 { token_id: u32, amount: u32 },
    Erc721 { token_id: u32 },
}

/// Generates a global NFT coin instance with a random private key and an initial 100 ETH balance.
/// Optionally mints a specified NFT (either ERC721 or ERC1155) to the global NFT address,
/// with details recorded in the `nfts_infos` field based on the provided `nft_type`.
pub fn global_nft_with_random_privkey(swap_contract_address: Address, nft_type: Option<TestNftType>) -> EthCoin {
    let nft_conf = nft_dev_conf();
    let req = json!({
        "method": "enable",
        "coin": "NFT_ETH",
        "urls": [GETH_RPC_URL],
        "swap_contract_address": swap_contract_address,
    });

    let global_nft = block_on(eth_coin_from_conf_and_request(
        &MM_CTX,
        "NFT_ETH",
        &nft_conf,
        &req,
        CoinProtocol::NFT {
            platform: "ETH".to_string(),
        },
        PrivKeyBuildPolicy::IguanaPrivKey(random_secp256k1_secret()),
    ))
    .unwrap();

    let my_address = block_on(global_nft.my_addr());
    fill_eth(my_address, U256::from(10).pow(U256::from(20)));

    if let Some(nft_type) = nft_type {
        match nft_type {
            TestNftType::Erc1155 { token_id, amount } => {
                mint_erc1155(my_address, U256::from(token_id), U256::from(amount));
                block_on(fill_erc1155_info(&global_nft, token_id, amount));
            },
            TestNftType::Erc721 { token_id } => {
                mint_erc721(my_address, U256::from(token_id));
                block_on(fill_erc721_info(&global_nft, token_id));
            },
        }
    }

    global_nft
}

/// Fills the private key's public address with ETH and ERC20 tokens
pub fn fill_eth_erc20_with_private_key(priv_key: Secp256k1Secret) {
    let eth_conf = eth_dev_conf();
    let req = json!({
        "coin": "ETH",
        "urls": [GETH_RPC_URL],
        "swap_contract_address": swap_contract(),
    });

    let eth_coin = block_on(eth_coin_from_conf_and_request(
        &MM_CTX,
        "ETH",
        &eth_conf,
        &req,
        CoinProtocol::ETH,
        PrivKeyBuildPolicy::IguanaPrivKey(priv_key),
    ))
    .unwrap();
    let my_address = block_on(eth_coin.derivation_method().single_addr_or_err()).unwrap();

    // 100 ETH
    fill_eth(my_address, U256::from(10).pow(U256::from(20)));

    let erc20_conf = erc20_dev_conf(&erc20_contract_checksum());
    let req = json!({
        "method": "enable",
        "coin": "ERC20DEV",
        "urls": [GETH_RPC_URL],
        "swap_contract_address": swap_contract(),
    });

    let _erc20_coin = block_on(eth_coin_from_conf_and_request(
        &MM_CTX,
        "ERC20DEV",
        &erc20_conf,
        &req,
        CoinProtocol::ERC20 {
            platform: "ETH".to_string(),
            contract_address: erc20_contract_checksum(),
        },
        PrivKeyBuildPolicy::IguanaPrivKey(priv_key),
    ))
    .unwrap();

    // 100 tokens (it has 8 decimals)
    fill_erc20(my_address, U256::from(10000000000u64));
}

fn send_and_refund_eth_maker_payment_impl(swap_txfee_policy: SwapTxFeePolicy) {
    let eth_coin = eth_coin_with_random_privkey(swap_contract());
    eth_coin.set_swap_transaction_fee_policy(swap_txfee_policy);

    let time_lock = now_sec() - 100;
    let other_pubkey = &[
        0x02, 0xc6, 0x6e, 0x7d, 0x89, 0x66, 0xb5, 0xc5, 0x55, 0xaf, 0x58, 0x05, 0x98, 0x9d, 0xa9, 0xfb, 0xf8, 0xdb,
        0x95, 0xe1, 0x56, 0x31, 0xce, 0x35, 0x8c, 0x3a, 0x17, 0x10, 0xc9, 0x62, 0x67, 0x90, 0x63,
    ];

    let send_payment_args = SendPaymentArgs {
        time_lock_duration: 100,
        time_lock,
        other_pubkey,
        secret_hash: &[0; 20],
        amount: 1.into(),
        swap_contract_address: &Some(swap_contract().as_bytes().into()),
        swap_unique_data: &[],
        payment_instructions: &None,
        watcher_reward: None,
        wait_for_confirmation_until: 0,
    };
    let eth_maker_payment = eth_coin.send_maker_payment(send_payment_args).wait().unwrap();

    let confirm_input = ConfirmPaymentInput {
        payment_tx: eth_maker_payment.tx_hex(),
        confirmations: 1,
        requires_nota: false,
        wait_until: now_sec() + 60,
        check_every: 1,
    };
    eth_coin.wait_for_confirmations(confirm_input).wait().unwrap();

    let refund_args = RefundPaymentArgs {
        payment_tx: &eth_maker_payment.tx_hex(),
        time_lock,
        other_pubkey,
        tx_type_with_secret_hash: SwapTxTypeWithSecretHash::TakerOrMakerPayment {
            maker_secret_hash: &[0; 20],
        },
        swap_contract_address: &Some(swap_contract().as_bytes().into()),
        swap_unique_data: &[],
        watcher_reward: false,
    };
    let payment_refund = block_on(eth_coin.send_maker_refunds_payment(refund_args)).unwrap();
    log!("Payment refund tx hash {:02x}", payment_refund.tx_hash_as_bytes());

    let confirm_input = ConfirmPaymentInput {
        payment_tx: payment_refund.tx_hex(),
        confirmations: 1,
        requires_nota: false,
        wait_until: now_sec() + 60,
        check_every: 1,
    };
    eth_coin.wait_for_confirmations(confirm_input).wait().unwrap();

    let search_input = SearchForSwapTxSpendInput {
        time_lock,
        other_pub: other_pubkey,
        secret_hash: &[0; 20],
        tx: &eth_maker_payment.tx_hex(),
        search_from_block: 0,
        swap_contract_address: &Some(swap_contract().as_bytes().into()),
        swap_unique_data: &[],
        watcher_reward: false,
    };
    let search_tx = block_on(eth_coin.search_for_swap_tx_spend_my(search_input))
        .unwrap()
        .unwrap();

    let expected = FoundSwapTxSpend::Refunded(payment_refund);
    assert_eq!(expected, search_tx);
}

#[test]
fn send_and_refund_eth_maker_payment_internal_gas_policy() {
    send_and_refund_eth_maker_payment_impl(SwapTxFeePolicy::Internal);
}

#[test]
fn send_and_refund_eth_maker_payment_priority_fee() { send_and_refund_eth_maker_payment_impl(SwapTxFeePolicy::Medium); }

fn send_and_spend_eth_maker_payment_impl(swap_txfee_policy: SwapTxFeePolicy) {
    let maker_eth_coin = eth_coin_with_random_privkey(swap_contract());
    let taker_eth_coin = eth_coin_with_random_privkey(swap_contract());

    maker_eth_coin.set_swap_transaction_fee_policy(swap_txfee_policy.clone());
    taker_eth_coin.set_swap_transaction_fee_policy(swap_txfee_policy);

    let time_lock = now_sec() + 1000;
    let maker_pubkey = maker_eth_coin.derive_htlc_pubkey(&[]);
    let taker_pubkey = taker_eth_coin.derive_htlc_pubkey(&[]);
    let secret = &[1; 32];
    let secret_hash_owned = dhash160(secret);
    let secret_hash = secret_hash_owned.as_slice();

    let send_payment_args = SendPaymentArgs {
        time_lock_duration: 1000,
        time_lock,
        other_pubkey: &taker_pubkey,
        secret_hash,
        amount: 1.into(),
        swap_contract_address: &Some(swap_contract().as_bytes().into()),
        swap_unique_data: &[],
        payment_instructions: &None,
        watcher_reward: None,
        wait_for_confirmation_until: 0,
    };
    let eth_maker_payment = maker_eth_coin.send_maker_payment(send_payment_args).wait().unwrap();

    let confirm_input = ConfirmPaymentInput {
        payment_tx: eth_maker_payment.tx_hex(),
        confirmations: 1,
        requires_nota: false,
        wait_until: now_sec() + 60,
        check_every: 1,
    };
    taker_eth_coin.wait_for_confirmations(confirm_input).wait().unwrap();

    let spend_args = SpendPaymentArgs {
        other_payment_tx: &eth_maker_payment.tx_hex(),
        time_lock,
        other_pubkey: &maker_pubkey,
        secret,
        secret_hash,
        swap_contract_address: &Some(swap_contract().as_bytes().into()),
        swap_unique_data: &[],
        watcher_reward: false,
    };
    let payment_spend = block_on(taker_eth_coin.send_taker_spends_maker_payment(spend_args)).unwrap();
    log!("Payment spend tx hash {:02x}", payment_spend.tx_hash_as_bytes());

    let confirm_input = ConfirmPaymentInput {
        payment_tx: payment_spend.tx_hex(),
        confirmations: 1,
        requires_nota: false,
        wait_until: now_sec() + 60,
        check_every: 1,
    };
    taker_eth_coin.wait_for_confirmations(confirm_input).wait().unwrap();

    let search_input = SearchForSwapTxSpendInput {
        time_lock,
        other_pub: &taker_pubkey,
        secret_hash,
        tx: &eth_maker_payment.tx_hex(),
        search_from_block: 0,
        swap_contract_address: &Some(swap_contract().as_bytes().into()),
        swap_unique_data: &[],
        watcher_reward: false,
    };
    let search_tx = block_on(maker_eth_coin.search_for_swap_tx_spend_my(search_input))
        .unwrap()
        .unwrap();

    let expected = FoundSwapTxSpend::Spent(payment_spend);
    assert_eq!(expected, search_tx);
}

#[test]
fn send_and_spend_eth_maker_payment_internal_gas_policy() {
    send_and_spend_eth_maker_payment_impl(SwapTxFeePolicy::Internal);
}

#[test]
fn send_and_spend_eth_maker_payment_priority_fee() { send_and_spend_eth_maker_payment_impl(SwapTxFeePolicy::Medium); }

fn send_and_refund_erc20_maker_payment_impl(swap_txfee_policy: SwapTxFeePolicy) {
    let erc20_coin = erc20_coin_with_random_privkey(swap_contract());
    erc20_coin.set_swap_transaction_fee_policy(swap_txfee_policy);

    let time_lock = now_sec() - 100;
    let other_pubkey = &[
        0x02, 0xc6, 0x6e, 0x7d, 0x89, 0x66, 0xb5, 0xc5, 0x55, 0xaf, 0x58, 0x05, 0x98, 0x9d, 0xa9, 0xfb, 0xf8, 0xdb,
        0x95, 0xe1, 0x56, 0x31, 0xce, 0x35, 0x8c, 0x3a, 0x17, 0x10, 0xc9, 0x62, 0x67, 0x90, 0x63,
    ];
    let secret_hash = &[1; 20];

    let send_payment_args = SendPaymentArgs {
        time_lock_duration: 100,
        time_lock,
        other_pubkey,
        secret_hash,
        amount: 1.into(),
        swap_contract_address: &Some(swap_contract().as_bytes().into()),
        swap_unique_data: &[],
        payment_instructions: &None,
        watcher_reward: None,
        wait_for_confirmation_until: now_sec() + 60,
    };
    let eth_maker_payment = erc20_coin.send_maker_payment(send_payment_args).wait().unwrap();

    let confirm_input = ConfirmPaymentInput {
        payment_tx: eth_maker_payment.tx_hex(),
        confirmations: 1,
        requires_nota: false,
        wait_until: now_sec() + 60,
        check_every: 1,
    };
    erc20_coin.wait_for_confirmations(confirm_input).wait().unwrap();

    let refund_args = RefundPaymentArgs {
        payment_tx: &eth_maker_payment.tx_hex(),
        time_lock,
        other_pubkey,
        tx_type_with_secret_hash: SwapTxTypeWithSecretHash::TakerOrMakerPayment {
            maker_secret_hash: secret_hash,
        },
        swap_contract_address: &Some(swap_contract().as_bytes().into()),
        swap_unique_data: &[],
        watcher_reward: false,
    };
    let payment_refund = block_on(erc20_coin.send_maker_refunds_payment(refund_args)).unwrap();
    log!("Payment refund tx hash {:02x}", payment_refund.tx_hash_as_bytes());

    let confirm_input = ConfirmPaymentInput {
        payment_tx: payment_refund.tx_hex(),
        confirmations: 1,
        requires_nota: false,
        wait_until: now_sec() + 60,
        check_every: 1,
    };
    erc20_coin.wait_for_confirmations(confirm_input).wait().unwrap();

    let search_input = SearchForSwapTxSpendInput {
        time_lock,
        other_pub: other_pubkey,
        secret_hash,
        tx: &eth_maker_payment.tx_hex(),
        search_from_block: 0,
        swap_contract_address: &Some(swap_contract().as_bytes().into()),
        swap_unique_data: &[],
        watcher_reward: false,
    };
    let search_tx = block_on(erc20_coin.search_for_swap_tx_spend_my(search_input))
        .unwrap()
        .unwrap();

    let expected = FoundSwapTxSpend::Refunded(payment_refund);
    assert_eq!(expected, search_tx);
}

#[test]
fn send_and_refund_erc20_maker_payment_internal_gas_policy() {
    send_and_refund_erc20_maker_payment_impl(SwapTxFeePolicy::Internal);
}

#[test]
fn send_and_refund_erc20_maker_payment_priority_fee() {
    send_and_refund_erc20_maker_payment_impl(SwapTxFeePolicy::Medium);
}

fn send_and_spend_erc20_maker_payment_impl(swap_txfee_policy: SwapTxFeePolicy) {
    let maker_erc20_coin = erc20_coin_with_random_privkey(swap_contract());
    let taker_erc20_coin = erc20_coin_with_random_privkey(swap_contract());

    maker_erc20_coin.set_swap_transaction_fee_policy(swap_txfee_policy.clone());
    taker_erc20_coin.set_swap_transaction_fee_policy(swap_txfee_policy);

    let time_lock = now_sec() + 1000;
    let maker_pubkey = maker_erc20_coin.derive_htlc_pubkey(&[]);
    let taker_pubkey = taker_erc20_coin.derive_htlc_pubkey(&[]);
    let secret = &[2; 32];
    let secret_hash_owned = dhash160(secret);
    let secret_hash = secret_hash_owned.as_slice();

    let send_payment_args = SendPaymentArgs {
        time_lock_duration: 1000,
        time_lock,
        other_pubkey: &taker_pubkey,
        secret_hash,
        amount: 1.into(),
        swap_contract_address: &Some(swap_contract().as_bytes().into()),
        swap_unique_data: &[],
        payment_instructions: &None,
        watcher_reward: None,
        wait_for_confirmation_until: now_sec() + 60,
    };
    let eth_maker_payment = maker_erc20_coin.send_maker_payment(send_payment_args).wait().unwrap();

    let confirm_input = ConfirmPaymentInput {
        payment_tx: eth_maker_payment.tx_hex(),
        confirmations: 1,
        requires_nota: false,
        wait_until: now_sec() + 60,
        check_every: 1,
    };
    taker_erc20_coin.wait_for_confirmations(confirm_input).wait().unwrap();

    let spend_args = SpendPaymentArgs {
        other_payment_tx: &eth_maker_payment.tx_hex(),
        time_lock,
        other_pubkey: &maker_pubkey,
        secret,
        secret_hash,
        swap_contract_address: &Some(swap_contract().as_bytes().into()),
        swap_unique_data: &[],
        watcher_reward: false,
    };
    let payment_spend = block_on(taker_erc20_coin.send_taker_spends_maker_payment(spend_args)).unwrap();
    log!("Payment spend tx hash {:02x}", payment_spend.tx_hash_as_bytes());

    let confirm_input = ConfirmPaymentInput {
        payment_tx: payment_spend.tx_hex(),
        confirmations: 1,
        requires_nota: false,
        wait_until: now_sec() + 60,
        check_every: 1,
    };
    taker_erc20_coin.wait_for_confirmations(confirm_input).wait().unwrap();

    let search_input = SearchForSwapTxSpendInput {
        time_lock,
        other_pub: &taker_pubkey,
        secret_hash,
        tx: &eth_maker_payment.tx_hex(),
        search_from_block: 0,
        swap_contract_address: &Some(swap_contract().as_bytes().into()),
        swap_unique_data: &[],
        watcher_reward: false,
    };
    let search_tx = block_on(maker_erc20_coin.search_for_swap_tx_spend_my(search_input))
        .unwrap()
        .unwrap();

    let expected = FoundSwapTxSpend::Spent(payment_spend);
    assert_eq!(expected, search_tx);
}

#[test]
fn send_and_spend_erc20_maker_payment_internal_gas_policy() {
    send_and_spend_erc20_maker_payment_impl(SwapTxFeePolicy::Internal);
}

#[test]
fn send_and_spend_erc20_maker_payment_priority_fee() {
    send_and_spend_erc20_maker_payment_impl(SwapTxFeePolicy::Medium);
}

#[test]
fn send_and_spend_erc721_maker_payment() {
    // TODO: Evaluate implementation strategy — either employing separate contracts for maker and taker
    // functionalities for both coins and NFTs, or utilizing the Diamond Standard (EIP-2535) for a unified contract approach.
    // Decision will inform whether to maintain multiple "swap_contract_address" fields in `EthCoin` for distinct contract types
    // or a singular field for a Diamond Standard-compatible contract address.

    let erc721_nft = TestNftType::Erc721 { token_id: 2 };

    let maker_global_nft = global_nft_with_random_privkey(nft_swap_contract(), Some(erc721_nft));
    let taker_global_nft = global_nft_with_random_privkey(nft_swap_contract(), None);

    let time_lock = now_sec() + 1000;
    let maker_pubkey = maker_global_nft.derive_htlc_pubkey(&[]);
    let taker_pubkey = taker_global_nft.derive_htlc_pubkey(&[]);

    let maker_secret = &[1; 32];
    let maker_secret_hash = sha256(maker_secret).to_vec();

    let nft_swap_info = NftSwapInfo {
        token_address: &erc721_contract(),
        token_id: &BigUint::from(2u32).to_bytes(),
        contract_type: &ContractType::Erc721,
        swap_contract_address: &nft_swap_contract(),
    };

    let send_payment_args: SendNftMakerPaymentArgs<EthCoin> = SendNftMakerPaymentArgs {
        time_lock,
        taker_secret_hash: &[0; 32],
        maker_secret_hash: &maker_secret_hash,
        amount: 1.into(),
        taker_pub: &taker_global_nft.parse_pubkey(&taker_pubkey).unwrap(),
        swap_unique_data: &[],
        nft_swap_info: &nft_swap_info,
    };
    let maker_payment = block_on(maker_global_nft.send_nft_maker_payment_v2(send_payment_args)).unwrap();
    log!(
        "Maker sent ERC721 NFT Payment tx hash {:02x}",
        maker_payment.tx_hash_as_bytes()
    );

    let confirm_input = ConfirmPaymentInput {
        payment_tx: maker_payment.tx_hex(),
        confirmations: 1,
        requires_nota: false,
        wait_until: now_sec() + 70,
        check_every: 1,
    };
    maker_global_nft.wait_for_confirmations(confirm_input).wait().unwrap();

    let validate_args = ValidateNftMakerPaymentArgs {
        maker_payment_tx: &maker_payment,
        time_lock,
        taker_secret_hash: &[0; 32],
        maker_secret_hash: &maker_secret_hash,
        amount: 1.into(),
        taker_pub: &taker_global_nft.parse_pubkey(&taker_pubkey).unwrap(),
        maker_pub: &maker_global_nft.parse_pubkey(&maker_pubkey).unwrap(),
        swap_unique_data: &[],
        nft_swap_info: &nft_swap_info,
    };
    block_on(maker_global_nft.validate_nft_maker_payment_v2(validate_args)).unwrap();

    let spend_payment_args = SpendNftMakerPaymentArgs {
        maker_payment_tx: &maker_payment,
        time_lock,
        taker_secret_hash: &[0; 32],
        maker_secret_hash: &maker_secret_hash,
        maker_secret,
        maker_pub: &maker_global_nft.parse_pubkey(&maker_pubkey).unwrap(),
        swap_unique_data: &[],
        contract_type: &ContractType::Erc721,
        swap_contract_address: &nft_swap_contract(),
    };
    let spend_tx = block_on(taker_global_nft.spend_nft_maker_payment_v2(spend_payment_args)).unwrap();

    let confirm_input = ConfirmPaymentInput {
        payment_tx: spend_tx.tx_hex(),
        confirmations: 1,
        requires_nota: false,
        wait_until: now_sec() + 70,
        check_every: 1,
    };
    taker_global_nft.wait_for_confirmations(confirm_input).wait().unwrap();

    let new_owner = erc712_owner(U256::from(2));
    let my_address = block_on(taker_global_nft.my_addr());
    assert_eq!(new_owner, my_address);
}

#[test]
fn send_and_spend_erc1155_maker_payment() {
    let erc1155_nft = TestNftType::Erc1155 { token_id: 4, amount: 3 };

    let maker_global_nft = global_nft_with_random_privkey(nft_swap_contract(), Some(erc1155_nft));
    let taker_global_nft = global_nft_with_random_privkey(nft_swap_contract(), None);

    let time_lock = now_sec() + 1000;
    let maker_pubkey = maker_global_nft.derive_htlc_pubkey(&[]);
    let taker_pubkey = taker_global_nft.derive_htlc_pubkey(&[]);

    let maker_secret = &[1; 32];
    let maker_secret_hash = sha256(maker_secret).to_vec();

    let nft_swap_info = NftSwapInfo {
        token_address: &erc1155_contract(),
        token_id: &BigUint::from(4u32).to_bytes(),
        contract_type: &ContractType::Erc1155,
        swap_contract_address: &nft_swap_contract(),
    };

    let send_payment_args: SendNftMakerPaymentArgs<EthCoin> = SendNftMakerPaymentArgs {
        time_lock,
        taker_secret_hash: &[0; 32],
        maker_secret_hash: &maker_secret_hash,
        amount: 3.into(),
        taker_pub: &taker_global_nft.parse_pubkey(&taker_pubkey).unwrap(),
        swap_unique_data: &[],
        nft_swap_info: &nft_swap_info,
    };
    let maker_payment = block_on(maker_global_nft.send_nft_maker_payment_v2(send_payment_args)).unwrap();
    log!(
        "Maker sent ERC1155 NFT Payment tx hash {:02x}",
        maker_payment.tx_hash_as_bytes()
    );

    let confirm_input = ConfirmPaymentInput {
        payment_tx: maker_payment.tx_hex(),
        confirmations: 1,
        requires_nota: false,
        wait_until: now_sec() + 60,
        check_every: 1,
    };
    maker_global_nft.wait_for_confirmations(confirm_input).wait().unwrap();

    let validate_args = ValidateNftMakerPaymentArgs {
        maker_payment_tx: &maker_payment,
        time_lock,
        taker_secret_hash: &[0; 32],
        maker_secret_hash: &maker_secret_hash,
        amount: 3.into(),
        taker_pub: &taker_global_nft.parse_pubkey(&taker_pubkey).unwrap(),
        maker_pub: &maker_global_nft.parse_pubkey(&maker_pubkey).unwrap(),
        swap_unique_data: &[],
        nft_swap_info: &nft_swap_info,
    };
    block_on(maker_global_nft.validate_nft_maker_payment_v2(validate_args)).unwrap();

    let spend_payment_args = SpendNftMakerPaymentArgs {
        maker_payment_tx: &maker_payment,
        time_lock,
        taker_secret_hash: &[0; 32],
        maker_secret_hash: &maker_secret_hash,
        maker_secret,
        maker_pub: &maker_global_nft.parse_pubkey(&maker_pubkey).unwrap(),
        swap_unique_data: &[],
        contract_type: &ContractType::Erc1155,
        swap_contract_address: &nft_swap_contract(),
    };
    let spend_tx = block_on(taker_global_nft.spend_nft_maker_payment_v2(spend_payment_args)).unwrap();

    let confirm_input = ConfirmPaymentInput {
        payment_tx: spend_tx.tx_hex(),
        confirmations: 1,
        requires_nota: false,
        wait_until: now_sec() + 60,
        check_every: 1,
    };
    taker_global_nft.wait_for_confirmations(confirm_input).wait().unwrap();

    let my_address = block_on(taker_global_nft.my_addr());
    let balance = erc1155_balance(my_address, U256::from(4));
    assert_eq!(balance, U256::from(3));
}

#[test]
fn test_nonce_several_urls() {
    // Use one working and one failing URL.
    let coin = eth_coin_with_random_privkey_using_urls(swap_contract(), &[GETH_RPC_URL, "http://127.0.0.1:0"]);
    let my_address = block_on(coin.derivation_method().single_addr_or_err()).unwrap();
    let (old_nonce, _) = coin.clone().get_addr_nonce(my_address).wait().unwrap();

    // Send a payment to increase the nonce.
    coin.send_to_address(my_address, 200000000.into()).wait().unwrap();

    let (new_nonce, _) = coin.get_addr_nonce(my_address).wait().unwrap();
    assert_eq!(old_nonce + 1, new_nonce);
}

#[test]
fn test_nonce_lock() {
    use crate::common::Future01CompatExt;
    use futures::future::join_all;

    let coin = eth_coin_with_random_privkey(swap_contract());
    let my_address = block_on(coin.derivation_method().single_addr_or_err()).unwrap();
    let futures = (0..5).map(|_| coin.send_to_address(my_address, 200000000.into()).compat());
    let results = block_on(join_all(futures));

    // make sure all transactions are successful
    for result in results {
        result.unwrap();
    }
}
