//! # AfterHours
//!
//! A 24/7 price for a tokenized stock on Robinhood Chain.
//!
//! Chainlink's stock feeds follow US market hours (24/5): every weekend they go
//! silent for ~52 hours, ~76 on holiday weekends, while the stock token keeps
//! trading on-chain. AfterHours is a drop-in feed that
//!
//! * passes the Chainlink answer through while it is fresh (`LIVE_FEED`),
//! * otherwise prices the token from where it actually trades: a time-weighted
//!   average of the Uniswap v3 pool, bounded to a per-asset band around the
//!   last exchange print and refused when the pool is too thin (`ONCHAIN_TWAP`),
//! * refuses to price while the issuer has paused the token's oracle for a
//!   corporate action (`PAUSED`), when neither source is usable, or when the
//!   last exchange print is older than any market closure can explain
//!   (`NO_DATA`).
//!
//! It speaks Chainlink's `AggregatorV3Interface` (plus the v2 getters) and
//! Morpho's `IOracle`, so a lending market swaps one address and keeps working
//! through the weekend. The configuration is fixed at deployment; there is no
//! owner and no upgrade.
#![cfg_attr(not(any(test, feature = "export-abi")), no_main)]
#![cfg_attr(not(any(test, feature = "export-abi")), no_std)]
#![allow(clippy::type_complexity, clippy::too_many_arguments)]

#[macro_use]
extern crate alloc;

use alloc::{string::String, vec::Vec};

use alloy_primitives::{
    aliases::{U128, U160, U32, U64, U8, U80},
    Address, I256, U256,
};
use alloy_sol_types::sol;
use stylus_sdk::prelude::*;

pub mod tickmath;

/// Session codes returned by `state()`.
pub const SESSION_LIVE_FEED: u8 = 0;
pub const SESSION_ONCHAIN_TWAP: u8 = 1;
pub const SESSION_PAUSED: u8 = 2;
pub const SESSION_NO_DATA: u8 = 3;

/// Reason codes carried by `NoData(reason)` and `state()`.
pub const REASON_NONE: u8 = 0;
pub const REASON_FEED_INVALID: u8 = 1;
pub const REASON_POOL_TOO_THIN: u8 = 2;
pub const REASON_TWAP_UNAVAILABLE: u8 = 3;
/// The last exchange print is older than `maxAnchorAge`: the feed is dead or
/// the stock is halted, and no band anchored to it can be trusted.
pub const REASON_ANCHOR_STALE: u8 = 4;

/// `InvalidConfig(reason)` codes raised by `initialize`.
pub const CONFIG_ZERO_LIVE_MAX_AGE: u8 = 1;
pub const CONFIG_ZERO_TWAP_WINDOW: u8 = 2;
pub const CONFIG_BAD_DEVIATION: u8 = 3;
pub const CONFIG_ZERO_MIN_LIQUIDITY: u8 = 4;
pub const CONFIG_STOCK_NOT_IN_POOL: u8 = 5;
pub const CONFIG_SCALE_UNDERFLOW: u8 = 6;
pub const CONFIG_DECIMALS_TOO_LARGE: u8 = 7;
/// `maxAnchorAge` must exceed `liveMaxAge`, otherwise the TWAP path can never run.
pub const CONFIG_ANCHOR_AGE: u8 = 8;
pub const CONFIG_WINDOW_TOO_LONG: u8 = 9;
/// `observe([twapWindow, 0])` failed at deployment: not a v3 pool, or no history yet.
pub const CONFIG_POOL_NOT_OBSERVABLE: u8 = 10;

const BPS: u64 = 10_000;
/// Longest TWAP window accepted (one day); longer windows are always unavailable.
const MAX_TWAP_WINDOW: u32 = 86_400;
/// Token/feed decimals above this would overflow the fixed-point scales.
const MAX_DECIMALS: u8 = 36;

sol_interface! {
    interface IAggregatorV3 {
        function decimals() external view returns (uint8);
        function description() external view returns (string);
        function latestRoundData() external view returns (uint80, int256, uint256, uint256, uint80);
        function getRoundData(uint80 round_id) external view returns (uint80, int256, uint256, uint256, uint80);
    }

    interface IUniswapV3PoolMinimal {
        function token0() external view returns (address);
        function token1() external view returns (address);
        function observe(uint32[] seconds_agos) external view returns (int56[], uint160[]);
    }

    interface IERC20Decimals {
        function decimals() external view returns (uint8);
    }

    interface IStockOraclePause {
        function oraclePaused() external view returns (bool);
    }
}

sol! {
    /// The issuer has paused the token's oracle (corporate action in progress).
    #[derive(Debug, PartialEq, Eq)]
    error IssuerPaused();
    /// Neither the feed nor the pool can be trusted right now (REASON_* codes).
    #[derive(Debug, PartialEq, Eq)]
    error NoData(uint8 reason);
    /// A read from one of the configured contracts failed.
    #[derive(Debug, PartialEq, Eq)]
    error CallFailed(address target);
    /// A deployment parameter is inconsistent (CONFIG_* codes).
    #[derive(Debug, PartialEq, Eq)]
    error InvalidConfig(uint8 reason);
    /// `initialize` has already run; the configuration is permanent.
    #[derive(Debug, PartialEq, Eq)]
    error AlreadyInitialized();
    /// `initialize` has not run yet.
    #[derive(Debug, PartialEq, Eq)]
    error NotInitialized();
}

#[derive(SolidityError, Debug)]
pub enum AfterHoursError {
    IssuerPaused(IssuerPaused),
    NoData(NoData),
    CallFailed(CallFailed),
    InvalidConfig(InvalidConfig),
    AlreadyInitialized(AlreadyInitialized),
    NotInitialized(NotInitialized),
}

sol_storage! {
    #[entrypoint]
    pub struct AfterHours {
        /// Set once by `initialize`; every read refuses until then.
        bool initialized;
        /// The account (tx.origin) that ran `initialize`, recorded for transparency only.
        address initializer;
        /// Chainlink-style feed for the stock (8 decimals on Robinhood Chain).
        address feed;
        /// Uniswap v3 pool where the stock trades against the quote token.
        address pool;
        /// The stock token (also queried for the issuer's oracle pause flag).
        address stock;
        /// The quote token of the pool (USDG on Robinhood Chain).
        address quote;
        bool stock_is_token0;
        uint8 feed_decimals;
        uint8 stock_decimals;
        uint8 quote_decimals;
        /// Feed answers older than this many seconds are treated as closed-market.
        uint64 live_max_age;
        /// Seconds of pool history averaged while the market is closed.
        uint32 twap_window;
        /// Widest move (basis points) the on-chain price may make away from the last feed print.
        uint64 max_deviation_bps;
        /// Lowest window-averaged pool liquidity the TWAP is trusted at.
        uint128 min_liquidity;
        /// Oldest feed print the band may be anchored to; beyond it every read refuses.
        uint64 max_anchor_age;
        /// 10^(36 + quote_decimals - stock_decimals - feed_decimals): Morpho's price scale.
        uint256 morpho_scale;
    }
}

/// Everything one evaluation of the oracle knows. `state()` returns it as a tuple.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Quote {
    pub session: u8,
    pub reason: u8,
    /// The price this oracle stands behind, in feed decimals (0 when it refuses).
    pub answer: U256,
    pub feed_round_id: U80,
    pub feed_answer: U256,
    pub feed_started_at: U256,
    pub feed_updated_at: U256,
    pub feed_answered_in_round: U80,
    /// Raw pool TWAP in feed decimals before the band is applied (0 outside ONCHAIN_TWAP).
    pub twap: U256,
    /// Harmonic-mean in-range liquidity over the TWAP window (0 unless the pool was read).
    pub liquidity: u128,
    /// True when the TWAP was pulled back to the edge of the allowed band.
    pub clamped: bool,
}

#[public]
impl AfterHours {
    /// Fixes the configuration forever. Runs once, right after deployment and
    /// activation, from the same script (Robinhood Chain mainnet has no
    /// StylusDeployer factory, so a constructor cannot be used there). Token
    /// order and decimals are read from the contracts themselves, and every
    /// read the oracle will ever make is exercised once, so a wrong pool or a
    /// token without the pause flag fails here rather than at the first weekend.
    pub fn initialize(
        &mut self,
        feed: Address,
        pool: Address,
        stock: Address,
        live_max_age: u64,
        twap_window: u32,
        max_deviation_bps: u64,
        min_liquidity: u128,
        max_anchor_age: u64,
    ) -> Result<(), AfterHoursError> {
        if self.initialized.get() {
            return Err(AfterHoursError::AlreadyInitialized(AlreadyInitialized {}));
        }
        if live_max_age == 0 {
            return Err(invalid(CONFIG_ZERO_LIVE_MAX_AGE));
        }
        if twap_window == 0 {
            return Err(invalid(CONFIG_ZERO_TWAP_WINDOW));
        }
        if twap_window > MAX_TWAP_WINDOW {
            return Err(invalid(CONFIG_WINDOW_TOO_LONG));
        }
        if max_deviation_bps == 0 || max_deviation_bps >= BPS {
            return Err(invalid(CONFIG_BAD_DEVIATION));
        }
        if min_liquidity == 0 {
            return Err(invalid(CONFIG_ZERO_MIN_LIQUIDITY));
        }
        if max_anchor_age <= live_max_age {
            return Err(invalid(CONFIG_ANCHOR_AGE));
        }

        let pool_iface = IUniswapV3PoolMinimal::new(pool);
        let token0 = pool_iface
            .token_0(self.vm(), Call::new())
            .map_err(|_| call_failed(pool))?;
        let token1 = pool_iface
            .token_1(self.vm(), Call::new())
            .map_err(|_| call_failed(pool))?;
        let (stock_is_token0, quote) = if token0 == stock {
            (true, token1)
        } else if token1 == stock {
            (false, token0)
        } else {
            return Err(invalid(CONFIG_STOCK_NOT_IN_POOL));
        };
        // A v2 pair has token0/token1 too; only a v3 pool with enough history
        // answers observe() for the window this oracle will ask for.
        match pool_iface.observe(self.vm(), Call::new(), vec![twap_window, 0]) {
            Ok((cumulatives, seconds_per_liquidity))
                if cumulatives.len() == 2 && seconds_per_liquidity.len() == 2 => {}
            _ => return Err(invalid(CONFIG_POOL_NOT_OBSERVABLE)),
        }

        let feed_decimals = IAggregatorV3::new(feed)
            .decimals(self.vm(), Call::new())
            .map_err(|_| call_failed(feed))?;
        let stock_decimals = IERC20Decimals::new(stock)
            .decimals(self.vm(), Call::new())
            .map_err(|_| call_failed(stock))?;
        let quote_decimals = IERC20Decimals::new(quote)
            .decimals(self.vm(), Call::new())
            .map_err(|_| call_failed(quote))?;
        if feed_decimals > MAX_DECIMALS
            || stock_decimals > MAX_DECIMALS
            || quote_decimals > MAX_DECIMALS
        {
            return Err(invalid(CONFIG_DECIMALS_TOO_LARGE));
        }
        // The pause flag must be readable now, so a token without it fails at
        // deployment rather than silently at the first weekend.
        IStockOraclePause::new(stock)
            .oracle_paused(self.vm(), Call::new())
            .map_err(|_| call_failed(stock))?;

        // Morpho: 1e36 * 10^loan / (10^collateral * 10^feed) with loan = quote.
        let scale_num = 36u64 + u64::from(quote_decimals);
        let scale_den = u64::from(stock_decimals) + u64::from(feed_decimals);
        if scale_den > scale_num {
            return Err(invalid(CONFIG_SCALE_UNDERFLOW));
        }
        let morpho_scale = U256::from(10u64).pow(U256::from(scale_num - scale_den));

        self.initialized.set(true);
        self.initializer.set(self.vm().tx_origin());
        self.feed.set(feed);
        self.pool.set(pool);
        self.stock.set(stock);
        self.quote.set(quote);
        self.stock_is_token0.set(stock_is_token0);
        self.feed_decimals.set(U8::from(feed_decimals));
        self.stock_decimals.set(U8::from(stock_decimals));
        self.quote_decimals.set(U8::from(quote_decimals));
        self.live_max_age.set(U64::from(live_max_age));
        self.twap_window.set(U32::from(twap_window));
        self.max_deviation_bps.set(U64::from(max_deviation_bps));
        self.min_liquidity.set(U128::from(min_liquidity));
        self.max_anchor_age.set(U64::from(max_anchor_age));
        self.morpho_scale.set(morpho_scale);
        Ok(())
    }

    // ---- Chainlink AggregatorV3Interface -------------------------------------

    pub fn decimals(&self) -> u8 {
        self.feed_decimals.get().to::<u8>()
    }

    pub fn description(&self) -> Result<String, AfterHoursError> {
        let feed = self.feed.get();
        let mut out = IAggregatorV3::new(feed)
            .description(self.vm(), Call::new())
            .map_err(|_| call_failed(feed))?;
        out.push_str(" (AfterHours)");
        Ok(out)
    }

    pub fn version(&self) -> U256 {
        U256::from(1u64)
    }

    /// Chainlink-shaped answer. While the feed is fresh this is the feed's own
    /// round verbatim. While the market is closed, `answer` is the bounded pool
    /// TWAP, `startedAt` is the last exchange print and `updatedAt` is now.
    /// `roundId` is always the feed's, so it does not advance between reads
    /// in ONCHAIN_TWAP mode. Reverts with `IssuerPaused` or `NoData` when there
    /// is nothing to stand behind.
    pub fn latest_round_data(&self) -> Result<(U80, I256, U256, U256, U80), AfterHoursError> {
        let q = self.evaluate()?;
        self.round_tuple(&q)
    }

    /// Historical rounds are the feed's, verbatim. The feed's current round is
    /// answered exactly like `latestRoundData()` so the two never disagree.
    pub fn get_round_data(
        &self,
        round_id: U80,
    ) -> Result<(U80, I256, U256, U256, U80), AfterHoursError> {
        let q = self.evaluate()?;
        if round_id == q.feed_round_id {
            return self.round_tuple(&q);
        }
        let feed = self.feed.get();
        IAggregatorV3::new(feed)
            .get_round_data(self.vm(), Call::new(), round_id)
            .map_err(|_| call_failed(feed))
    }

    // ---- Chainlink AggregatorInterface (v2 getters) ---------------------------

    pub fn latest_answer(&self) -> Result<I256, AfterHoursError> {
        let q = self.evaluate()?;
        Ok(I256::from_raw(require_price(&q)?))
    }

    pub fn latest_timestamp(&self) -> Result<U256, AfterHoursError> {
        let q = self.evaluate()?;
        require_price(&q)?;
        Ok(self.updated_at(&q))
    }

    pub fn latest_round(&self) -> Result<U256, AfterHoursError> {
        let q = self.evaluate()?;
        require_price(&q)?;
        Ok(U256::from(q.feed_round_id))
    }

    // ---- Morpho Blue IOracle ---------------------------------------------------

    /// Quote-token value of one raw stock unit, scaled by 1e36 (Morpho's convention).
    pub fn price(&self) -> Result<U256, AfterHoursError> {
        let q = self.evaluate()?;
        let answer = require_price(&q)?;
        // An answer too large for Morpho's 1e36 scale is not a price we can stand behind.
        answer
            .checked_mul(self.morpho_scale.get())
            .ok_or(AfterHoursError::NoData(NoData {
                reason: REASON_FEED_INVALID,
            }))
    }

    // ---- AfterHours -----------------------------------------------------------

    /// The full picture, never reverting for market reasons (only for failed reads):
    /// (session, reason, answer, feedAnswer, feedUpdatedAt, twap, liquidity, clamped).
    pub fn state(&self) -> Result<(u8, u8, U256, U256, U256, U256, u128, bool), AfterHoursError> {
        let q = self.evaluate()?;
        Ok((
            q.session,
            q.reason,
            q.answer,
            q.feed_answer,
            q.feed_updated_at,
            q.twap,
            q.liquidity,
            q.clamped,
        ))
    }

    /// Deployment parameters:
    /// (initialized, initializer, feed, pool, stock, quote, stockIsToken0, feedDecimals,
    ///  stockDecimals, quoteDecimals, liveMaxAge, twapWindow, maxDeviationBps, minLiquidity,
    ///  maxAnchorAge).
    pub fn config(
        &self,
    ) -> (
        bool,
        Address,
        Address,
        Address,
        Address,
        Address,
        bool,
        u8,
        u8,
        u8,
        u64,
        u32,
        u64,
        u128,
        u64,
    ) {
        (
            self.initialized.get(),
            self.initializer.get(),
            self.feed.get(),
            self.pool.get(),
            self.stock.get(),
            self.quote.get(),
            self.stock_is_token0.get(),
            self.feed_decimals.get().to::<u8>(),
            self.stock_decimals.get().to::<u8>(),
            self.quote_decimals.get().to::<u8>(),
            self.live_max_age.get().to::<u64>(),
            self.twap_window.get().to::<u32>(),
            self.max_deviation_bps.get().to::<u64>(),
            self.min_liquidity.get().to::<u128>(),
            self.max_anchor_age.get().to::<u64>(),
        )
    }
}

impl AfterHours {
    /// One evaluation of both sources. Only failed reads are errors; every
    /// market condition is reported through `Quote.session` / `Quote.reason`.
    pub fn evaluate(&self) -> Result<Quote, AfterHoursError> {
        if !self.initialized.get() {
            return Err(AfterHoursError::NotInitialized(NotInitialized {}));
        }
        let feed = self.feed.get();
        let stock = self.stock.get();
        let pool = self.pool.get();

        let paused = IStockOraclePause::new(stock)
            .oracle_paused(self.vm(), Call::new())
            .map_err(|_| call_failed(stock))?;

        let (round_id, feed_answer, started_at, updated_at, answered_in_round) =
            IAggregatorV3::new(feed)
                .latest_round_data(self.vm(), Call::new())
                .map_err(|_| call_failed(feed))?;

        let mut q = Quote {
            feed_round_id: round_id,
            feed_started_at: started_at,
            feed_updated_at: updated_at,
            feed_answered_in_round: answered_in_round,
            ..Quote::default()
        };

        let now = U256::from(self.vm().block_timestamp());
        // A completed round has a positive answer and a past, non-zero timestamp.
        let feed_valid = feed_answer > I256::ZERO && !updated_at.is_zero() && updated_at <= now;
        if feed_valid {
            q.feed_answer = feed_answer.into_raw();
        }

        // The issuer's flag wins over everything: a corporate action is being
        // processed and neither the feed nor the pool price means what it says.
        if paused {
            q.session = SESSION_PAUSED;
            return Ok(q);
        }
        if !feed_valid {
            q.session = SESSION_NO_DATA;
            q.reason = REASON_FEED_INVALID;
            return Ok(q);
        }
        let feed_answer = q.feed_answer;

        let age = now - updated_at;
        if age <= U256::from(self.live_max_age.get()) {
            q.session = SESSION_LIVE_FEED;
            q.answer = feed_answer;
            return Ok(q);
        }
        // Older than any market closure explains: the feed is gone or the stock
        // is halted, and a band around that print would only look fresh.
        if age > U256::from(self.max_anchor_age.get()) {
            q.session = SESSION_NO_DATA;
            q.reason = REASON_ANCHOR_STALE;
            return Ok(q);
        }

        // Closed market: price from the pool, guarded by depth and the band.
        let window = self.twap_window.get().to::<u32>();
        let observed = IUniswapV3PoolMinimal::new(pool)
            .observe(self.vm(), Call::new(), vec![window, 0])
            .ok()
            .filter(|(ticks, liq)| ticks.len() == 2 && liq.len() == 2);
        // A pool with too little history reverts with "OLD"; treat it as no data.
        let Some((cumulatives, seconds_per_liquidity)) = observed else {
            q.session = SESSION_NO_DATA;
            q.reason = REASON_TWAP_UNAVAILABLE;
            return Ok(q);
        };

        // Harmonic-mean liquidity over the window (Uniswap OracleLibrary.consult):
        // liquidity added in the last block cannot make a thinned pool look deep.
        let spl_delta = seconds_per_liquidity[1].wrapping_sub(seconds_per_liquidity[0]);
        let Some(liquidity) = harmonic_liquidity(window, spl_delta) else {
            q.session = SESSION_NO_DATA;
            q.reason = REASON_TWAP_UNAVAILABLE;
            return Ok(q);
        };
        q.liquidity = liquidity;
        if liquidity < self.min_liquidity.get().to::<u128>() {
            q.session = SESSION_NO_DATA;
            q.reason = REASON_POOL_TOO_THIN;
            return Ok(q);
        }

        let twap = tickmath::mean_tick(cumulatives[0].as_i64(), cumulatives[1].as_i64(), window)
            .and_then(tickmath::ratio_q96)
            .and_then(|ratio| {
                tickmath::stock_price(
                    ratio,
                    self.stock_is_token0.get(),
                    self.stock_decimals.get().to::<u8>(),
                    self.quote_decimals.get().to::<u8>(),
                    self.feed_decimals.get().to::<u8>(),
                )
            });
        let Some(twap) = twap else {
            q.session = SESSION_NO_DATA;
            q.reason = REASON_TWAP_UNAVAILABLE;
            return Ok(q);
        };
        q.twap = twap;

        let dev = U256::from(self.max_deviation_bps.get());
        let bps = U256::from(BPS);
        // The band is anchored to the last feed print; a print so large that the
        // arithmetic overflows is not a print to anchor to.
        let (Some(lower), Some(upper)) = (
            feed_answer.checked_mul(bps - dev).map(|v| v / bps),
            feed_answer.checked_mul(bps + dev).map(|v| v / bps),
        ) else {
            q.session = SESSION_NO_DATA;
            q.reason = REASON_FEED_INVALID;
            return Ok(q);
        };
        let (answer, clamped) = if twap < lower {
            (lower, true)
        } else if twap > upper {
            (upper, true)
        } else {
            (twap, false)
        };
        q.session = SESSION_ONCHAIN_TWAP;
        q.answer = answer;
        q.clamped = clamped;
        Ok(q)
    }

    fn updated_at(&self, q: &Quote) -> U256 {
        if q.session == SESSION_LIVE_FEED {
            q.feed_updated_at
        } else {
            U256::from(self.vm().block_timestamp())
        }
    }

    fn round_tuple(&self, q: &Quote) -> Result<(U80, I256, U256, U256, U80), AfterHoursError> {
        let answer = require_price(q)?;
        let started_at = if q.session == SESSION_LIVE_FEED {
            q.feed_started_at
        } else {
            q.feed_updated_at
        };
        Ok((
            q.feed_round_id,
            I256::from_raw(answer),
            started_at,
            self.updated_at(q),
            q.feed_answered_in_round,
        ))
    }
}

/// `window * 2^128 / delta(secondsPerLiquidityCumulativeX128)`, saturating at
/// `u128::MAX`. `None` when the pool recorded no liquidity-seconds (window 0
/// or identical observations), which is not a pool to price from.
fn harmonic_liquidity(window: u32, spl_delta: U160) -> Option<u128> {
    if window == 0 || spl_delta.is_zero() {
        return None;
    }
    let numerator = U256::from(window) << 128usize;
    let liquidity = numerator / U256::from(spl_delta);
    Some(liquidity.try_into().unwrap_or(u128::MAX))
}

fn require_price(q: &Quote) -> Result<U256, AfterHoursError> {
    match q.session {
        SESSION_PAUSED => Err(AfterHoursError::IssuerPaused(IssuerPaused {})),
        SESSION_NO_DATA => Err(AfterHoursError::NoData(NoData { reason: q.reason })),
        _ => Ok(q.answer),
    }
}

fn call_failed(target: Address) -> AfterHoursError {
    AfterHoursError::CallFailed(CallFailed { target })
}

fn invalid(reason: u8) -> AfterHoursError {
    AfterHoursError::InvalidConfig(InvalidConfig { reason })
}

#[cfg(test)]
mod mockvm;
#[cfg(test)]
mod tests;
