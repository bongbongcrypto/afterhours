//! Unit tests on the Stylus TestVM. Every external read is mocked with the
//! exact calldata the contract sends, encoded from the same Solidity
//! signatures, so a mismatch shows up as an unmocked call instead of a
//! silently green test.

use super::*;
use alloy_primitives::{aliases::I56, aliases::U160, aliases::U8, Address, I256, U256};
use alloy_sol_types::{sol, SolCall, SolValue};
use stylus_sdk::testing::*;

sol! {
    function decimals() external view returns (uint8);
    function description() external view returns (string);
    function latestRoundData() external view returns (uint80, int256, uint256, uint256, uint80);
    function token0() external view returns (address);
    function token1() external view returns (address);
    function liquidity() external view returns (uint128);
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

const NOW: u64 = 1_800_000_000;
/// $332.52 with 8 decimals: AAPL's last Friday print on 2026-09-11.
const FRIDAY_ANSWER: u64 = 33_252_000_000;
/// Mean tick of the AAPL/USDG 0.05% pool worth $330.96497305 (reference vector).
const AAPL_TICK: i64 = 218_301;
const AAPL_TWAP: u64 = 33_096_497_305;

struct World {
    vm: TestVM,
}

impl World {
    fn new() -> Self {
        let vm = TestVM::new();
        vm.set_block_timestamp(NOW);
        vm.set_tx_origin(DEPLOYER);
        let w = World { vm };
        w.mock_tokens(false);
        w.mock_paused(false);
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
            .mock_static_call(FEED, decimalsCall {}.abi_encode(), Ok(U8::from(8u8).abi_encode()));
        self.vm
            .mock_static_call(STOCK, decimalsCall {}.abi_encode(), Ok(U8::from(18u8).abi_encode()));
        self.vm
            .mock_static_call(USDG, decimalsCall {}.abi_encode(), Ok(U8::from(6u8).abi_encode()));
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
        let ret = (
            U80::from(645u64),
            I256::try_from(answer).unwrap(),
            U256::from(updated_at),
            U256::from(updated_at),
            U80::from(645u64),
        );
        self.vm.mock_static_call(
            FEED,
            latestRoundDataCall {}.abi_encode(),
            Ok(ret.abi_encode()),
        );
    }

    fn mock_liquidity(&self, liquidity: u128) {
        self.vm.mock_static_call(
            POOL,
            liquidityCall {}.abi_encode(),
            Ok(liquidity.abi_encode()),
        );
    }

    /// A pool whose time-weighted mean tick over TWAP_WINDOW is `mean_tick`.
    fn mock_twap_tick(&self, mean_tick: i64) {
        let then = I56::try_from(1_000_000i64).unwrap();
        let now = I56::try_from(1_000_000i64 + mean_tick * i64::from(TWAP_WINDOW)).unwrap();
        let ret = (vec![then, now], vec![U160::ZERO, U160::ZERO]);
        self.vm.mock_static_call(
            POOL,
            observeCall {
                secondsAgos: vec![TWAP_WINDOW, 0],
            }
            .abi_encode(),
            Ok(ret.abi_encode()),
        );
    }

    fn mock_twap_reverts(&self) {
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
        )
        .expect("initialize");
        c
    }
}

fn u(s: &str) -> U256 {
    U256::from_str_radix(s, 10).unwrap()
}

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
    ) = c.config();
    assert!(initialized);
    assert_eq!(initializer, DEPLOYER, "records tx.origin of initialize");
    assert_eq!((feed, pool, stock, quote), (FEED, POOL, STOCK, USDG));
    assert!(!stock_is_token0, "AAPL is token1 in the real pool");
    assert_eq!((fd, sd, qd), (8, 18, 6));
    assert_eq!(
        (age, window, dev, min_liq),
        (LIVE_MAX_AGE, TWAP_WINDOW, MAX_DEV_BPS, MIN_LIQUIDITY)
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
    let cases: [(u64, u32, u64, u128, u8); 6] = [
        (
            0,
            TWAP_WINDOW,
            MAX_DEV_BPS,
            MIN_LIQUIDITY,
            CONFIG_ZERO_LIVE_MAX_AGE,
        ),
        (
            LIVE_MAX_AGE,
            0,
            MAX_DEV_BPS,
            MIN_LIQUIDITY,
            CONFIG_ZERO_TWAP_WINDOW,
        ),
        (
            LIVE_MAX_AGE,
            TWAP_WINDOW,
            0,
            MIN_LIQUIDITY,
            CONFIG_BAD_DEVIATION,
        ),
        (
            LIVE_MAX_AGE,
            TWAP_WINDOW,
            10_000,
            MIN_LIQUIDITY,
            CONFIG_BAD_DEVIATION,
        ),
        (
            LIVE_MAX_AGE,
            TWAP_WINDOW,
            20_000,
            MIN_LIQUIDITY,
            CONFIG_BAD_DEVIATION,
        ),
        (
            LIVE_MAX_AGE,
            TWAP_WINDOW,
            MAX_DEV_BPS,
            0,
            CONFIG_ZERO_MIN_LIQUIDITY,
        ),
    ];
    for (age, window, dev, min_liq, expected) in cases {
        let err = c
            .initialize(FEED, POOL, STOCK, age, window, dev, min_liq)
            .expect_err("must reject");
        assert!(
            matches!(err, AfterHoursError::InvalidConfig(InvalidConfig { reason }) if reason == expected),
            "case ({age},{window},{dev},{min_liq}) expected reason {expected}"
        );
    }
}

#[test]
fn initialize_rejects_a_pool_without_the_stock() {
    let w = World::new();
    let other = Address::repeat_byte(0x99);
    w.vm.mock_static_call(POOL, token0Call {}.abi_encode(), Ok(other.abi_encode()));
    w.vm.mock_static_call(POOL, token1Call {}.abi_encode(), Ok(USDG.abi_encode()));
    let mut c = AfterHours::from(&w.vm);
    let err = c
        .initialize(
            FEED,
            POOL,
            STOCK,
            LIVE_MAX_AGE,
            TWAP_WINDOW,
            MAX_DEV_BPS,
            MIN_LIQUIDITY,
        )
        .expect_err("stock is neither token");
    assert!(matches!(
        err,
        AfterHoursError::InvalidConfig(InvalidConfig {
            reason: CONFIG_STOCK_NOT_IN_POOL
        })
    ));
}

#[test]
fn initialize_rejects_a_scale_that_would_underflow() {
    let w = World::new();
    // 36 + 6 < 30 + 8: Morpho's scale would need a negative exponent.
    w.vm.mock_static_call(STOCK, decimalsCall {}.abi_encode(), Ok(U8::from(30u8).abi_encode()));
    let mut c = AfterHours::from(&w.vm);
    let err = c
        .initialize(
            FEED,
            POOL,
            STOCK,
            LIVE_MAX_AGE,
            TWAP_WINDOW,
            MAX_DEV_BPS,
            MIN_LIQUIDITY,
        )
        .expect_err("scale underflow");
    assert!(matches!(
        err,
        AfterHoursError::InvalidConfig(InvalidConfig {
            reason: CONFIG_SCALE_UNDERFLOW
        })
    ));
}

#[test]
fn initialize_requires_the_pause_flag_to_be_readable() {
    let w = World::new();
    w.vm.mock_static_call(STOCK, oraclePausedCall {}.abi_encode(), Err(Vec::new()));
    let mut c = AfterHours::from(&w.vm);
    let err = c
        .initialize(
            FEED,
            POOL,
            STOCK,
            LIVE_MAX_AGE,
            TWAP_WINDOW,
            MAX_DEV_BPS,
            MIN_LIQUIDITY,
        )
        .expect_err("token without oraclePaused()");
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
        )
        .expect_err("second initialize");
    assert!(matches!(err, AfterHoursError::AlreadyInitialized(_)));
    // and a failed initialize leaves the contract uninitialized
    let mut fresh = AfterHours::from(&TestVM::new());
    let (initialized, ..) = fresh.config();
    assert!(!initialized);
    assert!(fresh
        .initialize(
            FEED,
            POOL,
            STOCK,
            0,
            TWAP_WINDOW,
            MAX_DEV_BPS,
            MIN_LIQUIDITY
        )
        .is_err());
    let (initialized, ..) = fresh.config();
    assert!(!initialized);
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
}

// ---- LIVE_FEED -------------------------------------------------------------------

#[test]
fn fresh_feed_passes_through_verbatim() {
    let w = World::new();
    let c = w.deploy();
    w.mock_feed(FRIDAY_ANSWER as i128, NOW - 120);

    let (round, answer, started, updated, answered_in) = c.latest_round_data().unwrap();
    assert_eq!(round, U80::from(645u64));
    assert_eq!(answer, I256::try_from(FRIDAY_ANSWER).unwrap());
    assert_eq!(started, U256::from(NOW - 120));
    assert_eq!(updated, U256::from(NOW - 120));
    assert_eq!(answered_in, U80::from(645u64));

    // Morpho: answer * 10^(36 + 6 - 18 - 8) = answer * 1e16
    assert_eq!(
        c.price().unwrap(),
        U256::from(FRIDAY_ANSWER) * u("10000000000000000")
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

// ---- ONCHAIN_TWAP ----------------------------------------------------------------

#[test]
fn stale_feed_switches_to_the_pool_twap() {
    let w = World::new();
    let c = w.deploy();
    w.mock_feed(FRIDAY_ANSWER as i128, NOW - 40 * 3600);
    w.mock_liquidity(MIN_LIQUIDITY);
    w.mock_twap_tick(AAPL_TICK);

    let (round, answer, started, updated, answered_in) = c.latest_round_data().unwrap();
    assert_eq!(round, U80::from(645u64), "round ids stay the feed's");
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
    assert_eq!(answered_in, U80::from(645u64));

    assert_eq!(
        c.price().unwrap(),
        U256::from(AAPL_TWAP) * u("10000000000000000")
    );

    let (session, reason, ans, feed_answer, _, twap, liq, clamped) = c.state().unwrap();
    assert_eq!((session, reason), (SESSION_ONCHAIN_TWAP, REASON_NONE));
    assert_eq!(ans, U256::from(AAPL_TWAP));
    assert_eq!(twap, U256::from(AAPL_TWAP));
    assert_eq!(feed_answer, U256::from(FRIDAY_ANSWER));
    assert_eq!(liq, MIN_LIQUIDITY);
    assert!(!clamped);
}

#[test]
fn one_second_past_the_age_limit_uses_the_pool() {
    let w = World::new();
    let c = w.deploy();
    w.mock_feed(FRIDAY_ANSWER as i128, NOW - LIVE_MAX_AGE - 1);
    w.mock_liquidity(MIN_LIQUIDITY);
    w.mock_twap_tick(AAPL_TICK);
    let (session, ..) = c.state().unwrap();
    assert_eq!(session, SESSION_ONCHAIN_TWAP);
}

#[test]
fn twap_far_below_the_last_print_is_clamped_to_the_band() {
    let w = World::new();
    let c = w.deploy();
    w.mock_feed(FRIDAY_ANSWER as i128, NOW - 40 * 3600);
    w.mock_liquidity(MIN_LIQUIDITY);
    // +2000 ticks = the pool says roughly -18%; the band allows -10%.
    w.mock_twap_tick(AAPL_TICK + 2000);

    let lower = U256::from(FRIDAY_ANSWER) * U256::from(9000u64) / U256::from(10_000u64);
    let (_, answer, ..) = c.latest_round_data().unwrap();
    assert_eq!(answer, I256::from_raw(lower));
    let (session, _, ans, _, _, twap, _, clamped) = c.state().unwrap();
    assert_eq!(session, SESSION_ONCHAIN_TWAP);
    assert_eq!(ans, lower);
    assert!(twap < lower, "raw twap is reported unclamped");
    assert!(clamped);
    assert_eq!(c.price().unwrap(), lower * u("10000000000000000"));
}

#[test]
fn twap_far_above_the_last_print_is_clamped_to_the_band() {
    let w = World::new();
    let c = w.deploy();
    w.mock_feed(FRIDAY_ANSWER as i128, NOW - 40 * 3600);
    w.mock_liquidity(MIN_LIQUIDITY);
    // -2000 ticks = the pool says roughly +22%; the band allows +10%.
    w.mock_twap_tick(AAPL_TICK - 2000);

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
    w.mock_liquidity(MIN_LIQUIDITY);
    // +500 ticks = about -4.9%, inside the 10% band.
    w.mock_twap_tick(AAPL_TICK + 500);
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
    w.mock_liquidity(MIN_LIQUIDITY - 1);
    w.mock_twap_tick(AAPL_TICK);

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
fn pool_without_history_refuses_to_price() {
    let w = World::new();
    let c = w.deploy();
    w.mock_feed(FRIDAY_ANSWER as i128, NOW - 40 * 3600);
    w.mock_liquidity(MIN_LIQUIDITY);
    w.mock_twap_reverts();

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
fn broken_feed_rounds_are_no_data() {
    let w = World::new();
    let c = w.deploy();
    w.mock_liquidity(MIN_LIQUIDITY);
    w.mock_twap_tick(AAPL_TICK);
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
fn unreadable_feed_is_a_call_failure_not_a_price() {
    let w = World::new();
    let c = w.deploy();
    w.vm.mock_static_call(FEED, latestRoundDataCall {}.abi_encode(), Err(Vec::new()));
    assert!(matches!(
        c.state().expect_err("feed reverted"),
        AfterHoursError::CallFailed(CallFailed { target }) if target == FEED
    ));
}
