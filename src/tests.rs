//! Unit tests on `MockVM` (a TestVM wrapper). Every external read is mocked with the
//! exact calldata the contract sends, encoded from the same Solidity
//! signatures, so a mismatch shows up as an unmocked call instead of a
//! silently green test.

use super::*;
use crate::mockvm::MockVM;
use alloy_primitives::{aliases::I56, aliases::U160, Address, I256, U256};
use alloy_sol_types::{sol, sol_data, SolCall, SolType, SolValue};

sol! {
    function decimals() external view returns (uint8);
    function description() external view returns (string);
    function latestRoundData() external view returns (uint80, int256, uint256, uint256, uint80);
    function getRoundData(uint80 roundId) external view returns (uint80, int256, uint256, uint256, uint80);
    function token0() external view returns (address);
    function token1() external view returns (address);
    function observe(uint32[] secondsAgos) external view returns (int56[], uint160[]);
    function oraclePaused() external view returns (bool);
}

const FEED: Address = Address::repeat_byte(0xF1);
const POOL: Address = Address::repeat_byte(0xB0);
const STOCK: Address = Address::repeat_byte(0xAA);
const USDG: Address = Address::repeat_byte(0x5D);
const DEPLOYER: Address = Address::repeat_byte(0xDE);

const LIVE_MAX_AGE: u64 = 6 * 3600;
const TWAP_WINDOW: u32 = 1800;
const MAX_DEV_BPS: u64 = 1000;
const MIN_LIQUIDITY: u128 = 1_000_000_000_000_000; // 1e15
const MAX_ANCHOR_AGE: u64 = 5 * 86_400;

const NOW: u64 = 1_800_000_000;
/// $332.52 with 8 decimals: AAPL's last Friday print on 2026-09-11.
const FRIDAY_ANSWER: u64 = 33_252_000_000;
const ROUND: u64 = 645;
/// Mean tick of the AAPL/USDG 0.05% pool. Exact value 330.96497304690...;
/// the contract truncates (integer fixed point), so it must answer ...304.
const AAPL_TICK: i64 = 218_301;
const AAPL_TWAP: u64 = 33_096_497_304;

struct World {
    vm: MockVM,
}

impl World {
    fn new() -> Self {
        let vm = MockVM::new();
        vm.set_block_timestamp(NOW);
        vm.set_tx_origin(DEPLOYER);
        let w = World { vm };
        w.mock_tokens(false);
        w.mock_paused(false);
        w.mock_observe(AAPL_TICK, MIN_LIQUIDITY);
        w.mock_feed(FRIDAY_ANSWER as i128, NOW - 120);
        w
    }

    /// token0/token1, decimals of feed and tokens. `stock_first` puts the stock at token0.
    fn mock_tokens(&self, stock_first: bool) {
        let (t0, t1) = if stock_first {
            (STOCK, USDG)
        } else {
            (USDG, STOCK)
        };
        self.vm
            .mock_static_call(POOL, token0Call {}.abi_encode(), Ok(t0.abi_encode()));
        self.vm
            .mock_static_call(POOL, token1Call {}.abi_encode(), Ok(t1.abi_encode()));
        self.vm
            .mock_static_call(FEED, decimalsCall {}.abi_encode(), Ok(enc_u8(8)));
        self.vm
            .mock_static_call(STOCK, decimalsCall {}.abi_encode(), Ok(enc_u8(18)));
        self.vm
            .mock_static_call(USDG, decimalsCall {}.abi_encode(), Ok(enc_u8(6)));
        self.vm.mock_static_call(
            FEED,
            descriptionCall {}.abi_encode(),
            Ok(String::from("AAPL / USD").abi_encode()),
        );
    }

    fn mock_paused(&self, paused: bool) {
        self.vm.mock_static_call(
            STOCK,
            oraclePausedCall {}.abi_encode(),
            Ok(paused.abi_encode()),
        );
    }

    fn mock_feed(&self, answer: i128, updated_at: u64) {
        self.mock_feed_raw(I256::try_from(answer).unwrap(), updated_at);
    }

    fn mock_feed_raw(&self, answer: I256, updated_at: u64) {
        let ret = (
            U80::from(ROUND),
            answer,
            U256::from(updated_at),
            U256::from(updated_at),
            U80::from(ROUND),
        );
        self.vm.mock_static_call(
            FEED,
            latestRoundDataCall {}.abi_encode(),
            Ok(ret.abi_encode_params()),
        );
    }

    /// A pool whose time-weighted mean tick over TWAP_WINDOW is `mean_tick` and
    /// whose harmonic-mean liquidity over the window is `liquidity`
    /// (secondsPerLiquidityCumulativeX128 grows by window * 2^128 / L).
    fn mock_observe(&self, mean_tick: i64, liquidity: u128) {
        let then = I56::try_from(1_000_000i64).unwrap();
        let now = I56::try_from(1_000_000i64 + mean_tick * i64::from(TWAP_WINDOW)).unwrap();
        let spl_then = U160::from(7_777_777u64);
        let spl_delta = (U256::from(TWAP_WINDOW) << 128usize) / U256::from(liquidity);
        let spl_now = spl_then + U160::from(spl_delta);
        self.mock_observe_raw(vec![then, now], vec![spl_then, spl_now]);
    }

    fn mock_observe_raw(&self, ticks: Vec<I56>, spl: Vec<U160>) {
        self.vm.mock_static_call(
            POOL,
            observeCall {
                secondsAgos: vec![TWAP_WINDOW, 0],
            }
            .abi_encode(),
            // return values are encoded as a sequence (no leading offset), like a Solidity return
            Ok((ticks, spl).abi_encode_params()),
        );
    }

    fn mock_observe_reverts(&self) {
        self.vm.mock_static_call(
            POOL,
            observeCall {
                secondsAgos: vec![TWAP_WINDOW, 0],
            }
            .abi_encode(),
            Err(b"OLD".to_vec()),
        );
    }

    fn deploy(&self) -> AfterHours {
        let mut c = AfterHours::from(&self.vm);
        c.initialize(
            FEED,
            POOL,
            STOCK,
            LIVE_MAX_AGE,
            TWAP_WINDOW,
            MAX_DEV_BPS,
            MIN_LIQUIDITY,
            MAX_ANCHOR_AGE,
        )
        .expect("initialize");
        c
    }

    fn try_deploy(&self) -> Result<(), AfterHoursError> {
        let mut c = AfterHours::from(&self.vm);
        c.initialize(
            FEED,
            POOL,
            STOCK,
            LIVE_MAX_AGE,
            TWAP_WINDOW,
            MAX_DEV_BPS,
            MIN_LIQUIDITY,
            MAX_ANCHOR_AGE,
        )
    }
}

fn u(s: &str) -> U256 {
    U256::from_str_radix(s, 10).unwrap()
}

/// `uint8` return data. `u8` has no `SolValue` impl (it would clash with `bytes`).
fn enc_u8(v: u8) -> Vec<u8> {
    <sol_data::Uint<8> as SolType>::abi_encode(&v)
}

fn config_reason(err: AfterHoursError) -> u8 {
    match err {
        AfterHoursError::InvalidConfig(InvalidConfig { reason }) => reason,
        other => panic!("expected InvalidConfig, got {other:?}"),
    }
}

const MORPHO_SCALE: &str = "10000000000000000"; // 10^(36 + 6 - 18 - 8)

// ---- initialize ----------------------------------------------------------------

#[test]
fn initialize_reads_layout_and_decimals_from_the_contracts() {
    let w = World::new();
    let c = w.deploy();
    let (
        initialized,
        initializer,
        feed,
        pool,
        stock,
        quote,
        stock_is_token0,
        fd,
        sd,
        qd,
        age,
        window,
        dev,
        min_liq,
        anchor,
    ) = c.config();
    assert!(initialized);
    assert_eq!(initializer, DEPLOYER, "records tx.origin of initialize");
    assert_eq!((feed, pool, stock, quote), (FEED, POOL, STOCK, USDG));
    assert!(!stock_is_token0, "AAPL is token1 in the real pool");
    assert_eq!((fd, sd, qd), (8, 18, 6));
    assert_eq!(
        (age, window, dev, min_liq, anchor),
        (
            LIVE_MAX_AGE,
            TWAP_WINDOW,
            MAX_DEV_BPS,
            MIN_LIQUIDITY,
            MAX_ANCHOR_AGE
        )
    );
    assert_eq!(c.decimals(), 8);
    assert_eq!(c.version(), U256::from(1u64));
    assert_eq!(c.description().unwrap(), "AAPL / USD (AfterHours)");
}

#[test]
fn initialize_accepts_the_stock_as_token0() {
    let w = World::new();
    w.mock_tokens(true);
    let c = w.deploy();
    let (_, _, _, _, _, quote, stock_is_token0, ..) = c.config();
    assert!(stock_is_token0);
    assert_eq!(quote, USDG);
}

#[test]
fn initialize_rejects_bad_parameters() {
    let w = World::new();
    let mut c = AfterHours::from(&w.vm);
    let cases: [(u64, u32, u64, u128, u64, u8); 9] = [
        (
            0,
            TWAP_WINDOW,
            MAX_DEV_BPS,
            MIN_LIQUIDITY,
            MAX_ANCHOR_AGE,
            CONFIG_ZERO_LIVE_MAX_AGE,
        ),
        (
            LIVE_MAX_AGE,
            0,
            MAX_DEV_BPS,
            MIN_LIQUIDITY,
            MAX_ANCHOR_AGE,
            CONFIG_ZERO_TWAP_WINDOW,
        ),
        (
            LIVE_MAX_AGE,
            86_401,
            MAX_DEV_BPS,
            MIN_LIQUIDITY,
            MAX_ANCHOR_AGE,
            CONFIG_WINDOW_TOO_LONG,
        ),
        (
            LIVE_MAX_AGE,
            TWAP_WINDOW,
            0,
            MIN_LIQUIDITY,
            MAX_ANCHOR_AGE,
            CONFIG_BAD_DEVIATION,
        ),
        (
            LIVE_MAX_AGE,
            TWAP_WINDOW,
            10_000,
            MIN_LIQUIDITY,
            MAX_ANCHOR_AGE,
            CONFIG_BAD_DEVIATION,
        ),
        (
            LIVE_MAX_AGE,
            TWAP_WINDOW,
            20_000,
            MIN_LIQUIDITY,
            MAX_ANCHOR_AGE,
            CONFIG_BAD_DEVIATION,
        ),
        (
            LIVE_MAX_AGE,
            TWAP_WINDOW,
            MAX_DEV_BPS,
            0,
            MAX_ANCHOR_AGE,
            CONFIG_ZERO_MIN_LIQUIDITY,
        ),
        (
            LIVE_MAX_AGE,
            TWAP_WINDOW,
            MAX_DEV_BPS,
            MIN_LIQUIDITY,
            LIVE_MAX_AGE,
            CONFIG_ANCHOR_AGE,
        ),
        (
            LIVE_MAX_AGE,
            TWAP_WINDOW,
            MAX_DEV_BPS,
            MIN_LIQUIDITY,
            LIVE_MAX_AGE - 1,
            CONFIG_ANCHOR_AGE,
        ),
    ];
    for (age, window, dev, min_liq, anchor, expected) in cases {
        let err = c
            .initialize(FEED, POOL, STOCK, age, window, dev, min_liq, anchor)
            .expect_err("must reject");
        assert_eq!(
            config_reason(err),
            expected,
            "case ({age},{window},{dev},{min_liq},{anchor})"
        );
    }
    let (initialized, ..) = c.config();
    assert!(
        !initialized,
        "a rejected initialize leaves the contract untouched"
    );
}

#[test]
fn initialize_rejects_a_pool_without_the_stock() {
    let w = World::new();
    let other = Address::repeat_byte(0x99);
    w.vm.mock_static_call(POOL, token0Call {}.abi_encode(), Ok(other.abi_encode()));
    w.vm.mock_static_call(POOL, token1Call {}.abi_encode(), Ok(USDG.abi_encode()));
    assert_eq!(
        config_reason(w.try_deploy().expect_err("stock is neither token")),
        CONFIG_STOCK_NOT_IN_POOL
    );
}

#[test]
fn initialize_rejects_a_pool_that_cannot_be_observed() {
    // A v2 pair has token0/token1 but no observe(); a fresh v3 pool reverts "OLD".
    let w = World::new();
    w.mock_observe_reverts();
    assert_eq!(
        config_reason(w.try_deploy().expect_err("observe reverted")),
        CONFIG_POOL_NOT_OBSERVABLE
    );
    // ...and one that answers with the wrong shape.
    let w = World::new();
    w.mock_observe_raw(vec![I56::ZERO], vec![U160::ZERO]);
    assert_eq!(
        config_reason(w.try_deploy().expect_err("one observation")),
        CONFIG_POOL_NOT_OBSERVABLE
    );
}

#[test]
fn initialize_rejects_a_scale_that_would_underflow() {
    let w = World::new();
    // 36 + 6 < 36 + 8: Morpho's scale would need a negative exponent.
    w.vm.mock_static_call(STOCK, decimalsCall {}.abi_encode(), Ok(enc_u8(36)));
    assert_eq!(
        config_reason(w.try_deploy().expect_err("scale underflow")),
        CONFIG_SCALE_UNDERFLOW
    );
}

#[test]
fn initialize_rejects_absurd_decimals() {
    let w = World::new();
    w.vm.mock_static_call(USDG, decimalsCall {}.abi_encode(), Ok(enc_u8(37)));
    assert_eq!(
        config_reason(w.try_deploy().expect_err("37 decimals")),
        CONFIG_DECIMALS_TOO_LARGE
    );
}

#[test]
fn initialize_requires_the_pause_flag_to_be_readable() {
    let w = World::new();
    w.vm.mock_static_call(STOCK, oraclePausedCall {}.abi_encode(), Err(Vec::new()));
    let err = w.try_deploy().expect_err("token without oraclePaused()");
    assert!(matches!(err, AfterHoursError::CallFailed(CallFailed { target }) if target == STOCK));
}

#[test]
fn initialize_runs_only_once() {
    let w = World::new();
    let mut c = w.deploy();
    let err = c
        .initialize(
            FEED,
            POOL,
            STOCK,
            LIVE_MAX_AGE,
            TWAP_WINDOW,
            MAX_DEV_BPS,
            MIN_LIQUIDITY,
            MAX_ANCHOR_AGE,
        )
        .expect_err("second initialize");
    assert!(matches!(err, AfterHoursError::AlreadyInitialized(_)));
}

#[test]
fn reads_refuse_before_initialize() {
    let w = World::new();
    let c = AfterHours::from(&w.vm);
    w.mock_feed(FRIDAY_ANSWER as i128, NOW - 60);
    assert!(matches!(
        c.latest_round_data().expect_err("uninitialized"),
        AfterHoursError::NotInitialized(_)
    ));
    assert!(matches!(
        c.price().expect_err("uninitialized"),
        AfterHoursError::NotInitialized(_)
    ));
    assert!(matches!(
        c.state().expect_err("uninitialized"),
        AfterHoursError::NotInitialized(_)
    ));
    assert!(matches!(
        c.latest_answer().expect_err("uninitialized"),
        AfterHoursError::NotInitialized(_)
    ));
}

// ---- LIVE_FEED -------------------------------------------------------------------

#[test]
fn fresh_feed_passes_through_verbatim() {
    let w = World::new();
    let c = w.deploy();
    w.mock_feed(FRIDAY_ANSWER as i128, NOW - 120);

    let (round, answer, started, updated, answered_in) = c.latest_round_data().unwrap();
    assert_eq!(round, U80::from(ROUND));
    assert_eq!(answer, I256::try_from(FRIDAY_ANSWER).unwrap());
    assert_eq!(started, U256::from(NOW - 120));
    assert_eq!(updated, U256::from(NOW - 120));
    assert_eq!(answered_in, U80::from(ROUND));

    assert_eq!(
        c.latest_answer().unwrap(),
        I256::try_from(FRIDAY_ANSWER).unwrap()
    );
    assert_eq!(c.latest_timestamp().unwrap(), U256::from(NOW - 120));
    assert_eq!(c.latest_round().unwrap(), U256::from(ROUND));

    // Morpho: answer * 10^(36 + 6 - 18 - 8) = answer * 1e16
    assert_eq!(
        c.price().unwrap(),
        U256::from(FRIDAY_ANSWER) * u(MORPHO_SCALE)
    );

    let (session, reason, ans, feed_answer, feed_updated, twap, liq, clamped) = c.state().unwrap();
    assert_eq!((session, reason), (SESSION_LIVE_FEED, REASON_NONE));
    assert_eq!(ans, U256::from(FRIDAY_ANSWER));
    assert_eq!(feed_answer, U256::from(FRIDAY_ANSWER));
    assert_eq!(feed_updated, U256::from(NOW - 120));
    assert_eq!((twap, liq, clamped), (U256::ZERO, 0, false));
}

#[test]
fn feed_exactly_at_the_age_limit_is_still_live() {
    let w = World::new();
    let c = w.deploy();
    w.mock_feed(FRIDAY_ANSWER as i128, NOW - LIVE_MAX_AGE);
    let (session, ..) = c.state().unwrap();
    assert_eq!(session, SESSION_LIVE_FEED);
}

#[test]
fn get_round_data_forwards_history_and_mirrors_the_current_round() {
    let w = World::new();
    let c = w.deploy();
    w.mock_feed(FRIDAY_ANSWER as i128, NOW - 120);
    let old = (
        U80::from(ROUND - 1),
        I256::try_from(31_000_000_000u64).unwrap(),
        U256::from(NOW - 9000),
        U256::from(NOW - 9000),
        U80::from(ROUND - 1),
    );
    w.vm.mock_static_call(
        FEED,
        getRoundDataCall {
            roundId: U80::from(ROUND - 1),
        }
        .abi_encode(),
        Ok(old.abi_encode_params()),
    );
    assert_eq!(c.get_round_data(U80::from(ROUND - 1)).unwrap(), old);
    assert_eq!(
        c.get_round_data(U80::from(ROUND)).unwrap(),
        c.latest_round_data().unwrap()
    );
}

// ---- ONCHAIN_TWAP ----------------------------------------------------------------

#[test]
fn stale_feed_switches_to_the_pool_twap() {
    let w = World::new();
    let c = w.deploy();
    w.mock_feed(FRIDAY_ANSWER as i128, NOW - 40 * 3600);
    w.mock_observe(AAPL_TICK, MIN_LIQUIDITY);

    let (round, answer, started, updated, answered_in) = c.latest_round_data().unwrap();
    assert_eq!(round, U80::from(ROUND), "round ids stay the feed's");
    assert_eq!(answer, I256::try_from(AAPL_TWAP).unwrap());
    assert_eq!(
        started,
        U256::from(NOW - 40 * 3600),
        "startedAt = last exchange print"
    );
    assert_eq!(
        updated,
        U256::from(NOW),
        "updatedAt = now, the TWAP is fresh"
    );
    assert_eq!(answered_in, U80::from(ROUND));

    assert_eq!(
        c.latest_answer().unwrap(),
        I256::try_from(AAPL_TWAP).unwrap()
    );
    assert_eq!(c.latest_timestamp().unwrap(), U256::from(NOW));
    assert_eq!(c.latest_round().unwrap(), U256::from(ROUND));
    assert_eq!(
        c.get_round_data(U80::from(ROUND)).unwrap(),
        c.latest_round_data().unwrap()
    );
    assert_eq!(c.price().unwrap(), U256::from(AAPL_TWAP) * u(MORPHO_SCALE));

    let (session, reason, ans, feed_answer, _, twap, liq, clamped) = c.state().unwrap();
    assert_eq!((session, reason), (SESSION_ONCHAIN_TWAP, REASON_NONE));
    assert_eq!(ans, U256::from(AAPL_TWAP));
    assert_eq!(twap, U256::from(AAPL_TWAP));
    assert_eq!(feed_answer, U256::from(FRIDAY_ANSWER));
    assert_eq!(
        liq, MIN_LIQUIDITY,
        "harmonic-mean liquidity over the window"
    );
    assert!(!clamped);
}

#[test]
fn stock_as_token0_prices_from_a_negative_tick() {
    let w = World::new();
    w.mock_tokens(true);
    let c = w.deploy();
    w.mock_feed(FRIDAY_ANSWER as i128, NOW - 40 * 3600);
    // Same pool price, mirrored: quote per stock = 1.0001^(-218301) in raw units.
    w.mock_observe(-AAPL_TICK, MIN_LIQUIDITY);
    let (session, _, ans, ..) = c.state().unwrap();
    assert_eq!(session, SESSION_ONCHAIN_TWAP);
    assert_eq!(ans, U256::from(AAPL_TWAP));
}

#[test]
fn one_second_past_the_age_limit_uses_the_pool() {
    let w = World::new();
    let c = w.deploy();
    w.mock_feed(FRIDAY_ANSWER as i128, NOW - LIVE_MAX_AGE - 1);
    let (session, ..) = c.state().unwrap();
    assert_eq!(session, SESSION_ONCHAIN_TWAP);
}

#[test]
fn twap_far_below_the_last_print_is_clamped_to_the_band() {
    let w = World::new();
    let c = w.deploy();
    w.mock_feed(FRIDAY_ANSWER as i128, NOW - 40 * 3600);
    // +2000 ticks = the pool says roughly -18%; the band allows -10%.
    w.mock_observe(AAPL_TICK + 2000, MIN_LIQUIDITY);

    let lower = U256::from(FRIDAY_ANSWER) * U256::from(9000u64) / U256::from(10_000u64);
    let (_, answer, ..) = c.latest_round_data().unwrap();
    assert_eq!(answer, I256::from_raw(lower));
    let (session, _, ans, _, _, twap, _, clamped) = c.state().unwrap();
    assert_eq!(session, SESSION_ONCHAIN_TWAP);
    assert_eq!(ans, lower);
    assert!(twap < lower, "raw twap is reported unclamped");
    assert!(clamped);
    assert_eq!(c.price().unwrap(), lower * u(MORPHO_SCALE));
}

#[test]
fn twap_far_above_the_last_print_is_clamped_to_the_band() {
    let w = World::new();
    let c = w.deploy();
    w.mock_feed(FRIDAY_ANSWER as i128, NOW - 40 * 3600);
    // -2000 ticks = the pool says roughly +22%; the band allows +10%.
    w.mock_observe(AAPL_TICK - 2000, MIN_LIQUIDITY);

    let upper = U256::from(FRIDAY_ANSWER) * U256::from(11_000u64) / U256::from(10_000u64);
    let (_, answer, ..) = c.latest_round_data().unwrap();
    assert_eq!(answer, I256::from_raw(upper));
    let (.., twap, _, clamped) = c.state().unwrap();
    assert!(twap > upper);
    assert!(clamped);
}

#[test]
fn twap_inside_the_band_is_not_clamped() {
    let w = World::new();
    let c = w.deploy();
    w.mock_feed(FRIDAY_ANSWER as i128, NOW - 40 * 3600);
    // +500 ticks = about -5.3% against the Friday print, inside the 10% band.
    w.mock_observe(AAPL_TICK + 500, MIN_LIQUIDITY);
    let (_, _, ans, _, _, twap, _, clamped) = c.state().unwrap();
    assert_eq!(ans, twap);
    assert!(!clamped);
    let lower = U256::from(FRIDAY_ANSWER) * U256::from(9000u64) / U256::from(10_000u64);
    assert!(ans > lower && ans < U256::from(FRIDAY_ANSWER));
}

// ---- refusals ----------------------------------------------------------------------

#[test]
fn thin_pool_refuses_to_price() {
    let w = World::new();
    let c = w.deploy();
    w.mock_feed(FRIDAY_ANSWER as i128, NOW - 40 * 3600);
    w.mock_observe(AAPL_TICK, MIN_LIQUIDITY - 1);

    let err = c.latest_round_data().expect_err("thin pool");
    assert!(matches!(
        err,
        AfterHoursError::NoData(NoData {
            reason: REASON_POOL_TOO_THIN
        })
    ));
    let err = c.price().expect_err("thin pool");
    assert!(matches!(
        err,
        AfterHoursError::NoData(NoData {
            reason: REASON_POOL_TOO_THIN
        })
    ));
    let (session, reason, ans, _, _, twap, liq, _) = c.state().unwrap();
    assert_eq!((session, reason), (SESSION_NO_DATA, REASON_POOL_TOO_THIN));
    assert_eq!((ans, twap), (U256::ZERO, U256::ZERO));
    assert_eq!(liq, MIN_LIQUIDITY - 1);
}

#[test]
fn liquidity_is_the_window_average_not_the_spot_value() {
    // Only observe() is read while closed: a position added in the last block
    // does not enter the check. Encode a pool that spent the window thin.
    let w = World::new();
    let c = w.deploy();
    let calls_before = w.vm.call_log().len();
    w.mock_feed(FRIDAY_ANSWER as i128, NOW - 40 * 3600);
    w.mock_observe(AAPL_TICK, MIN_LIQUIDITY / 2);
    let (session, reason, _, _, _, _, liq, _) = c.state().unwrap();
    assert_eq!((session, reason), (SESSION_NO_DATA, REASON_POOL_TOO_THIN));
    assert_eq!(liq, MIN_LIQUIDITY / 2);
    let reads = w.vm.call_log().split_off(calls_before);
    assert!(
        reads
            .iter()
            .all(|(to, data)| *to != POOL || data.starts_with(&observeCall::SELECTOR)),
        "after initialize the pool is only ever asked observe(): {reads:?}"
    );
    assert!(reads.iter().any(|(to, _)| *to == POOL), "the pool was read");
}

#[test]
fn pool_without_history_refuses_to_price() {
    let w = World::new();
    let c = w.deploy();
    w.mock_feed(FRIDAY_ANSWER as i128, NOW - 40 * 3600);
    w.mock_observe_reverts();

    let err = c.latest_round_data().expect_err("observe reverted");
    assert!(matches!(
        err,
        AfterHoursError::NoData(NoData {
            reason: REASON_TWAP_UNAVAILABLE
        })
    ));
    let (session, reason, ..) = c.state().unwrap();
    assert_eq!(
        (session, reason),
        (SESSION_NO_DATA, REASON_TWAP_UNAVAILABLE)
    );
}

#[test]
fn malformed_observations_refuse_to_price() {
    let w = World::new();
    let c = w.deploy();
    w.mock_feed(FRIDAY_ANSWER as i128, NOW - 40 * 3600);
    // one observation instead of two
    w.mock_observe_raw(vec![I56::ZERO], vec![U160::ZERO]);
    let (session, reason, ..) = c.state().unwrap();
    assert_eq!(
        (session, reason),
        (SESSION_NO_DATA, REASON_TWAP_UNAVAILABLE)
    );
    // no liquidity-seconds recorded across the window
    w.mock_observe_raw(
        vec![I56::ZERO, I56::ZERO],
        vec![U160::from(5u64), U160::from(5u64)],
    );
    let (session, reason, ..) = c.state().unwrap();
    assert_eq!(
        (session, reason),
        (SESSION_NO_DATA, REASON_TWAP_UNAVAILABLE)
    );
    // a mean tick outside Uniswap's range
    let spl_delta = U160::from((U256::from(TWAP_WINDOW) << 128usize) / U256::from(MIN_LIQUIDITY));
    w.mock_observe_raw(
        vec![
            I56::ZERO,
            I56::try_from(1_000_000i64 * i64::from(TWAP_WINDOW)).unwrap(),
        ],
        vec![U160::ZERO, spl_delta],
    );
    let (session, reason, ..) = c.state().unwrap();
    assert_eq!(
        (session, reason),
        (SESSION_NO_DATA, REASON_TWAP_UNAVAILABLE)
    );
}

#[test]
fn issuer_pause_refuses_even_a_fresh_feed() {
    let w = World::new();
    let c = w.deploy();
    w.mock_paused(true);
    w.mock_feed(FRIDAY_ANSWER as i128, NOW - 60);

    assert!(matches!(
        c.latest_round_data().expect_err("paused"),
        AfterHoursError::IssuerPaused(_)
    ));
    assert!(matches!(
        c.price().expect_err("paused"),
        AfterHoursError::IssuerPaused(_)
    ));
    assert!(matches!(
        c.latest_answer().expect_err("paused"),
        AfterHoursError::IssuerPaused(_)
    ));
    let (session, reason, ans, feed_answer, ..) = c.state().unwrap();
    assert_eq!((session, reason), (SESSION_PAUSED, REASON_NONE));
    assert_eq!(ans, U256::ZERO);
    assert_eq!(
        feed_answer,
        U256::from(FRIDAY_ANSWER),
        "state still shows the feed"
    );
}

#[test]
fn issuer_pause_wins_over_a_stale_or_broken_feed() {
    let w = World::new();
    let c = w.deploy();
    w.mock_paused(true);
    w.mock_feed(FRIDAY_ANSWER as i128, NOW - 40 * 3600);
    let (session, ..) = c.state().unwrap();
    assert_eq!(session, SESSION_PAUSED, "never falls through to the TWAP");
    w.mock_feed(0, NOW - 60);
    let (session, _, _, feed_answer, ..) = c.state().unwrap();
    assert_eq!(session, SESSION_PAUSED);
    assert_eq!(feed_answer, U256::ZERO);
}

#[test]
fn broken_feed_rounds_are_no_data() {
    let w = World::new();
    let c = w.deploy();
    for (answer, updated_at) in [
        (0i128, NOW - 60),
        (-1, NOW - 60),
        (FRIDAY_ANSWER as i128, 0),
        (FRIDAY_ANSWER as i128, NOW + 1),
    ] {
        w.mock_feed(answer, updated_at);
        let err = c.latest_round_data().expect_err("invalid round");
        assert!(
            matches!(
                err,
                AfterHoursError::NoData(NoData {
                    reason: REASON_FEED_INVALID
                })
            ),
            "answer {answer} updated_at {updated_at}"
        );
        let (session, reason, ..) = c.state().unwrap();
        assert_eq!((session, reason), (SESSION_NO_DATA, REASON_FEED_INVALID));
    }
}

#[test]
fn a_print_older_than_any_closure_is_not_an_anchor() {
    let w = World::new();
    let c = w.deploy();
    // Just inside: a Tuesday-after-Labor-Day read still anchors to Friday.
    w.mock_feed(FRIDAY_ANSWER as i128, NOW - MAX_ANCHOR_AGE);
    let (session, ..) = c.state().unwrap();
    assert_eq!(session, SESSION_ONCHAIN_TWAP);
    // One second past: the feed is gone or the stock is halted; refuse.
    w.mock_feed(FRIDAY_ANSWER as i128, NOW - MAX_ANCHOR_AGE - 1);
    let (session, reason, ans, ..) = c.state().unwrap();
    assert_eq!((session, reason), (SESSION_NO_DATA, REASON_ANCHOR_STALE));
    assert_eq!(ans, U256::ZERO);
    assert!(matches!(
        c.latest_round_data().expect_err("anchor stale"),
        AfterHoursError::NoData(NoData {
            reason: REASON_ANCHOR_STALE
        })
    ));
    assert!(matches!(
        c.latest_timestamp().expect_err("anchor stale"),
        AfterHoursError::NoData(NoData {
            reason: REASON_ANCHOR_STALE
        })
    ));
}

#[test]
fn absurd_feed_answers_fail_closed_instead_of_overflowing() {
    let w = World::new();
    let c = w.deploy();
    // Largest positive int256. LIVE: price() would overflow the 1e16 Morpho scale.
    w.mock_feed_raw(I256::MAX, NOW - 60);
    assert!(matches!(
        c.price().expect_err("overflow"),
        AfterHoursError::NoData(NoData {
            reason: REASON_FEED_INVALID
        })
    ));
    let (_, answer, ..) = c.latest_round_data().unwrap();
    assert_eq!(answer, I256::MAX, "the feed itself still passes through");
    // Stale: the band arithmetic would overflow, so the oracle refuses.
    w.mock_feed_raw(I256::MAX, NOW - 40 * 3600);
    let (session, reason, ..) = c.state().unwrap();
    assert_eq!((session, reason), (SESSION_NO_DATA, REASON_FEED_INVALID));
    assert!(matches!(
        c.latest_round_data().expect_err("overflow"),
        AfterHoursError::NoData(NoData {
            reason: REASON_FEED_INVALID
        })
    ));
}

#[test]
fn initialize_requires_a_readable_feed_round() {
    let w = World::new();
    w.vm
        .mock_static_call(FEED, latestRoundDataCall {}.abi_encode(), Err(Vec::new()));
    let err = w.try_deploy().expect_err("feed without latestRoundData()");
    assert!(matches!(err, AfterHoursError::CallFailed(CallFailed { target }) if target == FEED));
}

#[test]
fn a_second_of_empty_liquidity_inside_the_window_refuses() {
    // Uniswap accumulates seconds * 2^128 / max(L, 1). One second at zero
    // in-range liquidity adds 2^128, which drags the harmonic mean to ~1800
    // no matter how deep the other 1799 seconds were. Fail closed for the window.
    let w = World::new();
    let c = w.deploy();
    w.mock_feed(FRIDAY_ANSWER as i128, NOW - 40 * 3600);
    let deep = U256::from(1_600_000_000_000_000_000u128); // 1.6e18, today's pool
    let delta = (U256::from(TWAP_WINDOW - 1) << 128usize) / deep + (U256::from(1u64) << 128usize);
    let then = I56::try_from(1_000_000i64).unwrap();
    let now = I56::try_from(1_000_000i64 + AAPL_TICK * i64::from(TWAP_WINDOW)).unwrap();
    w.mock_observe_raw(vec![then, now], vec![U160::ZERO, U160::from(delta)]);
    let (session, reason, _, _, _, _, liq, _) = c.state().unwrap();
    assert_eq!((session, reason), (SESSION_NO_DATA, REASON_POOL_TOO_THIN));
    assert!(liq < 2000, "harmonic mean collapses to about the window length: {liq}");
}

#[test]
fn liquidity_accumulator_wraps_around_uint160() {
    // The pool's secondsPerLiquidityCumulativeX128 is an unchecked uint160
    // accumulator; a window that straddles the wrap must still read correctly.
    let w = World::new();
    let c = w.deploy();
    w.mock_feed(FRIDAY_ANSWER as i128, NOW - 40 * 3600);
    let delta = U160::from((U256::from(TWAP_WINDOW) << 128usize) / U256::from(MIN_LIQUIDITY));
    let spl_then = U160::MAX - U160::from(5u64);
    let spl_now = spl_then.wrapping_add(delta);
    assert!(spl_now < spl_then, "the test straddles the wrap");
    let then = I56::try_from(1_000_000i64).unwrap();
    let now = I56::try_from(1_000_000i64 + AAPL_TICK * i64::from(TWAP_WINDOW)).unwrap();
    w.mock_observe_raw(vec![then, now], vec![spl_then, spl_now]);
    let (session, _, ans, _, _, _, liq, _) = c.state().unwrap();
    assert_eq!(session, SESSION_ONCHAIN_TWAP);
    assert_eq!(liq, MIN_LIQUIDITY);
    assert_eq!(ans, U256::from(AAPL_TWAP));
}

#[test]
fn absurdly_deep_liquidity_saturates_instead_of_overflowing() {
    let w = World::new();
    let c = w.deploy();
    w.mock_feed(FRIDAY_ANSWER as i128, NOW - 40 * 3600);
    let then = I56::try_from(1_000_000i64).unwrap();
    let now = I56::try_from(1_000_000i64 + AAPL_TICK * i64::from(TWAP_WINDOW)).unwrap();
    // delta of 1: window * 2^128 liquidity, far beyond u128
    w.mock_observe_raw(vec![then, now], vec![U160::from(9u64), U160::from(10u64)]);
    let (session, _, _, _, _, _, liq, _) = c.state().unwrap();
    assert_eq!(session, SESSION_ONCHAIN_TWAP);
    assert_eq!(liq, u128::MAX);
}

#[test]
fn get_round_data_serves_history_while_refusing_to_price() {
    let w = World::new();
    let c = w.deploy();
    w.mock_paused(true);
    w.mock_feed(FRIDAY_ANSWER as i128, NOW - 120);
    let old = (
        U80::from(ROUND - 1),
        I256::try_from(31_000_000_000u64).unwrap(),
        U256::from(NOW - 9000),
        U256::from(NOW - 9000),
        U80::from(ROUND - 1),
    );
    w.vm.mock_static_call(
        FEED,
        getRoundDataCall {
            roundId: U80::from(ROUND - 1),
        }
        .abi_encode(),
        Ok(old.abi_encode_params()),
    );
    assert_eq!(c.get_round_data(U80::from(ROUND - 1)).unwrap(), old);
    assert!(matches!(
        c.get_round_data(U80::from(ROUND)).expect_err("current round is refused while paused"),
        AfterHoursError::IssuerPaused(_)
    ));
    // a round the feed itself rejects is a failed read
    w.vm.mock_static_call(
        FEED,
        getRoundDataCall {
            roundId: U80::from(ROUND - 2),
        }
        .abi_encode(),
        Err(b"No data present".to_vec()),
    );
    assert!(matches!(
        c.get_round_data(U80::from(ROUND - 2)).expect_err("feed rejected the round"),
        AfterHoursError::CallFailed(CallFailed { target }) if target == FEED
    ));
}

#[test]
fn unreadable_feed_is_a_call_failure_not_a_price() {
    let w = World::new();
    let c = w.deploy();
    w.vm.mock_static_call(FEED, latestRoundDataCall {}.abi_encode(), Err(Vec::new()));
    assert!(matches!(
        c.state().expect_err("feed reverted"),
        AfterHoursError::CallFailed(CallFailed { target }) if target == FEED
    ));
}
