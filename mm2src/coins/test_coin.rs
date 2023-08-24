#![allow(clippy::all)]

use super::{CoinBalance, HistorySyncState, MarketCoinOps, MmCoin, RawTransactionFut, RawTransactionRequest, SwapOps,
            TradeFee, TransactionEnum, TransactionFut};
use crate::{coin_errors::MyAddressError, BalanceFut, CanRefundHtlc, CheckIfMyPaymentSentArgs, CoinFutSpawner,
            ConfirmPaymentInput, FeeApproxStage, FoundSwapTxSpend, MakerSwapTakerCoin, MmCoinEnum,
            NegotiateSwapContractAddrErr, PaymentInstructionArgs, PaymentInstructions, PaymentInstructionsErr,
            RefundPaymentArgs, RefundResult, SearchForSwapTxSpendInput, SendMakerPaymentSpendPreimageInput,
            SendPaymentArgs, SignRawTransactionRequest, SignRawTransactionResult, SignatureResult, SpendPaymentArgs,
            TakerSwapMakerCoin, TradePreimageFut, TradePreimageResult, TradePreimageValue, TxMarshalingErr,
            UnexpectedDerivationMethod, ValidateAddressResult, ValidateFeeArgs, ValidateInstructionsErr,
            ValidateOtherPubKeyErr, ValidatePaymentError, ValidatePaymentFut, ValidatePaymentInput,
            VerificationResult, WaitForHTLCTxSpendArgs, WatcherOps, WatcherReward, WatcherRewardError,
            WatcherSearchForSwapTxSpendInput, WatcherValidatePaymentInput, WatcherValidateTakerFeeInput, WithdrawFut,
            WithdrawRequest, SignEthTransactionRequest, SignEthTransactionResult};
use async_trait::async_trait;
use common::executor::AbortedError;
use futures01::Future;
use keys::KeyPair;
use mm2_core::mm_ctx::MmArc;
use mm2_err_handle::prelude::*;
use mm2_number::{BigDecimal, MmNumber};
use mocktopus::macros::*;
use rpc::v1::types::Bytes as BytesJson;
use serde_json::Value as Json;
use std::ops::Deref;
use std::sync::Arc;

/// Dummy coin struct used in tests which functions are unimplemented but then mocked
/// in specific test to emulate the required behaviour
#[derive(Clone, Debug)]
pub struct TestCoin(Arc<TestCoinImpl>);

impl Deref for TestCoin {
    type Target = TestCoinImpl;

    fn deref(&self) -> &Self::Target { &self.0 }
}

#[derive(Debug)]
pub struct TestCoinImpl {
    ticker: String,
}

impl Default for TestCoin {
    fn default() -> Self { TestCoin(Arc::new(TestCoinImpl { ticker: "test".into() })) }
}

impl TestCoin {
    pub fn new(ticker: &str) -> TestCoin { TestCoin(Arc::new(TestCoinImpl { ticker: ticker.into() })) }
}

#[async_trait]
#[mockable]
impl MarketCoinOps for TestCoin {
    fn ticker(&self) -> &str { &self.ticker }

    fn my_address(&self) -> MmResult<String, MyAddressError> { unimplemented!() }

    fn get_public_key(&self) -> Result<String, MmError<UnexpectedDerivationMethod>> { unimplemented!() }

    fn sign_message_hash(&self, _message: &str) -> Option<[u8; 32]> { unimplemented!() }

    fn sign_message(&self, _message: &str) -> SignatureResult<String> { unimplemented!() }

    fn verify_message(&self, _signature: &str, _message: &str, _address: &str) -> VerificationResult<bool> {
        unimplemented!()
    }

    fn my_balance(&self) -> BalanceFut<CoinBalance> { unimplemented!() }

    fn base_coin_balance(&self) -> BalanceFut<BigDecimal> { unimplemented!() }

    fn platform_ticker(&self) -> &str { &self.ticker }

    /// Receives raw transaction bytes in hexadecimal format as input and returns tx hash in hexadecimal format
    fn send_raw_tx(&self, tx: &str) -> Box<dyn Future<Item = String, Error = String> + Send> { unimplemented!() }

    fn send_raw_tx_bytes(&self, tx: &[u8]) -> Box<dyn Future<Item = String, Error = String> + Send> { unimplemented!() }

    #[inline(always)]
    async fn sign_raw_tx(&self, _args: &SignRawTransactionRequest) -> SignRawTransactionResult { unimplemented!() }
    
    #[inline(always)]
    async fn sign_eth_tx(&self, _args: &SignEthTransactionRequest) -> SignEthTransactionResult { unimplemented!() }

    fn wait_for_confirmations(&self, _input: ConfirmPaymentInput) -> Box<dyn Future<Item = (), Error = String> + Send> {
        unimplemented!()
    }

    fn wait_for_htlc_tx_spend(&self, args: WaitForHTLCTxSpendArgs<'_>) -> TransactionFut { unimplemented!() }

    fn tx_enum_from_bytes(&self, _bytes: &[u8]) -> Result<TransactionEnum, MmError<TxMarshalingErr>> {
        MmError::err(TxMarshalingErr::NotSupported(
            "tx_enum_from_bytes is not supported for Test coin yet.".to_string(),
        ))
    }

    fn current_block(&self) -> Box<dyn Future<Item = u64, Error = String> + Send> { unimplemented!() }

    fn display_priv_key(&self) -> Result<String, String> { unimplemented!() }

    fn min_tx_amount(&self) -> BigDecimal { unimplemented!() }

    fn min_trading_vol(&self) -> MmNumber { MmNumber::from("0.00777") }
}

#[async_trait]
#[mockable]
impl SwapOps for TestCoin {
    fn send_taker_fee(&self, fee_addr: &[u8], amount: BigDecimal, uuid: &[u8]) -> TransactionFut { unimplemented!() }

    fn send_maker_payment(&self, _maker_payment_args: SendPaymentArgs) -> TransactionFut { unimplemented!() }

    fn send_taker_payment(&self, _taker_payment_args: SendPaymentArgs) -> TransactionFut { unimplemented!() }

    fn send_maker_spends_taker_payment(&self, _maker_spends_payment_args: SpendPaymentArgs) -> TransactionFut {
        unimplemented!()
    }

    fn send_taker_spends_maker_payment(&self, _taker_spends_payment_args: SpendPaymentArgs) -> TransactionFut {
        unimplemented!()
    }

    fn send_taker_refunds_payment(&self, _taker_refunds_payment_args: RefundPaymentArgs) -> TransactionFut {
        unimplemented!()
    }

    fn send_maker_refunds_payment(&self, _maker_refunds_payment_args: RefundPaymentArgs) -> TransactionFut {
        unimplemented!()
    }

    fn validate_fee(&self, _validate_fee_args: ValidateFeeArgs) -> ValidatePaymentFut<()> { unimplemented!() }

    fn validate_maker_payment(&self, _input: ValidatePaymentInput) -> ValidatePaymentFut<()> { unimplemented!() }

    fn validate_taker_payment(&self, _input: ValidatePaymentInput) -> ValidatePaymentFut<()> { unimplemented!() }

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

    fn check_tx_signed_by_pub(&self, tx: &[u8], expected_pub: &[u8]) -> Result<bool, MmError<ValidatePaymentError>> {
        unimplemented!();
    }

    async fn extract_secret(
        &self,
        secret_hash: &[u8],
        spend_tx: &[u8],
        watcher_reward: bool,
    ) -> Result<Vec<u8>, String> {
        unimplemented!()
    }

    fn is_auto_refundable(&self) -> bool { false }

    async fn wait_for_htlc_refund(&self, _tx: &[u8], _locktime: u64) -> RefundResult<()> { unimplemented!() }

    fn negotiate_swap_contract_addr(
        &self,
        other_side_address: Option<&[u8]>,
    ) -> Result<Option<BytesJson>, MmError<NegotiateSwapContractAddrErr>> {
        unimplemented!()
    }

    fn derive_htlc_key_pair(&self, _swap_unique_data: &[u8]) -> KeyPair { unimplemented!() }

    fn derive_htlc_pubkey(&self, _swap_unique_data: &[u8]) -> Vec<u8> { unimplemented!() }

    fn can_refund_htlc(&self, locktime: u64) -> Box<dyn Future<Item = CanRefundHtlc, Error = String> + Send + '_> {
        unimplemented!()
    }

    fn validate_other_pubkey(&self, raw_pubkey: &[u8]) -> MmResult<(), ValidateOtherPubKeyErr> { unimplemented!() }

    async fn maker_payment_instructions(
        &self,
        _args: PaymentInstructionArgs<'_>,
    ) -> Result<Option<Vec<u8>>, MmError<PaymentInstructionsErr>> {
        unimplemented!()
    }

    async fn taker_payment_instructions(
        &self,
        args: PaymentInstructionArgs<'_>,
    ) -> Result<Option<Vec<u8>>, MmError<PaymentInstructionsErr>> {
        unimplemented!()
    }

    fn validate_maker_payment_instructions(
        &self,
        _instructions: &[u8],
        args: PaymentInstructionArgs,
    ) -> Result<PaymentInstructions, MmError<ValidateInstructionsErr>> {
        unimplemented!()
    }

    fn validate_taker_payment_instructions(
        &self,
        _instructions: &[u8],
        args: PaymentInstructionArgs,
    ) -> Result<PaymentInstructions, MmError<ValidateInstructionsErr>> {
        unimplemented!()
    }
}

#[async_trait]
impl TakerSwapMakerCoin for TestCoin {
    async fn on_taker_payment_refund_start(&self, _maker_payment: &[u8]) -> RefundResult<()> { Ok(()) }

    async fn on_taker_payment_refund_success(&self, _maker_payment: &[u8]) -> RefundResult<()> { Ok(()) }
}

#[async_trait]
impl MakerSwapTakerCoin for TestCoin {
    async fn on_maker_payment_refund_start(&self, _taker_payment: &[u8]) -> RefundResult<()> { Ok(()) }

    async fn on_maker_payment_refund_success(&self, _taker_payment: &[u8]) -> RefundResult<()> { Ok(()) }
}

#[async_trait]
#[mockable]
impl WatcherOps for TestCoin {
    fn create_maker_payment_spend_preimage(
        &self,
        _maker_payment_tx: &[u8],
        _time_lock: u32,
        _maker_pub: &[u8],
        _secret_hash: &[u8],
        _swap_unique_data: &[u8],
    ) -> TransactionFut {
        unimplemented!();
    }

    fn send_maker_payment_spend_preimage(&self, _input: SendMakerPaymentSpendPreimageInput) -> TransactionFut {
        unimplemented!();
    }

    fn create_taker_payment_refund_preimage(
        &self,
        _taker_payment_tx: &[u8],
        _time_lock: u32,
        _maker_pub: &[u8],
        _secret_hash: &[u8],
        _swap_contract_address: &Option<BytesJson>,
        _swap_unique_data: &[u8],
    ) -> TransactionFut {
        unimplemented!();
    }

    fn send_taker_payment_refund_preimage(&self, _watcher_refunds_payment_args: RefundPaymentArgs) -> TransactionFut {
        unimplemented!();
    }

    fn watcher_validate_taker_fee(&self, input: WatcherValidateTakerFeeInput) -> ValidatePaymentFut<()> {
        unimplemented!();
    }

    fn watcher_validate_taker_payment(&self, _input: WatcherValidatePaymentInput) -> ValidatePaymentFut<()> {
        unimplemented!();
    }

    async fn watcher_search_for_swap_tx_spend(
        &self,
        input: WatcherSearchForSwapTxSpendInput<'_>,
    ) -> Result<Option<FoundSwapTxSpend>, String> {
        unimplemented!();
    }

    async fn get_taker_watcher_reward(
        &self,
        other_coin: &MmCoinEnum,
        coin_amount: Option<BigDecimal>,
        other_coin_amount: Option<BigDecimal>,
        reward_amount: Option<BigDecimal>,
        wait_until: u64,
    ) -> Result<WatcherReward, MmError<WatcherRewardError>> {
        unimplemented!()
    }

    async fn get_maker_watcher_reward(
        &self,
        other_coin: &MmCoinEnum,
        reward_amount: Option<BigDecimal>,
        wait_until: u64,
    ) -> Result<Option<WatcherReward>, MmError<WatcherRewardError>> {
        unimplemented!()
    }
}

#[async_trait]
#[mockable]
impl MmCoin for TestCoin {
    fn is_asset_chain(&self) -> bool { unimplemented!() }

    fn spawner(&self) -> CoinFutSpawner { unimplemented!() }

    fn get_raw_transaction(&self, _req: RawTransactionRequest) -> RawTransactionFut { unimplemented!() }

    fn get_tx_hex_by_hash(&self, tx_hash: Vec<u8>) -> RawTransactionFut { unimplemented!() }

    fn withdraw(&self, req: WithdrawRequest) -> WithdrawFut { unimplemented!() }

    fn decimals(&self) -> u8 { unimplemented!() }

    fn convert_to_address(&self, from: &str, to_address_format: Json) -> Result<String, String> { unimplemented!() }

    fn validate_address(&self, address: &str) -> ValidateAddressResult { unimplemented!() }

    fn process_history_loop(&self, ctx: MmArc) -> Box<dyn Future<Item = (), Error = ()> + Send> { unimplemented!() }

    fn history_sync_status(&self) -> HistorySyncState { unimplemented!() }

    /// Get fee to be paid per 1 swap transaction
    fn get_trade_fee(&self) -> Box<dyn Future<Item = TradeFee, Error = String> + Send> { unimplemented!() }

    async fn get_sender_trade_fee(
        &self,
        _value: TradePreimageValue,
        _stage: FeeApproxStage,
    ) -> TradePreimageResult<TradeFee> {
        unimplemented!()
    }

    fn get_receiver_trade_fee(&self, _stage: FeeApproxStage) -> TradePreimageFut<TradeFee> { unimplemented!() }

    async fn get_fee_to_send_taker_fee(
        &self,
        _dex_fee_amount: BigDecimal,
        _stage: FeeApproxStage,
    ) -> TradePreimageResult<TradeFee> {
        unimplemented!()
    }

    fn required_confirmations(&self) -> u64 { 1 }

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

    fn on_token_deactivated(&self, _ticker: &str) { () }
}
