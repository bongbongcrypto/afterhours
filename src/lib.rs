//! # AfterHours
//!
//! A 24/7 price for a tokenized stock on Robinhood Chain.
//!
//! Chainlink's stock feeds follow US market hours (24/5): every weekend they go
//! silent for ~52 hours, ~76 on holiday weekends, while the stock token keeps
//! trading on-chain. AfterHours is a drop-in feed that
//!
//! * passes the Chainlink answer through while it is fresh (`LIVE_FEED`),
//! * otherwise prices the token from where it actually trades: the median of
//!   three 10-minute time-weighted averages of its primary Uniswap v3 pool,
//!   bounded to a band around the last exchange print and refused when that
//!   pool is too thin (`ONCHAIN_TWAP`). The band is narrow while the print is
//!   younger than the feed's heartbeat on a weekday (a feed that is only
//!   quiet looks exactly like one that has just closed) and wide after it
//!   and all weekend. The primary is fixed at deployment; standby pools are
//!   read only while the primary cannot be observed, so nobody can move the
//!   price source to a pool they control,
//! * refuses to price while the issuer has paused the token's oracle for a
//!   corporate action (`PAUSED`), when neither source is usable, or when the
//!   last exchange print is older than any market closure can explain
//!   (`NO_DATA`).
//!
//! It speaks Chainlink's `AggregatorV3Interface` (plus the v2 getters) and
//! Morpho's `IOracle`: a feed consumer swaps one address, and a Morpho market
//! opened with it as its oracle keeps working through the weekend. The
//! configuration is fixed at deployment; there is no owner and no upgrade.
//!
//! Units: Chainlink prices one share. A Robinhood stock token is a scaled-UI
//! token that keeps balances raw: one token of raw balance is
//! `uiMultiplier / 1e18` shares, and the multiplier
//! grows with every dividend and jumps on a split. Every Chainlink-shaped
//! answer here is per share, like the feed; `price()` (Morpho) values raw
//! collateral units, so it multiplies by the token's current `uiMultiplier`.
#![cfg_attr(not(any(test, feature = "export-abi")), no_main)]
#![cfg_attr(not(any(test, feature = "export-abi")), no_std)]
#![allow(clippy::type_complexity, clippy::too_many_arguments)]

#[macro_use]
extern crate alloc;

use alloc::{string::String, vec::Vec};

use alloy_primitives::{
    aliases::{I56, U128, U160, U32, U64, U8, U80},
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
/// A new share multiplier took effect after the last print (a corporate action)
/// and the pool, read per share, sits outside the wide band: a split or a merger
/// the last print knows nothing about. Refuse until the feed prints again.
pub const REASON_MULTIPLIER_CHANGED: u8 = 5;
/// Only `price()` raises this: the answer is valid, but Morpho's 1e36 scale
/// times the share multiplier cannot hold it (an absurd price).
pub const REASON_PRICE_OVERFLOW: u8 = 6;

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
/// `observe()` at the four sub-window boundaries failed at deployment: not a v3
/// pool, or not enough history yet.
pub const CONFIG_POOL_NOT_OBSERVABLE: u8 = 10;
/// Between 1 and `MAX_POOLS` pools must be given.
pub const CONFIG_POOL_COUNT: u8 = 11;
/// Every pool must pair the stock with the same quote token.
pub const CONFIG_POOL_QUOTE_MISMATCH: u8 = 12;
/// The same pool was given twice.
pub const CONFIG_DUPLICATE_POOL: u8 = 13;
/// The stock's `uiMultiplier()` answered zero.
pub const CONFIG_BAD_MULTIPLIER: u8 = 14;
/// `heartbeat` must lie in `[liveMaxAge, maxAnchorAge)` and `quietBandBps` in
/// `(0, maxDeviationBps]`.
pub const CONFIG_QUIET_TIER: u8 = 15;
/// The primary pool keeps fewer than `twapWindow + 1` observations. Uniswap
/// writes at most one a second, so anyone swapping every second could then
/// make `observe()` answer "OLD" for the window and switch pricing off.
pub const CONFIG_PRIMARY_HISTORY_TOO_SHORT: u8 = 16;

const BPS: u64 = 10_000;
/// Longest TWAP window accepted (one day); longer windows are always unavailable.
const MAX_TWAP_WINDOW: u32 = 86_400;
/// Token/feed decimals above this would overflow the fixed-point scales.
const MAX_DECIMALS: u8 = 36;
/// Primary plus standby pools. Standbys are read only while the primary
/// cannot be observed.
const MAX_POOLS: usize = 3;
/// The window is judged in three sub-windows, price and depth alike: the
/// answer is the median sub-window's average and the median sub-window's
/// liquidity must clear the floor, so whatever happens inside one sub-window
/// alone neither moves the price nor switches pricing off.
const SUBWINDOWS: u32 = 3;
/// Shortest TWAP window: each sub-window must last at least a second.
const MIN_TWAP_WINDOW: u32 = SUBWINDOWS;
/// 1e18, the token's multiplier for "one token of raw balance is one share".
const ONE_SHARE: U256 = U256::from_limbs([1_000_000_000_000_000_000, 0, 0, 0]);

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
        function slot0() external view returns (uint160, int24, uint16, uint16, uint16, uint8, bool);
    }

    interface IERC20Decimals {
        function decimals() external view returns (uint8);
    }

    interface IStockOraclePause {
        function oraclePaused() external view returns (bool);
        function uiMultiplier() external view returns (uint256);
        function effectiveAt() external view returns (uint256);
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
        /// Uniswap v3 pools where the stock trades against the quote token (1-3).
        /// pools[0] is the primary and prices the asset; the others are standbys,
        /// read in order only while every earlier pool cannot be observed.
        address[] pools;
        /// Per pool: true when the stock is token0 (the pool's tick is quote per stock).
        bool[] pool_stock_is_token0;
        /// The stock token (also queried for the issuer's oracle pause flag and
        /// its share multiplier).
        address stock;
        /// The quote token shared by every pool (USDG on Robinhood Chain).
        address quote;
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
        /// The feed's heartbeat (seconds). On a weekday a print younger than
        /// this may come from a feed that is running but quiet, so the pool is
        /// held to `quiet_band_bps` around it; past it, and at weekends, the
        /// market is closed and the band is `max_deviation_bps`.
        uint64 heartbeat;
        /// Band (basis points) while the last print is at most `heartbeat` old.
        uint64 quiet_band_bps;
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
    /// The median sub-window's pool average, per share, in feed decimals,
    /// before the band is applied (0 outside ONCHAIN_TWAP).
    pub twap: U256,
    /// Median of the three sub-windows' harmonic-mean in-range liquidity of
    /// `pool` (0 when no pool was read).
    pub liquidity: u128,
    /// True when the TWAP was pulled back to the edge of the allowed band.
    pub clamped: bool,
    /// The pool that was observed to price (or to refuse: NO_DATA 2, 5 and the
    /// tick conversion case of 3). Zero when no pool was needed or none could be observed.
    pub pool: Address,
    /// The stock's share multiplier at this read (1e18 = one token of raw balance is one share).
    pub multiplier: U256,
}

#[public]
impl AfterHours {
    /// Fixes the configuration forever. Runs once, right after deployment and
    /// activation, from the same script (Robinhood Chain mainnet has no
    /// StylusDeployer factory, so a constructor cannot be used there). Token
    /// order and decimals are read from the contracts themselves, and every
    /// read on the price path (feed round, every pool's observe, pause flag) is
    /// exercised once, so a wrong pool or a token without the pause flag fails
    /// here rather than at the first weekend. `pools` holds one to three
    /// distinct Uniswap v3 pools of the stock against one quote token; the
    /// first is the primary (give the deepest fee tier), the rest are standbys.
    /// The stock must expose `oraclePaused()`, `uiMultiplier()` and `effectiveAt()`.
    /// `heartbeat` and `quiet_band_bps` set the quiet tier: a print at most
    /// `heartbeat` seconds old holds the pool to `quiet_band_bps` around it.
    pub fn initialize(
        &mut self,
        feed: Address,
        pools: Vec<Address>,
        stock: Address,
        live_max_age: u64,
        twap_window: u32,
        max_deviation_bps: u64,
        min_liquidity: u128,
        max_anchor_age: u64,
        heartbeat: u64,
        quiet_band_bps: u64,
    ) -> Result<(), AfterHoursError> {
        if self.initialized.get() {
            return Err(AfterHoursError::AlreadyInitialized(AlreadyInitialized {}));
        }
        if live_max_age == 0 {
            return Err(invalid(CONFIG_ZERO_LIVE_MAX_AGE));
        }
        if twap_window < MIN_TWAP_WINDOW {
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
        // The quiet tier sits between the live feed and the anchor limit, and
        // is never wider than the closed-market band. `heartbeat == liveMaxAge`
        // leaves it empty.
        if heartbeat < live_max_age
            || heartbeat >= max_anchor_age
            || quiet_band_bps == 0
            || quiet_band_bps > max_deviation_bps
        {
            return Err(invalid(CONFIG_QUIET_TIER));
        }
        if pools.is_empty() || pools.len() > MAX_POOLS {
            return Err(invalid(CONFIG_POOL_COUNT));
        }
        for (i, pool) in pools.iter().enumerate() {
            if pools[..i].contains(pool) {
                return Err(invalid(CONFIG_DUPLICATE_POOL));
            }
        }

        let mut quote = Address::ZERO;
        let mut stock_first: Vec<bool> = Vec::with_capacity(pools.len());
        for (i, &pool) in pools.iter().enumerate() {
            let pool_iface = IUniswapV3PoolMinimal::new(pool);
            let token0 = pool_iface
                .token_0(self.vm(), Call::new())
                .map_err(|_| call_failed(pool))?;
            let token1 = pool_iface
                .token_1(self.vm(), Call::new())
                .map_err(|_| call_failed(pool))?;
            let (is_token0, other) = if token0 == stock {
                (true, token1)
            } else if token1 == stock {
                (false, token0)
            } else {
                return Err(invalid(CONFIG_STOCK_NOT_IN_POOL));
            };
            if quote == Address::ZERO {
                quote = other;
            } else if other != quote {
                return Err(invalid(CONFIG_POOL_QUOTE_MISMATCH));
            }
            // A v2 pair has token0/token1 too; only a v3 pool with enough history
            // answers observe() for the points this oracle will ask for.
            let points = observation_points(twap_window);
            match pool_iface.observe(self.vm(), Call::new(), points.clone()) {
                Ok((cumulatives, seconds_per_liquidity))
                    if cumulatives.len() == points.len()
                        && seconds_per_liquidity.len() == points.len() => {}
                _ => return Err(invalid(CONFIG_POOL_NOT_OBSERVABLE)),
            }
            // The primary must keep a window's worth of one-a-second observations
            // (slot0's fourth field). A standby only covers a primary that cannot
            // be observed, so it may keep fewer.
            if i == 0 {
                let (_, _, _, cardinality, ..) = pool_iface
                    .slot0(self.vm(), Call::new())
                    .map_err(|_| call_failed(pool))?;
                if u32::from(cardinality) < twap_window + 1 {
                    return Err(invalid(CONFIG_PRIMARY_HISTORY_TOO_SHORT));
                }
            }
            stock_first.push(is_token0);
        }

        let feed_iface = IAggregatorV3::new(feed);
        let feed_decimals = feed_iface
            .decimals(self.vm(), Call::new())
            .map_err(|_| call_failed(feed))?;
        // The round read is the one every evaluation starts with; a feed that
        // answers decimals() but not latestRoundData() must fail here.
        feed_iface
            .latest_round_data(self.vm(), Call::new())
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
        // The pause flag and the share multiplier must be readable now, so a
        // token without them fails at deployment rather than at the first weekend.
        let stock_iface = IStockOraclePause::new(stock);
        stock_iface
            .oracle_paused(self.vm(), Call::new())
            .map_err(|_| call_failed(stock))?;
        let multiplier = stock_iface
            .ui_multiplier(self.vm(), Call::new())
            .map_err(|_| call_failed(stock))?;
        stock_iface
            .effective_at(self.vm(), Call::new())
            .map_err(|_| call_failed(stock))?;
        if multiplier.is_zero() {
            return Err(invalid(CONFIG_BAD_MULTIPLIER));
        }

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
        for (&pool, &is_token0) in pools.iter().zip(stock_first.iter()) {
            self.pools.push(pool);
            self.pool_stock_is_token0.push(is_token0);
        }
        self.stock.set(stock);
        self.quote.set(quote);
        self.feed_decimals.set(U8::from(feed_decimals));
        self.stock_decimals.set(U8::from(stock_decimals));
        self.quote_decimals.set(U8::from(quote_decimals));
        self.live_max_age.set(U64::from(live_max_age));
        self.twap_window.set(U32::from(twap_window));
        self.max_deviation_bps.set(U64::from(max_deviation_bps));
        self.min_liquidity.set(U128::from(min_liquidity));
        self.max_anchor_age.set(U64::from(max_anchor_age));
        self.morpho_scale.set(morpho_scale);
        self.heartbeat.set(U64::from(heartbeat));
        self.quiet_band_bps.set(U64::from(quiet_band_bps));
        Ok(())
    }

    // ---- Chainlink AggregatorV3Interface -------------------------------------

    /// The feed's decimals. Refuses before `initialize`, so an integrator that
    /// reads it first never sees a plausible 0.
    pub fn decimals(&self) -> Result<u8, AfterHoursError> {
        if !self.initialized.get() {
            return Err(AfterHoursError::NotInitialized(NotInitialized {}));
        }
        Ok(self.feed_decimals.get().to::<u8>())
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

    /// Historical rounds are the feed's, verbatim, read straight from the feed:
    /// they are served while the oracle refuses to price (PAUSED / NO_DATA) and
    /// even when the stock token cannot be read. The feed's current round is
    /// answered exactly like `latestRoundData()` so the two never disagree.
    pub fn get_round_data(
        &self,
        round_id: U80,
    ) -> Result<(U80, I256, U256, U256, U80), AfterHoursError> {
        if !self.initialized.get() {
            return Err(AfterHoursError::NotInitialized(NotInitialized {}));
        }
        let feed = self.feed.get();
        let feed_iface = IAggregatorV3::new(feed);
        let (current, ..) = feed_iface
            .latest_round_data(self.vm(), Call::new())
            .map_err(|_| call_failed(feed))?;
        if round_id == current {
            let q = self.evaluate()?;
            return self.round_tuple(&q);
        }
        feed_iface
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

    /// Quote-token value of one raw stock unit, scaled by 1e36 (Morpho's
    /// convention): the per-share answer times the token's share multiplier.
    pub fn price(&self) -> Result<U256, AfterHoursError> {
        let q = self.evaluate()?;
        let answer = require_price(&q)?;
        // An answer too large for Morpho's 1e36 scale is not a price we can stand
        // behind in raw units; only this surface refuses, with its own reason.
        answer
            .checked_mul(self.morpho_scale.get())
            .and_then(|v| v.checked_mul(q.multiplier))
            .map(|v| v / ONE_SHARE)
            .ok_or(AfterHoursError::NoData(NoData {
                reason: REASON_PRICE_OVERFLOW,
            }))
    }

    // ---- AfterHours -----------------------------------------------------------

    /// The full picture, never reverting for market reasons (only for failed reads):
    /// (session, reason, answer, feedAnswer, feedUpdatedAt, twap, liquidity, clamped, pool).
    pub fn state(
        &self,
    ) -> Result<(u8, u8, U256, U256, U256, U256, u128, bool, Address), AfterHoursError> {
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
            q.pool,
        ))
    }

    /// The configured pools, in the order given to `initialize`.
    pub fn pools(&self) -> Vec<Address> {
        (0..self.pools.len())
            .filter_map(|i| self.pools.get(i))
            .collect()
    }

    /// The quiet tier: (heartbeat, quietBandBps). While the last print is at
    /// most `heartbeat` seconds old on a weekday the pool is held to
    /// `quietBandBps` around it; after that, and on Saturdays and Sundays
    /// (UTC), to `maxDeviationBps`.
    pub fn quiet_tier(&self) -> (u64, u64) {
        (
            self.heartbeat.get().to::<u64>(),
            self.quiet_band_bps.get().to::<u64>(),
        )
    }

    /// Deployment parameters:
    /// (initialized, initializer, feed, firstPool, stock, quote, stockIsToken0OfFirstPool,
    ///  feedDecimals, stockDecimals, quoteDecimals, liveMaxAge, twapWindow, maxDeviationBps,
    ///  minLiquidity, maxAnchorAge). See `pools()` for every pool and
    ///  `quietTier()` for the narrow band.
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
            self.pools.get(0).unwrap_or(Address::ZERO),
            self.stock.get(),
            self.quote.get(),
            self.pool_stock_is_token0.get(0).unwrap_or(false),
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

        let stock_iface = IStockOraclePause::new(stock);
        let paused = stock_iface
            .oracle_paused(self.vm(), Call::new())
            .map_err(|_| call_failed(stock))?;
        // One raw token unit is `multiplier / 1e18` shares; the feed prices a share.
        let multiplier = stock_iface
            .ui_multiplier(self.vm(), Call::new())
            .map_err(|_| call_failed(stock))?;
        let effective_at = stock_iface
            .effective_at(self.vm(), Call::new())
            .map_err(|_| call_failed(stock))?;
        // The token itself never answers zero (it defaults to 1e18); a zero is a broken read.
        if multiplier.is_zero() {
            return Err(call_failed(stock));
        }

        let (round_id, feed_answer, started_at, updated_at, answered_in_round) =
            IAggregatorV3::new(feed)
                .latest_round_data(self.vm(), Call::new())
                .map_err(|_| call_failed(feed))?;

        let mut q = Quote {
            feed_round_id: round_id,
            feed_started_at: started_at,
            feed_updated_at: updated_at,
            feed_answered_in_round: answered_in_round,
            multiplier,
            ..Quote::default()
        };

        let now = U256::from(self.vm().block_timestamp());
        // A completed round has a positive answer and a past, non-zero timestamp.
        let feed_valid = feed_answer > I256::ZERO && !updated_at.is_zero() && updated_at <= now;
        if feed_valid {
            q.feed_answer = feed_answer.into_raw();
        }

        // The issuer's flag wins over every market condition (only a failed
        // read ranks above it): a corporate action is being processed and
        // neither the feed nor the pool price means what it says.
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
        // A new share multiplier took effect after the last print: that print is
        // in pre-action shares, so it is not passed through. The pool, which
        // trades raw units, is read per share instead, and a move beyond the
        // wide band (a split) refuses rather than clamps.
        let rebased = effective_at > updated_at && effective_at <= now;
        if age <= U256::from(self.live_max_age.get()) && !rebased {
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

        // Within the heartbeat, on a weekday, the feed may be running and merely
        // quiet (the price has not moved its deviation threshold) or the market
        // may have closed within the last day; on-chain the two look the same.
        // The pool is held close to the print. Past the heartbeat, or on a
        // Saturday or Sunday, the market is closed and the pool may move up to
        // `max_deviation_bps`.
        let quiet =
            age <= U256::from(self.heartbeat.get()) && !is_weekend(self.vm().block_timestamp());
        let band_bps = if quiet {
            self.quiet_band_bps.get().to::<u64>()
        } else {
            self.max_deviation_bps.get().to::<u64>()
        };
        // The band is anchored to the last feed print; a print so large that the
        // arithmetic overflows is not a print to anchor to. Checked before any
        // pool read so a refusal never carries a half-computed TWAP.
        let bps = U256::from(BPS);
        let bounds = |band: u64| {
            let dev = U256::from(band);
            Some((
                feed_answer.checked_mul(bps - dev)? / bps,
                feed_answer.checked_mul(bps + dev)? / bps,
            ))
        };
        // The wide band also judges a corporate action after the print: a split
        // moves the price per share by half or more, a distribution by a few
        // percent, and only the first means the print knows nothing.
        let (Some((wide_lower, wide_upper)), Some((lower, upper))) = (
            bounds(self.max_deviation_bps.get().to::<u64>()),
            bounds(band_bps),
        ) else {
            q.session = SESSION_NO_DATA;
            q.reason = REASON_FEED_INVALID;
            return Ok(q);
        };

        // The feed is not live: price from the primary pool, guarded by depth and
        // the band. The venue is fixed: a standby is read only while every earlier pool
        // cannot be observed at all (history too short, wrong shape). A primary
        // that answers is used, thin or not; switching to whichever pool looks
        // deepest would let anyone who deepens a shallow tier choose the source.
        let window = self.twap_window.get().to::<u32>();
        let points = observation_points(window);
        let mut chosen: Option<(Address, bool, u128, Option<i32>)> = None;
        for i in 0..self.pools.len() {
            let Some(pool) = self.pools.get(i) else {
                continue;
            };
            let is_token0 = self.pool_stock_is_token0.get(i).unwrap_or(false);
            // A pool with too little history reverts with "OLD": not observable.
            let observed = IUniswapV3PoolMinimal::new(pool)
                .observe(self.vm(), Call::new(), points.clone())
                .ok()
                .filter(|(ticks, liq)| ticks.len() == points.len() && liq.len() == points.len());
            let Some((cumulatives, seconds_per_liquidity)) = observed else {
                continue;
            };
            chosen = Some((
                pool,
                is_token0,
                median_liquidity(&points, &seconds_per_liquidity),
                median_tick(&points, &cumulatives),
            ));
            break;
        }
        let Some((pool, stock_is_token0, liquidity, tick)) = chosen else {
            q.session = SESSION_NO_DATA;
            q.reason = REASON_TWAP_UNAVAILABLE;
            return Ok(q);
        };
        q.pool = pool;
        q.liquidity = liquidity;
        if liquidity < self.min_liquidity.get().to::<u128>() {
            q.session = SESSION_NO_DATA;
            q.reason = REASON_POOL_TOO_THIN;
            return Ok(q);
        }

        let twap = tick.and_then(tickmath::ratio_q96).and_then(|ratio| {
            tickmath::stock_price(
                ratio,
                stock_is_token0,
                self.stock_decimals.get().to::<u8>(),
                self.quote_decimals.get().to::<u8>(),
                self.feed_decimals.get().to::<u8>(),
            )
        });
        // The pool prices one token of raw balance; the band and every Chainlink-shaped
        // answer are per share. A price that rounds to zero is not a price.
        let Some(twap) = twap
            .and_then(|raw| raw.checked_mul(ONE_SHARE))
            .map(|v| v / multiplier)
            .filter(|v| !v.is_zero())
        else {
            q.session = SESSION_NO_DATA;
            q.reason = REASON_TWAP_UNAVAILABLE;
            return Ok(q);
        };
        q.twap = twap;
        if rebased && (twap < wide_lower || twap > wide_upper) {
            q.session = SESSION_NO_DATA;
            q.reason = REASON_MULTIPLIER_CHANGED;
            q.twap = U256::ZERO;
            return Ok(q);
        }
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

/// Saturday or Sunday in UTC. Both days always fall inside the US equity
/// closure, which runs from Friday 20:00 or 21:00 UTC to Monday 00:00 or
/// 01:00 UTC depending on daylight saving. Weekdays that are exchange
/// holidays are left to the heartbeat: a holiday table in an immutable
/// contract would go stale, the weekend never does. 1970-01-01 was a
/// Thursday, so with Monday = 0 the weekday is (days + 3) mod 7.
fn is_weekend(timestamp: u64) -> bool {
    (timestamp / 86_400 + 3) % 7 >= 5
}

/// `observe()` points: the window split into three sub-windows, oldest first.
/// With the 30-minute window: 1800, 1200, 600 and 0 seconds ago.
fn observation_points(window: u32) -> Vec<u32> {
    let sub = window / SUBWINDOWS;
    vec![window, 2 * sub, sub, 0]
}

/// Median of the sub-windows' time-weighted mean ticks (Uniswap's
/// OracleLibrary.consult per sub-window, rounded toward negative infinity).
/// A move confined to one sub-window, however far, does not reach the answer:
/// it has to show in the averages of two of the three. With the depth rule
/// at most one sub-window may be thin, so moving the answer takes at least a
/// sub-window spent at a price where the pool clears the floor. `None` when
/// a sub-window's mean cannot be formed.
fn median_tick(points: &[u32], cumulatives: &[I56]) -> Option<i32> {
    let mut ticks = (0..points.len() - 1)
        .map(|k| {
            tickmath::mean_tick(
                cumulatives[k].as_i64(),
                cumulatives[k + 1].as_i64(),
                points[k] - points[k + 1],
            )
        })
        .collect::<Option<Vec<i32>>>()?;
    ticks.sort_unstable();
    Some(ticks[ticks.len() / 2])
}

/// Median of the sub-windows' harmonic-mean liquidity (Uniswap's
/// OracleLibrary.consult per sub-window). Liquidity added in the last block
/// cannot make a pool that spent the window thin look deep, and one excursion
/// out of range drags down one sub-window, not the answer. A sub-window with no
/// liquidity-seconds recorded counts as empty.
fn median_liquidity(points: &[u32], seconds_per_liquidity: &[U160]) -> u128 {
    let mut liquidity: Vec<u128> = (0..points.len() - 1)
        .map(|k| {
            let span = points[k] - points[k + 1];
            let delta = seconds_per_liquidity[k + 1].wrapping_sub(seconds_per_liquidity[k]);
            harmonic_liquidity(span, delta).unwrap_or(0)
        })
        .collect();
    liquidity.sort_unstable();
    liquidity[liquidity.len() / 2]
}

/// `window * 2^128 / delta(secondsPerLiquidityCumulativeX128)`, saturating at
/// `u128::MAX`. `None` when the pool recorded no liquidity-seconds (window 0
/// or identical observations).
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
