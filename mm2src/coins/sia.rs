use super::{CoinBalance, HistorySyncState, MarketCoinOps, MmCoin, RawTransactionFut, RawTransactionRequest, SwapOps,
            TradeFee, TransactionEnum, TransactionFut};
use crate::{coin_errors::MyAddressError, BalanceFut, CanRefundHtlc, CheckIfMyPaymentSentArgs, CoinFutSpawner,
            ConfirmPaymentInput, DexFee, FeeApproxStage, FoundSwapTxSpend, MakerSwapTakerCoin, MmCoinEnum,
            NegotiateSwapContractAddrErr, PaymentInstructionArgs, PaymentInstructions, PaymentInstructionsErr,
            PrivKeyBuildPolicy, PrivKeyPolicy, RawTransactionResult, RefundPaymentArgs, RefundResult,
            SearchForSwapTxSpendInput, SendMakerPaymentSpendPreimageInput, SendPaymentArgs, SignRawTransactionRequest,
            SignatureResult, SpendPaymentArgs, TakerSwapMakerCoin, TradePreimageFut, TradePreimageResult,
            TradePreimageValue, TransactionResult, TxMarshalingErr, UnexpectedDerivationMethod, ValidateAddressResult,
            ValidateFeeArgs, ValidateInstructionsErr, ValidateOtherPubKeyErr, ValidatePaymentError,
            ValidatePaymentFut, ValidatePaymentInput, ValidatePaymentResult, ValidateWatcherSpendInput,
            VerificationResult, WaitForHTLCTxSpendArgs, WatcherOps, WatcherReward, WatcherRewardError,
            WatcherSearchForSwapTxSpendInput, WatcherValidatePaymentInput, WatcherValidateTakerFeeInput, WithdrawFut,
            WithdrawRequest};
use async_trait::async_trait;
use common::executor::AbortedError;
use futures::{FutureExt, TryFutureExt};
use futures01::Future;
use keys::KeyPair;
use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::prelude::*;
use mm2_number::{BigDecimal, MmNumber};
use rpc::v1::types::Bytes as BytesJson;
use serde_json::Value as Json;
use std::ops::Deref;
use std::sync::Arc;
use url::Url;

pub mod address;
use address::v1_standard_address_from_pubkey;
pub mod blake2b_internal;
pub mod encoding;
pub mod http_client;
use http_client::{SiaApiClient, SiaApiClientError};
pub mod spend_policy;

#[derive(Clone)]
pub struct SiaCoin(SiaArc);
#[derive(Clone)]
pub struct SiaArc(Arc<SiaCoinFields>);

#[derive(Debug, Display)]
pub enum SiaConfError {
    #[display(fmt = "'foo' field is not found in config")]
    Foo,
    Bar(String),
}

pub type SiaConfResult<T> = Result<T, MmError<SiaConfError>>;

#[derive(Debug)]
pub struct SiaCoinConf {
    ticker: String,
    pub foo: u32,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SiaHttpConf {
    pub url: Url,
    pub auth: String,
}

// TODO see https://github.com/KomodoPlatform/komodo-defi-framework/pull/2086#discussion_r1521660384
// for additional fields needed
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SiaCoinActivationParams {
    #[serde(default)]
    pub tx_history: bool,
    pub required_confirmations: Option<u64>,
    pub gap_limit: Option<u32>,
    pub http_conf: SiaHttpConf,
}

pub struct SiaConfBuilder<'a> {
    #[allow(dead_code)]
    conf: &'a Json,
    ticker: &'a str,
}

impl<'a> SiaConfBuilder<'a> {
    pub fn new(conf: &'a Json, ticker: &'a str) -> Self { SiaConfBuilder { conf, ticker } }

    pub fn build(&self) -> SiaConfResult<SiaCoinConf> {
        Ok(SiaCoinConf {
            ticker: self.ticker.to_owned(),
            foo: 0,
        })
    }
}

// TODO see https://github.com/KomodoPlatform/komodo-defi-framework/pull/2086#discussion_r1521668313
// for additional fields needed
pub struct SiaCoinFields {
    /// SIA coin config
    pub conf: SiaCoinConf,
    pub priv_key_policy: PrivKeyPolicy<ed25519_dalek::Keypair>,
    /// HTTP(s) client
    pub http_client: SiaApiClient,
}

pub async fn sia_coin_from_conf_and_params(
    ctx: &MmArc,
    ticker: &str,
    conf: &Json,
    params: &SiaCoinActivationParams,
    priv_key_policy: PrivKeyBuildPolicy,
) -> Result<SiaCoin, MmError<SiaCoinBuildError>> {
    let priv_key = match priv_key_policy {
        PrivKeyBuildPolicy::IguanaPrivKey(priv_key) => priv_key,
        _ => return Err(SiaCoinBuildError::UnsupportedPrivKeyPolicy.into()),
    };
    let key_pair = generate_keypair_from_slice(priv_key.as_slice())?;
    let builder = SiaCoinBuilder::new(ctx, ticker, conf, key_pair, params);
    builder.build().await
}

pub struct SiaCoinBuilder<'a> {
    ctx: &'a MmArc,
    ticker: &'a str,
    conf: &'a Json,
    key_pair: ed25519_dalek::Keypair,
    params: &'a SiaCoinActivationParams,
}

impl<'a> SiaCoinBuilder<'a> {
    pub fn new(
        ctx: &'a MmArc,
        ticker: &'a str,
        conf: &'a Json,
        key_pair: ed25519_dalek::Keypair,
        params: &'a SiaCoinActivationParams,
    ) -> Self {
        SiaCoinBuilder {
            ctx,
            ticker,
            conf,
            key_pair,
            params,
        }
    }
}

fn generate_keypair_from_slice(priv_key: &[u8]) -> Result<ed25519_dalek::Keypair, SiaCoinBuildError> {
    let secret_key = ed25519_dalek::SecretKey::from_bytes(priv_key).map_err(SiaCoinBuildError::EllipticCurveError)?;
    let public_key = ed25519_dalek::PublicKey::from(&secret_key);
    Ok(ed25519_dalek::Keypair {
        secret: secret_key,
        public: public_key,
    })
}

impl From<SiaConfError> for SiaCoinBuildError {
    fn from(e: SiaConfError) -> Self { SiaCoinBuildError::ConfError(e) }
}

#[derive(Debug, Display)]
pub enum SiaCoinBuildError {
    ConfError(SiaConfError),
    UnsupportedPrivKeyPolicy,
    ClientError(SiaApiClientError),
    EllipticCurveError(ed25519_dalek::ed25519::Error),
}

impl<'a> SiaCoinBuilder<'a> {
    #[allow(dead_code)]
    fn ctx(&self) -> &MmArc { self.ctx }

    #[allow(dead_code)]
    fn conf(&self) -> &Json { self.conf }

    fn ticker(&self) -> &str { self.ticker }

    async fn build(self) -> MmResult<SiaCoin, SiaCoinBuildError> {
        let conf = SiaConfBuilder::new(self.conf, self.ticker()).build()?;
        let sia_fields = SiaCoinFields {
            conf,
            http_client: SiaApiClient::new(self.ticker(), self.params.http_conf.clone())
                .map_err(SiaCoinBuildError::ClientError)?,
            priv_key_policy: PrivKeyPolicy::Iguana(self.key_pair),
        };
        let sia_arc = SiaArc::new(sia_fields);

        Ok(SiaCoin::from(sia_arc))
    }
}

impl Deref for SiaArc {
    type Target = SiaCoinFields;
    fn deref(&self) -> &SiaCoinFields { &self.0 }
}

impl From<SiaCoinFields> for SiaArc {
    fn from(coin: SiaCoinFields) -> SiaArc { SiaArc::new(coin) }
}

impl From<Arc<SiaCoinFields>> for SiaArc {
    fn from(arc: Arc<SiaCoinFields>) -> SiaArc { SiaArc(arc) }
}

impl From<SiaArc> for SiaCoin {
    fn from(coin: SiaArc) -> SiaCoin { SiaCoin(coin) }
}

impl SiaArc {
    pub fn new(fields: SiaCoinFields) -> SiaArc { SiaArc(Arc::new(fields)) }

    pub fn with_arc(inner: Arc<SiaCoinFields>) -> SiaArc { SiaArc(inner) }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SiaCoinProtocolInfo;

#[async_trait]
impl MmCoin for SiaCoin {
    fn is_asset_chain(&self) -> bool { false }

    fn spawner(&self) -> CoinFutSpawner { unimplemented!() }

    fn get_raw_transaction(&self, _req: RawTransactionRequest) -> RawTransactionFut { unimplemented!() }

    fn get_tx_hex_by_hash(&self, _tx_hash: Vec<u8>) -> RawTransactionFut { unimplemented!() }

    fn withdraw(&self, _req: WithdrawRequest) -> WithdrawFut { unimplemented!() }

    fn decimals(&self) -> u8 { unimplemented!() }

    fn convert_to_address(&self, _from: &str, _to_address_format: Json) -> Result<String, String> { unimplemented!() }

    fn validate_address(&self, _address: &str) -> ValidateAddressResult { unimplemented!() }

    fn process_history_loop(&self, _ctx: MmArc) -> Box<dyn Future<Item = (), Error = ()> + Send> { unimplemented!() }

    fn history_sync_status(&self) -> HistorySyncState { unimplemented!() }

    /// Get fee to be paid per 1 swap transaction
    fn get_trade_fee(&self) -> Box<dyn Future<Item = TradeFee, Error = String> + Send> { unimplemented!() }

    async fn get_sender_trade_fee(
        &self,
        _value: TradePreimageValue,
        _stage: FeeApproxStage,
        _include_refund_fee: bool,
    ) -> TradePreimageResult<TradeFee> {
        unimplemented!()
    }

    fn get_receiver_trade_fee(&self, _stage: FeeApproxStage) -> TradePreimageFut<TradeFee> { unimplemented!() }

    async fn get_fee_to_send_taker_fee(
        &self,
        _dex_fee_amount: DexFee,
        _stage: FeeApproxStage,
    ) -> TradePreimageResult<TradeFee> {
        unimplemented!()
    }

    fn required_confirmations(&self) -> u64 { unimplemented!() }

    fn requires_notarization(&self) -> bool { false }

    fn set_required_confirmations(&self, _confirmations: u64) { unimplemented!() }

    fn set_requires_notarization(&self, _requires_nota: bool) { unimplemented!() }

    fn swap_contract_address(&self) -> Option<BytesJson> { unimplemented!() }

    fn fallback_swap_contract(&self) -> Option<BytesJson> { unimplemented!() }

    fn mature_confirmations(&self) -> Option<u32> { unimplemented!() }

    fn coin_protocol_info(&self, _amount_to_receive: Option<MmNumber>) -> Vec<u8> { Vec::new() }

    fn is_coin_protocol_supported(
        &self,
        _info: &Option<Vec<u8>>,
        _amount_to_send: Option<MmNumber>,
        _locktime: u64,
        _is_maker: bool,
    ) -> bool {
        true
    }

    fn on_disabled(&self) -> Result<(), AbortedError> { Ok(()) }

    fn on_token_deactivated(&self, _ticker: &str) {}
}

// TODO Alright - Dummy values for these functions allow minimal functionality to produce signatures
#[async_trait]
impl MarketCoinOps for SiaCoin {
    fn ticker(&self) -> &str { &self.0.conf.ticker }

    // needs test coverage FIXME COME BACK
    fn my_address(&self) -> MmResult<String, MyAddressError> {
        let key_pair = match &self.0.priv_key_policy {
            PrivKeyPolicy::Iguana(key_pair) => key_pair,
            PrivKeyPolicy::Trezor => {
                return Err(MyAddressError::UnexpectedDerivationMethod(
                    "Trezor not yet supported. Must use iguana seed.".to_string(),
                )
                .into());
            },
            PrivKeyPolicy::HDWallet { .. } => {
                return Err(MyAddressError::UnexpectedDerivationMethod(
                    "HDWallet not yet supported. Must use iguana seed.".to_string(),
                )
                .into());
            },
        };

        let address = v1_standard_address_from_pubkey(&key_pair.public);
        Ok(address.to_string())
    }

    async fn get_public_key(&self) -> Result<String, MmError<UnexpectedDerivationMethod>> { unimplemented!() }

    fn sign_message_hash(&self, _message: &str) -> Option<[u8; 32]> { unimplemented!() }

    fn sign_message(&self, _message: &str) -> SignatureResult<String> { unimplemented!() }

    fn verify_message(&self, _signature: &str, _message: &str, _address: &str) -> VerificationResult<bool> {
        unimplemented!()
    }

    fn my_balance(&self) -> BalanceFut<CoinBalance> {
        let fut = async move {
            Ok(CoinBalance {
                spendable: BigDecimal::default(),
                unspendable: BigDecimal::default(),
            })
        };
        Box::new(fut.boxed().compat())
    }
    fn base_coin_balance(&self) -> BalanceFut<BigDecimal> { unimplemented!() }

    fn platform_ticker(&self) -> &str { "FOO" } // TODO Alright

    /// Receives raw transaction bytes in hexadecimal format as input and returns tx hash in hexadecimal format
    fn send_raw_tx(&self, _tx: &str) -> Box<dyn Future<Item = String, Error = String> + Send> { unimplemented!() }

    fn send_raw_tx_bytes(&self, _tx: &[u8]) -> Box<dyn Future<Item = String, Error = String> + Send> {
        unimplemented!()
    }

    #[inline(always)]
    async fn sign_raw_tx(&self, _args: &SignRawTransactionRequest) -> RawTransactionResult { unimplemented!() }

    fn wait_for_confirmations(&self, _input: ConfirmPaymentInput) -> Box<dyn Future<Item = (), Error = String> + Send> {
        unimplemented!()
    }

    fn wait_for_htlc_tx_spend(&self, _args: WaitForHTLCTxSpendArgs<'_>) -> TransactionFut { unimplemented!() }

    fn tx_enum_from_bytes(&self, _bytes: &[u8]) -> Result<TransactionEnum, MmError<TxMarshalingErr>> {
        MmError::err(TxMarshalingErr::NotSupported(
            "tx_enum_from_bytes is not supported for Sia coin yet.".to_string(),
        ))
    }

    fn current_block(&self) -> Box<dyn Future<Item = u64, Error = String> + Send> {
        let http_client = self.0.http_client.clone(); // Clone the client

        let height_fut = async move { http_client.get_height().await.map_err(|e| e.to_string()) }
            .boxed() // Make the future 'static by boxing
            .compat(); // Convert to a futures 0.1-compatible future

        Box::new(height_fut)
    }

    fn display_priv_key(&self) -> Result<String, String> { unimplemented!() }

    fn min_tx_amount(&self) -> BigDecimal { unimplemented!() }

    fn min_trading_vol(&self) -> MmNumber { unimplemented!() }

    fn is_trezor(&self) -> bool { self.0.priv_key_policy.is_trezor() }
}

#[async_trait]
impl SwapOps for SiaCoin {
    fn send_taker_fee(&self, _fee_addr: &[u8], _dex_fee: DexFee, _uuid: &[u8], _expire_at: u64) -> TransactionFut {
        unimplemented!()
    }

    fn send_maker_payment(&self, _maker_payment_args: SendPaymentArgs) -> TransactionFut { unimplemented!() }

    fn send_taker_payment(&self, _taker_payment_args: SendPaymentArgs) -> TransactionFut { unimplemented!() }

    async fn send_maker_spends_taker_payment(
        &self,
        _maker_spends_payment_args: SpendPaymentArgs<'_>,
    ) -> TransactionResult {
        unimplemented!()
    }

    async fn send_taker_spends_maker_payment(
        &self,
        _taker_spends_payment_args: SpendPaymentArgs<'_>,
    ) -> TransactionResult {
        unimplemented!()
    }

    async fn send_taker_refunds_payment(
        &self,
        _taker_refunds_payment_args: RefundPaymentArgs<'_>,
    ) -> TransactionResult {
        unimplemented!()
    }

    async fn send_maker_refunds_payment(
        &self,
        _maker_refunds_payment_args: RefundPaymentArgs<'_>,
    ) -> TransactionResult {
        unimplemented!()
    }

    fn validate_fee(&self, _validate_fee_args: ValidateFeeArgs) -> ValidatePaymentFut<()> { unimplemented!() }

    async fn validate_maker_payment(&self, _input: ValidatePaymentInput) -> ValidatePaymentResult<()> {
        unimplemented!()
    }

    async fn validate_taker_payment(&self, _input: ValidatePaymentInput) -> ValidatePaymentResult<()> {
        unimplemented!()
    }

    fn check_if_my_payment_sent(
        &self,
        _if_my_payment_sent_args: CheckIfMyPaymentSentArgs,
    ) -> Box<dyn Future<Item = Option<TransactionEnum>, Error = String> + Send> {
        unimplemented!()
    }

    async fn search_for_swap_tx_spend_my(
        &self,
        _: SearchForSwapTxSpendInput<'_>,
    ) -> Result<Option<FoundSwapTxSpend>, String> {
        unimplemented!()
    }

    async fn search_for_swap_tx_spend_other(
        &self,
        _: SearchForSwapTxSpendInput<'_>,
    ) -> Result<Option<FoundSwapTxSpend>, String> {
        unimplemented!()
    }

    fn check_tx_signed_by_pub(&self, _tx: &[u8], _expected_pub: &[u8]) -> Result<bool, MmError<ValidatePaymentError>> {
        unimplemented!();
    }

    async fn extract_secret(
        &self,
        _secret_hash: &[u8],
        _spend_tx: &[u8],
        _watcher_reward: bool,
    ) -> Result<Vec<u8>, String> {
        unimplemented!()
    }

    fn is_auto_refundable(&self) -> bool { false }

    async fn wait_for_htlc_refund(&self, _tx: &[u8], _locktime: u64) -> RefundResult<()> { unimplemented!() }

    fn negotiate_swap_contract_addr(
        &self,
        _other_side_address: Option<&[u8]>,
    ) -> Result<Option<BytesJson>, MmError<NegotiateSwapContractAddrErr>> {
        unimplemented!()
    }

    fn derive_htlc_key_pair(&self, _swap_unique_data: &[u8]) -> KeyPair { unimplemented!() }

    fn derive_htlc_pubkey(&self, _swap_unique_data: &[u8]) -> Vec<u8> { unimplemented!() }

    fn can_refund_htlc(&self, _locktime: u64) -> Box<dyn Future<Item = CanRefundHtlc, Error = String> + Send + '_> {
        unimplemented!()
    }

    fn validate_other_pubkey(&self, _raw_pubkey: &[u8]) -> MmResult<(), ValidateOtherPubKeyErr> { unimplemented!() }

    async fn maker_payment_instructions(
        &self,
        _args: PaymentInstructionArgs<'_>,
    ) -> Result<Option<Vec<u8>>, MmError<PaymentInstructionsErr>> {
        unimplemented!()
    }

    async fn taker_payment_instructions(
        &self,
        _args: PaymentInstructionArgs<'_>,
    ) -> Result<Option<Vec<u8>>, MmError<PaymentInstructionsErr>> {
        unimplemented!()
    }

    fn validate_maker_payment_instructions(
        &self,
        _instructions: &[u8],
        _args: PaymentInstructionArgs,
    ) -> Result<PaymentInstructions, MmError<ValidateInstructionsErr>> {
        unimplemented!()
    }

    fn validate_taker_payment_instructions(
        &self,
        _instructions: &[u8],
        _args: PaymentInstructionArgs,
    ) -> Result<PaymentInstructions, MmError<ValidateInstructionsErr>> {
        unimplemented!()
    }
}

#[async_trait]
impl TakerSwapMakerCoin for SiaCoin {
    async fn on_taker_payment_refund_start(&self, _maker_payment: &[u8]) -> RefundResult<()> { Ok(()) }

    async fn on_taker_payment_refund_success(&self, _maker_payment: &[u8]) -> RefundResult<()> { Ok(()) }
}

#[async_trait]
impl MakerSwapTakerCoin for SiaCoin {
    async fn on_maker_payment_refund_start(&self, _taker_payment: &[u8]) -> RefundResult<()> { Ok(()) }

    async fn on_maker_payment_refund_success(&self, _taker_payment: &[u8]) -> RefundResult<()> { Ok(()) }
}

#[async_trait]
impl WatcherOps for SiaCoin {
    fn send_maker_payment_spend_preimage(&self, _input: SendMakerPaymentSpendPreimageInput) -> TransactionFut {
        unimplemented!();
    }

    fn send_taker_payment_refund_preimage(&self, _watcher_refunds_payment_args: RefundPaymentArgs) -> TransactionFut {
        unimplemented!();
    }

    fn create_taker_payment_refund_preimage(
        &self,
        _taker_payment_tx: &[u8],
        _time_lock: u64,
        _maker_pub: &[u8],
        _secret_hash: &[u8],
        _swap_contract_address: &Option<BytesJson>,
        _swap_unique_data: &[u8],
    ) -> TransactionFut {
        unimplemented!();
    }

    fn create_maker_payment_spend_preimage(
        &self,
        _maker_payment_tx: &[u8],
        _time_lock: u64,
        _maker_pub: &[u8],
        _secret_hash: &[u8],
        _swap_unique_data: &[u8],
    ) -> TransactionFut {
        unimplemented!();
    }

    fn watcher_validate_taker_fee(&self, _input: WatcherValidateTakerFeeInput) -> ValidatePaymentFut<()> {
        unimplemented!();
    }

    fn watcher_validate_taker_payment(&self, _input: WatcherValidatePaymentInput) -> ValidatePaymentFut<()> {
        unimplemented!();
    }

    fn taker_validates_payment_spend_or_refund(&self, _input: ValidateWatcherSpendInput) -> ValidatePaymentFut<()> {
        unimplemented!()
    }

    async fn watcher_search_for_swap_tx_spend(
        &self,
        _input: WatcherSearchForSwapTxSpendInput<'_>,
    ) -> Result<Option<FoundSwapTxSpend>, String> {
        unimplemented!();
    }

    async fn get_taker_watcher_reward(
        &self,
        _other_coin: &MmCoinEnum,
        _coin_amount: Option<BigDecimal>,
        _other_coin_amount: Option<BigDecimal>,
        _reward_amount: Option<BigDecimal>,
        _wait_until: u64,
    ) -> Result<WatcherReward, MmError<WatcherRewardError>> {
        unimplemented!()
    }

    async fn get_maker_watcher_reward(
        &self,
        _other_coin: &MmCoinEnum,
        _reward_amount: Option<BigDecimal>,
        _wait_until: u64,
    ) -> Result<Option<WatcherReward>, MmError<WatcherRewardError>> {
        unimplemented!()
    }
}
