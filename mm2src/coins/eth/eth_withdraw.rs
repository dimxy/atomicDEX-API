use super::{checksum_address, get_addr_nonce, get_eth_gas_details, u256_to_big_decimal, wei_from_big_decimal,
            EthCoinType, EthDerivationMethod, EthPrivKeyPolicy, Public, WithdrawError, WithdrawRequest,
            WithdrawResult, ERC20_CONTRACT, H160, H256};
use crate::eth::{Action, Address, EthTxFeeDetails, KeyPair, SignedEthTx, UnSignedEthTx};
use crate::hd_wallet::{HDCoinWithdrawOps, HDWalletOps, WithdrawFrom, WithdrawSenderAddress};
use crate::rpc_command::init_withdraw::{WithdrawInProgressStatus, WithdrawTaskHandleShared};
use crate::{BytesJson, CoinWithDerivationMethod, EthCoin, GetWithdrawSenderAddress, PrivKeyPolicy, TransactionDetails};
use async_trait::async_trait;
use bip32::DerivationPath;
use common::custom_futures::timeout::FutureTimerExt;
use common::now_sec;
use crypto::hw_rpc_task::HwRpcTaskAwaitingStatus;
use crypto::trezor::trezor_rpc_task::{TrezorRequestStatuses, TrezorRpcTaskProcessor};
use crypto::{CryptoCtx, HwRpcError};
use ethabi::Token;
use futures::compat::Future01CompatExt;
use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::map_mm_error::MapMmError;
use mm2_err_handle::mm_error::MmResult;
use mm2_err_handle::prelude::{MapToMmResult, MmError, OrMmError};
use std::ops::Deref;
use std::sync::Arc;

#[cfg(target_arch = "wasm32")]
use web3::types::TransactionRequest;

#[async_trait]
pub trait EthWithdraw
where
    Self: Sized + Sync,
{
    fn coin(&self) -> &EthCoin;

    fn request(&self) -> &WithdrawRequest;

    #[allow(clippy::result_large_err)]
    fn on_generating_transaction(&self) -> Result<(), MmError<WithdrawError>>;

    #[allow(clippy::result_large_err)]
    fn on_finishing(&self) -> Result<(), MmError<WithdrawError>>;

    async fn sign_tx_with_trezor(
        &self,
        derivation_path: &DerivationPath,
        unsigned_tx: &UnSignedEthTx,
    ) -> Result<SignedEthTx, MmError<WithdrawError>>;

    async fn get_from_address(&self, req: &WithdrawRequest) -> Result<H160, MmError<WithdrawError>> {
        let coin = self.coin();
        match req.from {
            Some(_) => Ok(coin.get_withdraw_sender_address(req).await?.address),
            None => Ok(coin.derivation_method.single_addr_or_err().await?),
        }
    }

    #[allow(clippy::result_large_err)]
    fn get_key_pair(&self, req: &WithdrawRequest) -> Result<KeyPair, MmError<WithdrawError>> {
        let coin = self.coin();
        if coin.priv_key_policy.is_trezor() {
            return MmError::err(WithdrawError::InternalError("no keypair for hw wallet".to_owned()));
        }

        match req.from {
            Some(ref from) => {
                let derivation_path = self.get_from_derivation_path(from)?;
                let raw_priv_key = coin
                    .priv_key_policy
                    .hd_wallet_derived_priv_key_or_err(&derivation_path)?;
                KeyPair::from_secret_slice(raw_priv_key.as_slice())
                    .map_to_mm(|e| WithdrawError::InternalError(e.to_string()))
            },
            None => coin
                .priv_key_policy
                .activated_key_or_err()
                .mm_err(|e| WithdrawError::InternalError(e.to_string()))
                .cloned(),
        }
    }

    #[allow(clippy::result_large_err)]
    fn get_from_derivation_path(&self, from: &WithdrawFrom) -> Result<DerivationPath, MmError<WithdrawError>> {
        let coin = self.coin();
        let path_to_coin = &coin.deref().derivation_method.hd_wallet_or_err()?.derivation_path;
        let path_to_address = from.to_address_path(path_to_coin.coin_type())?;
        let derivation_path = path_to_address.to_derivation_path(path_to_coin)?;
        Ok(derivation_path)
    }

    async fn get_withdraw_derivation_path(
        &self,
        req: &WithdrawRequest,
    ) -> Result<DerivationPath, MmError<WithdrawError>> {
        let coin = self.coin();
        match req.from {
            Some(ref from) => self.get_from_derivation_path(from),
            None => {
                let default_hd_address = &coin
                    .deref()
                    .derivation_method
                    .hd_wallet()
                    .ok_or(WithdrawError::UnexpectedDerivationMethod)?
                    .get_enabled_address()
                    .await
                    .ok_or_else(|| WithdrawError::InternalError("no enabled address".to_owned()))?;
                Ok(default_hd_address.derivation_path.clone())
            },
        }
    }

    #[allow(unused_variables)]
    async fn sign_withdraw_tx(
        &self,
        req: &WithdrawRequest,
        unsigned_tx: UnSignedEthTx,
    ) -> Result<(H256, BytesJson), MmError<WithdrawError>> {
        let coin = self.coin();
        match coin.priv_key_policy {
            EthPrivKeyPolicy::Iguana(_) | EthPrivKeyPolicy::HDWallet { .. } => {
                let key_pair = self.get_key_pair(req)?;
                let signed = unsigned_tx.sign(key_pair.secret(), coin.chain_id);
                let bytes = rlp::encode(&signed);

                Ok((signed.hash, BytesJson::from(bytes.to_vec())))
            },
            EthPrivKeyPolicy::Trezor => {
                let derivation_path = self.get_withdraw_derivation_path(req).await?;
                let signed = self.sign_tx_with_trezor(&derivation_path, &unsigned_tx).await?;
                let bytes = rlp::encode(&signed);
                Ok((signed.hash, BytesJson::from(bytes.to_vec())))
            },
            #[cfg(target_arch = "wasm32")]
            EthPrivKeyPolicy::Metamask(_) => MmError::err(WithdrawError::InternalError("invalid policy".to_owned())),
        }
    }

    #[cfg(target_arch = "wasm32")]
    async fn send_withdraw_tx(
        &self,
        req: &WithdrawRequest,
        tx_to_send: TransactionRequest,
    ) -> Result<(H256, BytesJson), MmError<WithdrawError>> {
        let coin = self.coin();
        match coin.priv_key_policy {
            EthPrivKeyPolicy::Metamask(_) => {
                if !req.broadcast {
                    let error =
                        "Set 'broadcast' to generate, sign and broadcast a transaction with MetaMask".to_string();
                    return MmError::err(WithdrawError::BroadcastExpected(error));
                }

                // Wait for 10 seconds for the transaction to appear on the RPC node.
                let wait_rpc_timeout = 10_000;
                let check_every = 1.;

                // Please note that this method may take a long time
                // due to `wallet_switchEthereumChain` and `eth_sendTransaction` requests.
                let tx_hash = coin.web3.eth().send_transaction(tx_to_send).await?;

                let signed_tx = coin
                    .wait_for_tx_appears_on_rpc(tx_hash, wait_rpc_timeout, check_every)
                    .await?;
                let tx_hex = signed_tx
                    .map(|signed_tx| BytesJson::from(rlp::encode(&signed_tx).to_vec()))
                    // Return an empty `tx_hex` if the transaction is still not appeared on the RPC node.
                    .unwrap_or_default();
                Ok((tx_hash, tx_hex))
            },
            EthPrivKeyPolicy::Iguana(_) | EthPrivKeyPolicy::HDWallet { .. } | EthPrivKeyPolicy::Trezor => {
                MmError::err(WithdrawError::InternalError("invalid policy".to_owned()))
            },
        }
    }

    async fn build(self) -> WithdrawResult {
        let coin = self.coin();
        let ticker = coin.deref().ticker.clone();
        let req = self.request().clone();

        self.on_generating_transaction()?;

        let to_addr = coin
            .address_from_str(&req.to)
            .map_to_mm(WithdrawError::InvalidAddress)?;
        let my_address = self.get_from_address(&req).await?;
        let my_balance = coin.address_balance(my_address).compat().await?;
        let my_balance_dec = u256_to_big_decimal(my_balance, coin.decimals)?;

        let (mut wei_amount, dec_amount) = if req.max {
            (my_balance, my_balance_dec.clone())
        } else {
            let wei_amount = wei_from_big_decimal(&req.amount, coin.decimals)?;
            (wei_amount, req.amount.clone())
        };
        if wei_amount > my_balance {
            return MmError::err(WithdrawError::NotSufficientBalance {
                coin: coin.ticker.clone(),
                available: my_balance_dec.clone(),
                required: dec_amount,
            });
        };
        let (mut eth_value, data, call_addr, fee_coin) = match &coin.coin_type {
            EthCoinType::Eth => (wei_amount, vec![], to_addr, ticker.as_str()),
            EthCoinType::Erc20 { platform, token_addr } => {
                let function = ERC20_CONTRACT.function("transfer")?;
                let data = function.encode_input(&[Token::Address(to_addr), Token::Uint(wei_amount)])?;
                (0.into(), data, *token_addr, platform.as_str())
            },
        };
        let eth_value_dec = u256_to_big_decimal(eth_value, coin.decimals)?;

        let (gas, gas_price) = get_eth_gas_details(
            coin,
            req.fee.clone(),
            eth_value,
            data.clone().into(),
            my_address,
            call_addr,
            false,
        )
        .await?;
        let total_fee = gas * gas_price;
        let total_fee_dec = u256_to_big_decimal(total_fee, coin.decimals)?;

        if req.max && coin.coin_type == EthCoinType::Eth {
            if eth_value < total_fee || wei_amount < total_fee {
                return MmError::err(WithdrawError::AmountTooLow {
                    amount: eth_value_dec,
                    threshold: total_fee_dec,
                });
            }
            eth_value -= total_fee;
            wei_amount -= total_fee;
        };

        let (tx_hash, tx_hex) = match coin.priv_key_policy {
            EthPrivKeyPolicy::Iguana(_) | EthPrivKeyPolicy::HDWallet { .. } | EthPrivKeyPolicy::Trezor => {
                // Todo: nonce_lock is still global for all addresses but this needs to be per address
                let _nonce_lock = coin.nonce_lock.lock().await;
                let (nonce, _) = get_addr_nonce(my_address, coin.web3_instances.clone())
                    .compat()
                    .timeout_secs(30.)
                    .await?
                    .map_to_mm(WithdrawError::Transport)?;

                let unsigned_tx = UnSignedEthTx {
                    nonce,
                    value: eth_value,
                    action: Action::Call(call_addr),
                    data: data.clone(),
                    gas,
                    gas_price,
                };
                self.sign_withdraw_tx(&req, unsigned_tx).await?
            },
            #[cfg(target_arch = "wasm32")]
            EthPrivKeyPolicy::Metamask(_) => {
                let tx_to_send = TransactionRequest {
                    from: my_address,
                    to: Some(to_addr),
                    gas: Some(gas),
                    gas_price: Some(gas_price),
                    value: Some(eth_value),
                    data: Some(data.into()),
                    nonce: None,
                    ..TransactionRequest::default()
                };
                self.send_withdraw_tx(&req, tx_to_send).await?
            },
        };

        self.on_finishing()?;
        let tx_hash_bytes = BytesJson::from(tx_hash.0.to_vec());
        let tx_hash_str = format!("{:02x}", tx_hash_bytes);

        let amount_decimal = u256_to_big_decimal(wei_amount, coin.decimals)?;
        let mut spent_by_me = amount_decimal.clone();
        let received_by_me = if to_addr == my_address {
            amount_decimal.clone()
        } else {
            0.into()
        };
        let fee_details = EthTxFeeDetails::new(gas, gas_price, fee_coin)?;
        if coin.coin_type == EthCoinType::Eth {
            spent_by_me += &fee_details.total_fee;
        }
        Ok(TransactionDetails {
            to: vec![checksum_address(&format!("{:#02x}", to_addr))],
            from: vec![checksum_address(&format!("{:#02x}", my_address))],
            total_amount: amount_decimal,
            my_balance_change: &received_by_me - &spent_by_me,
            spent_by_me,
            received_by_me,
            tx_hex,
            tx_hash: tx_hash_str,
            block_height: 0,
            fee_details: Some(fee_details.into()),
            coin: coin.ticker.clone(),
            internal_id: vec![].into(),
            timestamp: now_sec(),
            kmd_rewards: None,
            transaction_type: Default::default(),
            memo: None,
        })
    }
}

/// Eth withdraw version with user interaction support
pub struct InitEthWithdraw {
    ctx: MmArc,
    coin: EthCoin,
    task_handle: WithdrawTaskHandleShared,
    req: WithdrawRequest,
}

#[async_trait]
impl EthWithdraw for InitEthWithdraw {
    fn coin(&self) -> &EthCoin { &self.coin }

    fn request(&self) -> &WithdrawRequest { &self.req }

    fn on_generating_transaction(&self) -> Result<(), MmError<WithdrawError>> {
        Ok(self
            .task_handle
            .update_in_progress_status(WithdrawInProgressStatus::GeneratingTransaction)?)
    }

    fn on_finishing(&self) -> Result<(), MmError<WithdrawError>> {
        Ok(self
            .task_handle
            .update_in_progress_status(WithdrawInProgressStatus::Finishing)?)
    }

    async fn sign_tx_with_trezor(
        &self,
        derivation_path: &DerivationPath,
        unsigned_tx: &UnSignedEthTx,
    ) -> Result<SignedEthTx, MmError<WithdrawError>> {
        let coin = self.coin();
        let crypto_ctx = CryptoCtx::from_ctx(&self.ctx)?;
        let hw_ctx = crypto_ctx
            .hw_ctx()
            .or_mm_err(|| WithdrawError::HwError(HwRpcError::NoTrezorDeviceAvailable))?;
        let trezor_statuses = TrezorRequestStatuses {
            on_button_request: WithdrawInProgressStatus::FollowHwDeviceInstructions,
            on_pin_request: HwRpcTaskAwaitingStatus::EnterTrezorPin,
            on_passphrase_request: HwRpcTaskAwaitingStatus::EnterTrezorPassphrase,
            on_ready: WithdrawInProgressStatus::FollowHwDeviceInstructions,
        };
        let sign_processor = TrezorRpcTaskProcessor::new(self.task_handle.clone(), trezor_statuses);
        let sign_processor = Arc::new(sign_processor);
        let mut trezor_session = hw_ctx.trezor(sign_processor).await?;
        let chain_id = coin
            .chain_id
            .or_mm_err(|| WithdrawError::ChainIdRequired(String::from("chain_id is required for Trezor wallet")))?;
        let unverified_tx = trezor_session
            .sign_eth_tx(derivation_path, unsigned_tx, chain_id)
            .await?;
        Ok(SignedEthTx::new(unverified_tx).map_to_mm(|err| WithdrawError::InternalError(err.to_string()))?)
    }
}

#[allow(clippy::result_large_err)]
impl InitEthWithdraw {
    pub fn new(
        ctx: MmArc,
        coin: EthCoin,
        req: WithdrawRequest,
        task_handle: WithdrawTaskHandleShared,
    ) -> Result<InitEthWithdraw, MmError<WithdrawError>> {
        Ok(InitEthWithdraw {
            ctx,
            coin,
            task_handle,
            req,
        })
    }
}

/// Simple eth withdraw version without user interaction support
pub struct StandardEthWithdraw {
    coin: EthCoin,
    req: WithdrawRequest,
}

#[async_trait]
impl EthWithdraw for StandardEthWithdraw {
    fn coin(&self) -> &EthCoin { &self.coin }

    fn request(&self) -> &WithdrawRequest { &self.req }

    fn on_generating_transaction(&self) -> Result<(), MmError<WithdrawError>> { Ok(()) }

    fn on_finishing(&self) -> Result<(), MmError<WithdrawError>> { Ok(()) }

    async fn sign_tx_with_trezor(
        &self,
        _derivation_path: &DerivationPath,
        _unsigned_tx: &UnSignedEthTx,
    ) -> Result<SignedEthTx, MmError<WithdrawError>> {
        async {
            Err(MmError::new(WithdrawError::UnsupportedError(String::from(
                "Trezor not supported for legacy RPC",
            ))))
        }
        .await
    }
}

#[allow(clippy::result_large_err)]
impl StandardEthWithdraw {
    pub fn new(coin: EthCoin, req: WithdrawRequest) -> Result<StandardEthWithdraw, MmError<WithdrawError>> {
        Ok(StandardEthWithdraw { coin, req })
    }
}

#[async_trait]
impl GetWithdrawSenderAddress for EthCoin {
    type Address = Address;
    type Pubkey = Public;

    async fn get_withdraw_sender_address(
        &self,
        req: &WithdrawRequest,
    ) -> MmResult<WithdrawSenderAddress<Self::Address, Self::Pubkey>, WithdrawError> {
        eth_get_withdraw_from_address(self, req).await
    }
}

async fn eth_get_withdraw_from_address(
    coin: &EthCoin,
    req: &WithdrawRequest,
) -> MmResult<WithdrawSenderAddress<Address, Public>, WithdrawError> {
    match coin.derivation_method() {
        EthDerivationMethod::SingleAddress(my_address) => eth_get_withdraw_iguana_sender(coin, req, my_address),
        EthDerivationMethod::HDWallet(hd_wallet) => {
            let from = req.from.clone().or_mm_err(|| WithdrawError::FromAddressNotFound)?;
            coin.get_withdraw_hd_sender(hd_wallet, &from)
                .await
                .mm_err(WithdrawError::from)
        },
    }
}

#[allow(clippy::result_large_err)]
fn eth_get_withdraw_iguana_sender(
    coin: &EthCoin,
    req: &WithdrawRequest,
    my_address: &Address,
) -> MmResult<WithdrawSenderAddress<Address, Public>, WithdrawError> {
    if req.from.is_some() {
        let error = "'from' is not supported if the coin is initialized with an Iguana private key";
        return MmError::err(WithdrawError::UnexpectedFromAddress(error.to_owned()));
    }

    let pubkey = match coin.priv_key_policy {
        PrivKeyPolicy::Iguana(ref key_pair) => key_pair.public(),
        _ => return MmError::err(WithdrawError::InternalError("not iguana private key policy".to_owned())),
    };

    Ok(WithdrawSenderAddress {
        address: *my_address,
        pubkey: *pubkey,
        derivation_path: None,
    })
}
