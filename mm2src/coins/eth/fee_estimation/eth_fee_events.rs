use super::eip1559::GasApiConfig;
use crate::eth::EthCoin;
use common::executor::Timer;
use mm2_event_stream::{Event, EventStreamer, NoDataIn, StreamHandlerInput, StreamingManager};

use async_trait::async_trait;
use futures::channel::oneshot;
use instant::Instant;
use serde::Deserialize;
use serde_json::Value as Json;
use std::convert::TryFrom;

#[derive(Deserialize)]
#[serde(deny_unknown_fields, default)]
struct EthFeeStreamingConfig {
    /// The time in seconds to wait before re-estimating the gas fees.
    pub estimate_every: f64,
    /// Optional Gas API provider to be used to fetch gas prices.
    /// If not provided, the "simple" (history based) provider will be used.
    // FIXME: There is a similar field in MM2.json config, that one is the one used for swaps.
    // Should we allow setting the one in the streamer like this? More flexible but will end
    // up confusing users. What about setting `gas_api` with another end point and always use
    // the one set.
    pub gas_api_provider: Option<GasApiConfig>,
}

impl Default for EthFeeStreamingConfig {
    fn default() -> Self {
        Self {
            estimate_every: 15.0,
            gas_api_provider: None,
        }
    }
}

pub struct EthFeeEventStreamer {
    config: EthFeeStreamingConfig,
    coin: EthCoin,
}

impl EthFeeEventStreamer {
    pub fn try_new(config: Option<Json>, coin: EthCoin) -> serde_json::Result<Self> {
        let config = config.map(serde_json::from_value).unwrap_or(Ok(Default::default()))?;

        Ok(Self { config, coin })
    }
}

#[async_trait]
impl EventStreamer for EthFeeEventStreamer {
    type DataInType = NoDataIn;

    fn streamer_id(&self) -> String { format!("FEE_ESTIMATION:{}", self.coin.ticker) }

    async fn handle(
        self,
        broadcaster: StreamingManager,
        ready_tx: oneshot::Sender<Result<(), String>>,
        _: impl StreamHandlerInput<NoDataIn>,
    ) {
        ready_tx
            .send(Ok(()))
            .expect("Receiver is dropped, which should never happen.");

        loop {
            let now = Instant::now();
            match self
                .coin
                .get_eip1559_gas_fee(self.config.gas_api_provider.clone())
                .await
                .map(serialized::FeePerGasEstimated::try_from)
            {
                Ok(Ok(fee)) => {
                    let fee = serde_json::to_value(fee).expect("Serialization shouldn't fail");
                    broadcaster.broadcast(Event::new(self.streamer_id(), fee));
                },
                Ok(Err(err)) => {
                    let err = json!({ "error": err.to_string() });
                    broadcaster.broadcast(Event::err(self.streamer_id(), err));
                },
                Err(err) => {
                    let err = serde_json::to_value(err).expect("Serialization shouldn't fail");
                    broadcaster.broadcast(Event::err(self.streamer_id(), err));
                },
            }
            let sleep_time = self.config.estimate_every - now.elapsed().as_secs_f64();
            if sleep_time >= 0.1 {
                Timer::sleep(sleep_time).await;
            }
        }
    }
}

/// Serializable version of fee estimation data.
mod serialized {
    use crate::eth::fee_estimation::eip1559;
    use crate::{wei_to_gwei_decimal, NumConversError};
    use mm2_err_handle::mm_error::MmError;
    use mm2_number::BigDecimal;

    use std::convert::TryFrom;

    /// Estimated fee per gas units
    #[derive(Serialize)]
    pub enum EstimationUnits {
        Gwei,
    }

    /// Priority level estimated max fee per gas
    #[derive(Serialize)]
    pub struct FeePerGasLevel {
        /// estimated max priority tip fee per gas in gwei
        pub max_priority_fee_per_gas: BigDecimal,
        /// estimated max fee per gas in gwei
        pub max_fee_per_gas: BigDecimal,
        /// estimated transaction min wait time in mempool in ms for this priority level
        pub min_wait_time: Option<u32>,
        /// estimated transaction max wait time in mempool in ms for this priority level
        pub max_wait_time: Option<u32>,
    }

    /// External struct for estimated fee per gas for several priority levels, in gwei
    /// low/medium/high levels are supported
    #[derive(Serialize)]
    pub struct FeePerGasEstimated {
        /// base fee for the next block in gwei
        pub base_fee: BigDecimal,
        /// estimated low priority fee
        pub low: FeePerGasLevel,
        /// estimated medium priority fee
        pub medium: FeePerGasLevel,
        /// estimated high priority fee
        pub high: FeePerGasLevel,
        /// which estimator used
        pub source: String,
        /// base trend (up or down)
        pub base_fee_trend: String,
        /// priority trend (up or down)
        pub priority_fee_trend: String,
        /// fee units
        pub units: EstimationUnits,
    }

    impl TryFrom<eip1559::FeePerGasEstimated> for FeePerGasEstimated {
        type Error = MmError<NumConversError>;

        fn try_from(fees: eip1559::FeePerGasEstimated) -> Result<Self, Self::Error> {
            Ok(Self {
                base_fee: wei_to_gwei_decimal!(fees.base_fee)?,
                low: FeePerGasLevel {
                    max_fee_per_gas: wei_to_gwei_decimal!(fees.low.max_fee_per_gas)?,
                    max_priority_fee_per_gas: wei_to_gwei_decimal!(fees.low.max_priority_fee_per_gas)?,
                    min_wait_time: fees.low.min_wait_time,
                    max_wait_time: fees.low.max_wait_time,
                },
                medium: FeePerGasLevel {
                    max_fee_per_gas: wei_to_gwei_decimal!(fees.medium.max_fee_per_gas)?,
                    max_priority_fee_per_gas: wei_to_gwei_decimal!(fees.medium.max_priority_fee_per_gas)?,
                    min_wait_time: fees.medium.min_wait_time,
                    max_wait_time: fees.medium.max_wait_time,
                },
                high: FeePerGasLevel {
                    max_fee_per_gas: wei_to_gwei_decimal!(fees.high.max_fee_per_gas)?,
                    max_priority_fee_per_gas: wei_to_gwei_decimal!(fees.high.max_priority_fee_per_gas)?,
                    min_wait_time: fees.high.min_wait_time,
                    max_wait_time: fees.high.max_wait_time,
                },
                source: fees.source.to_string(),
                base_fee_trend: fees.base_fee_trend,
                priority_fee_trend: fees.priority_fee_trend,
                units: EstimationUnits::Gwei,
            })
        }
    }
}
