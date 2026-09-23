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
    function uiMultiplier() external view returns (uint256);
    function effectiveAt() external view returns (uint256);
    function slot0() external view returns (uint160, int24, uint16, uint16, uint16, uint8, bool);
}

const FEED: Address = Address::repeat_byte(0xF1);
const POOL: Address = Address::repeat_byte(0xB0);
const STOCK: Address = Address::repeat_byte(0xAA);
const USDG: Address = Address::repeat_byte(0x5D);
const POOL2: Address = Address::repeat_byte(0xB2);
const DEPLOYER: Address = Address::repeat_byte(0xDE);

const LIVE_MAX_AGE: u64 = 6 * 3600;
const TWAP_WINDOW: u32 = 1800;
const MAX_DEV_BPS: u64 = 1000;
const MIN_LIQUIDITY: u128 = 1_000_000_000_000_000; // 1e15
const MAX_ANCHOR_AGE: u64 = 5 * 86_400;
/// The feed's heartbeat: a print younger than this may come from a quiet, running feed.
const HEARTBEAT: u64 = 86_400;
/// The narrow band while the print is younger than the heartbeat.
const QUIET_BAND_BPS: u64 = 100;

const NOW: u64 = 1_800_000_000;
/// $332.52 with 8 decimals: AAPL's last Friday print on 2026-09-11.
const FRIDAY_ANSWER: u64 = 33_252_000_000;
const ROUND: u64 = 645;
/// Mean tick of the AAPL/USDG 0.05% pool. Exact value 330.96497304690...;
/// the contract truncates (integer fixed point), so it must answer ...304.
const AAPL_TICK: i64 = 218_301;
const AAPL_TWAP: u64 = 33_096_497_304;
/// POOL2 in the multi-pool tests: stock as token0, mean tick -(218301 + 500).
/// Exact 31482440784.887..., floor ...784 (80-digit decimal reference).
const POOL2_TICK: i64 = -(AAPL_TICK + 500);
const POOL2_TWAP: u64 = 31_482_440_784;
/// One token of raw balance is one share.
const ONE_X: u128 = 1_000_000_000_000_000_000;
/// AAPL's uiMultiplier on 2026-09-23 (dividends reinvested since tokenization).
const AAPL_MULT: u128 = 1_000_566_080_000_000_000;

/// The points AfterHours asks observe() for: three 600 s sub-windows.
fn points() -> Vec<u32> {
    let sub = TWAP_WINDOW / 3;
    vec![TWAP_WINDOW, 2 * sub, sub, 0]
}

/// Cumulatives for a constant mean tick and per-sub-window liquidity.
fn observation(
    start: i64,
    mean_tick: i64,
    spl_start: u64,
    liq: [u128; 3],
) -> (Vec<I56>, Vec<U160>) {
    observation_thirds(start, [mean_tick; 3], spl_start, liq)
}

/// Cumulatives for a mean tick and a liquidity per 600 s sub-window, oldest first.
fn observation_thirds(
    start: i64,
    mean_ticks: [i64; 3],
    spl_start: u64,
    liq: [u128; 3],
) -> (Vec<I56>, Vec<U160>) {
    let pts = points();
    let mut cum = vec![start];
    let mut spl = vec![U160::from(spl_start)];
    for k in 0..3 {
        let span = pts[k] - pts[k + 1];
        let last_cum = cum[k];
        cum.push(last_cum + mean_ticks[k] * i64::from(span));
        let d = U160::from((U256::from(span) << 128usize) / U256::from(liq[k]));
        let last = spl[k];
        spl.push(last.wrapping_add(d));
    }
    let ticks = cum.into_iter().map(|c| I56::try_from(c).unwrap()).collect();
    (ticks, spl)
}

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
        w.mock_multiplier(ONE_X, 0);
        w.mock_observe(AAPL_TICK, MIN_LIQUIDITY);
        w.mock_cardinality(POOL, (TWAP_WINDOW + 1) as u16);
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

    /// slot0() of `pool`; only the observation cardinality (fourth field) matters.
    fn mock_cardinality(&self, pool: Address, cardinality: u16) {
        type Slot0 = (
            sol_data::Uint<160>,
            sol_data::Int<24>,
            sol_data::Uint<16>,
            sol_data::Uint<16>,
            sol_data::Uint<16>,
            sol_data::Uint<8>,
            sol_data::Bool,
        );
        let ret = <Slot0 as SolType>::abi_encode_params(&(
            U160::ZERO,
            alloy_primitives::aliases::I24::ZERO,
            0u16,
            cardinality,
            cardinality,
            0u8,
            true,
        ));
        self.vm
            .mock_static_call(pool, slot0Call {}.abi_encode(), Ok(ret));
    }

    fn mock_paused(&self, paused: bool) {
        self.vm.mock_static_call(
            STOCK,
            oraclePausedCall {}.abi_encode(),
            Ok(paused.abi_encode()),
        );
    }

    /// The stock's share multiplier and the time it took effect.
    fn mock_multiplier(&self, multiplier: u128, effective_at: u64) {
        self.vm.mock_static_call(
            STOCK,
            uiMultiplierCall {}.abi_encode(),
            Ok(U256::from(multiplier).abi_encode()),
        );
        self.vm.mock_static_call(
            STOCK,
            effectiveAtCall {}.abi_encode(),
            Ok(U256::from(effective_at).abi_encode()),
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
    /// whose harmonic-mean liquidity is `liquidity` in every sub-window
    /// (secondsPerLiquidityCumulativeX128 grows by span * 2^128 / L).
    fn mock_observe(&self, mean_tick: i64, liquidity: u128) {
        self.mock_observe_thirds(mean_tick, [liquidity; 3]);
    }

    /// Same, with a separate liquidity per 600 s sub-window (oldest first).
    fn mock_observe_thirds(&self, mean_tick: i64, liquidity: [u128; 3]) {
        let (ticks, spl) = observation(1_000_000, mean_tick, 7_777_777, liquidity);
        self.mock_observe_raw(ticks, spl);
    }

    /// A second pool, stock as token0 (mirrored tick), with its own liquidity.
    fn mock_pool2(&self, mean_tick: i64, liquidity: u128) {
        self.vm
            .mock_static_call(POOL2, token0Call {}.abi_encode(), Ok(STOCK.abi_encode()));
        self.vm
            .mock_static_call(POOL2, token1Call {}.abi_encode(), Ok(USDG.abi_encode()));
        let (ticks, spl) = observation(5_000_000, mean_tick, 99, [liquidity; 3]);
        self.vm.mock_static_call(
            POOL2,
            observeCall {
                secondsAgos: points(),
            }
            .abi_encode(),
            Ok((ticks, spl).abi_encode_params()),
        );
    }

    fn mock_pool2_reverts(&self) {
        self.vm.mock_static_call(
            POOL2,
            observeCall {
                secondsAgos: points(),
            }
            .abi_encode(),
            Err(b"OLD".to_vec()),
        );
    }

    fn mock_pool2_wrong_shape(&self) {
        self.vm.mock_static_call(
            POOL2,
            observeCall {
                secondsAgos: points(),
            }
            .abi_encode(),
            Ok((vec![I56::ZERO], vec![U160::ZERO]).abi_encode_params()),
        );
    }

    /// POOL first (primary), POOL2 second (standby).
    fn deploy_two_pools(&self) -> AfterHours {
        let mut c = AfterHours::from(&self.vm);
        c.initialize(
            FEED,
            vec![POOL, POOL2],
            STOCK,
            LIVE_MAX_AGE,
            TWAP_WINDOW,
            MAX_DEV_BPS,
            MIN_LIQUIDITY,
            MAX_ANCHOR_AGE,
            HEARTBEAT,
            QUIET_BAND_BPS,
        )
        .expect("two pools");
        c
    }

    fn mock_observe_raw(&self, ticks: Vec<I56>, spl: Vec<U160>) {
        self.vm.mock_static_call(
            POOL,
            observeCall {
                secondsAgos: points(),
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
                secondsAgos: points(),
            }
            .abi_encode(),
            Err(b"OLD".to_vec()),
        );
    }

    fn deploy(&self) -> AfterHours {
        let mut c = AfterHours::from(&self.vm);
        c.initialize(
            FEED,
            vec![POOL],
            STOCK,
            LIVE_MAX_AGE,
            TWAP_WINDOW,
            MAX_DEV_BPS,
            MIN_LIQUIDITY,
            MAX_ANCHOR_AGE,
            HEARTBEAT,
            QUIET_BAND_BPS,
        )
        .expect("initialize");
        c
    }

    /// A pool whose three 600 s sub-windows average the given ticks (oldest
    /// first), each with the same harmonic-mean liquidity.
    fn mock_observe_ticks(&self, mean_ticks: [i64; 3], liquidity: u128) {
        let (ticks, spl) = observation_thirds(1_000_000, mean_ticks, 7_777_777, [liquidity; 3]);
        self.mock_observe_raw(ticks, spl);
    }

    /// One pool, with its own quiet tier.
    fn deploy_with_tier(&self, heartbeat: u64, quiet_band_bps: u64) -> AfterHours {
        let mut c = AfterHours::from(&self.vm);
        c.initialize(
            FEED,
            vec![POOL],
            STOCK,
            LIVE_MAX_AGE,
            TWAP_WINDOW,
            MAX_DEV_BPS,
            MIN_LIQUIDITY,
            MAX_ANCHOR_AGE,
            heartbeat,
            quiet_band_bps,
        )
        .expect("initialize");
        c
    }

    fn try_deploy(&self) -> Result<(), AfterHoursError> {
        self.try_deploy_with(vec![POOL])
    }

    fn try_deploy_with(&self, pools: Vec<Address>) -> Result<(), AfterHoursError> {
        let mut c = AfterHours::from(&self.vm);
        c.initialize(
            FEED,
            pools,
            STOCK,
            LIVE_MAX_AGE,
            TWAP_WINDOW,
            MAX_DEV_BPS,
            MIN_LIQUIDITY,
            MAX_ANCHOR_AGE,
            HEARTBEAT,
            QUIET_BAND_BPS,
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
    assert_eq!(c.quiet_tier(), (HEARTBEAT, QUIET_BAND_BPS));
    assert_eq!(c.decimals().unwrap(), 8);
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
    let cases: [(u64, u32, u64, u128, u64, u8); 10] = [
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
            2,
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
            .initialize(
                FEED,
                vec![POOL],
                STOCK,
                age,
                window,
                dev,
                min_liq,
                anchor,
                HEARTBEAT,
                QUIET_BAND_BPS,
            )
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
fn initialize_bounds_the_quiet_tier() {
    let w = World::new();
    let mut c = AfterHours::from(&w.vm);
    let deploy = |c: &mut AfterHours, heartbeat: u64, quiet: u64| {
        c.initialize(
            FEED,
            vec![POOL],
            STOCK,
            LIVE_MAX_AGE,
            TWAP_WINDOW,
            MAX_DEV_BPS,
            MIN_LIQUIDITY,
            MAX_ANCHOR_AGE,
            heartbeat,
            quiet,
        )
    };
    for (heartbeat, quiet) in [
        (LIVE_MAX_AGE - 1, QUIET_BAND_BPS), // shorter than the live feed
        (MAX_ANCHOR_AGE, QUIET_BAND_BPS),   // the wide band could never apply
        (HEARTBEAT, 0),                     // a zero band refuses every quiet read
        (HEARTBEAT, MAX_DEV_BPS + 1),       // wider than the closed-market band
    ] {
        let err = deploy(&mut c, heartbeat, quiet).expect_err("must reject");
        assert_eq!(
            config_reason(err),
            CONFIG_QUIET_TIER,
            "case ({heartbeat},{quiet})"
        );
    }
    // The edges are accepted: an empty quiet tier, and one as wide as the band.
    deploy(&mut c, LIVE_MAX_AGE, MAX_DEV_BPS).expect("empty tier, full band");
    assert_eq!(c.quiet_tier(), (LIVE_MAX_AGE, MAX_DEV_BPS));
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
fn initialize_rejects_a_primary_that_keeps_too_little_history() {
    // One observation short of the window: anyone swapping once a second could
    // then make observe() answer "OLD" and switch pricing off.
    let w = World::new();
    w.mock_cardinality(POOL, TWAP_WINDOW as u16);
    assert_eq!(
        config_reason(w.try_deploy().expect_err("cardinality 1800")),
        CONFIG_PRIMARY_HISTORY_TOO_SHORT
    );
    // An unreadable slot0 is a failed read, not a pass.
    let w = World::new();
    w.vm.mock_static_call(POOL, slot0Call {}.abi_encode(), Err(Vec::new()));
    assert!(matches!(
        w.try_deploy().expect_err("slot0 reverted"),
        AfterHoursError::CallFailed(CallFailed { target }) if target == POOL
    ));
    // Exactly window + 1 is enough, and a standby is not asked: it only covers
    // a primary that cannot be observed.
    let w = World::new();
    w.mock_pool2(POOL2_TICK, MIN_LIQUIDITY);
    w.mock_cardinality(POOL2, 10);
    w.try_deploy_with(vec![POOL, POOL2])
        .expect("primary at window + 1, standby short");
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
fn initialize_requires_the_share_multiplier_to_be_readable() {
    let w = World::new();
    w.vm.mock_static_call(STOCK, uiMultiplierCall {}.abi_encode(), Err(Vec::new()));
    let err = w.try_deploy().expect_err("token without uiMultiplier()");
    assert!(matches!(err, AfterHoursError::CallFailed(CallFailed { target }) if target == STOCK));
    let w = World::new();
    w.vm.mock_static_call(STOCK, effectiveAtCall {}.abi_encode(), Err(Vec::new()));
    let err = w.try_deploy().expect_err("token without effectiveAt()");
    assert!(matches!(err, AfterHoursError::CallFailed(CallFailed { target }) if target == STOCK));
    let w = World::new();
    w.mock_multiplier(0, 0);
    assert_eq!(
        config_reason(w.try_deploy().expect_err("zero multiplier")),
        CONFIG_BAD_MULTIPLIER
    );
}

#[test]
fn initialize_runs_only_once() {
    let w = World::new();
    let mut c = w.deploy();
    let err = c
        .initialize(
            FEED,
            vec![POOL],
            STOCK,
            LIVE_MAX_AGE,
            TWAP_WINDOW,
            MAX_DEV_BPS,
            MIN_LIQUIDITY,
            MAX_ANCHOR_AGE,
            HEARTBEAT,
            QUIET_BAND_BPS,
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
    assert!(matches!(
        c.decimals().expect_err("uninitialized"),
        AfterHoursError::NotInitialized(_)
    ));
    assert!(matches!(
        c.get_round_data(U80::from(ROUND - 1))
            .expect_err("uninitialized"),
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

    let (session, reason, ans, feed_answer, feed_updated, twap, liq, clamped, _) =
        c.state().unwrap();
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

    let (session, reason, ans, feed_answer, _, twap, liq, clamped, pool) = c.state().unwrap();
    assert_eq!((session, reason), (SESSION_ONCHAIN_TWAP, REASON_NONE));
    assert_eq!(ans, U256::from(AAPL_TWAP));
    assert_eq!(twap, U256::from(AAPL_TWAP));
    assert_eq!(feed_answer, U256::from(FRIDAY_ANSWER));
    assert_eq!(
        liq, MIN_LIQUIDITY,
        "harmonic-mean liquidity over the window"
    );
    assert!(!clamped);
    assert_eq!(pool, POOL, "state() names the pool that answered");
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
    let (session, _, ans, _, _, twap, _, clamped, _) = c.state().unwrap();
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
    let (_, _, _, _, _, twap, _, clamped, _) = c.state().unwrap();
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
    let (_, _, ans, _, _, twap, _, clamped, _) = c.state().unwrap();
    assert_eq!(ans, twap);
    assert!(!clamped);
    let lower = U256::from(FRIDAY_ANSWER) * U256::from(9000u64) / U256::from(10_000u64);
    assert!(ans > lower && ans < U256::from(FRIDAY_ANSWER));
}

// ---- the median sub-window ---------------------------------------------------------

/// AAPL_TICK + 500 on the primary (stock as token1), about -5.3% against the
/// print: the same exact price as POOL2's mirrored tick, so the same floor.
const DOWN_TWAP: u64 = POOL2_TWAP;
/// AAPL_TICK + 300: floor of the exact 32118396159.78 (80-digit decimal
/// reference); the contract's integer path lands on the same integer.
const HELD_TWAP: u64 = 32_118_396_159;

#[test]
fn a_spike_inside_one_sub_window_moves_nothing() {
    // For one 10-minute sub-window the pool trades at 7.4x (20,000 ticks lower:
    // less stock per USDG). A whole-window average would have moved 6,667 ticks
    // (+95%) and sat at the +10% band edge; the median sub-window has not moved.
    let w = World::new();
    let c = w.deploy();
    w.mock_feed(FRIDAY_ANSWER as i128, NOW - 40 * 3600);
    w.mock_observe_ticks([AAPL_TICK, AAPL_TICK - 20_000, AAPL_TICK], MIN_LIQUIDITY);
    let (session, reason, ans, _, _, twap, _, clamped, _) = c.state().unwrap();
    assert_eq!((session, reason), (SESSION_ONCHAIN_TWAP, REASON_NONE));
    assert_eq!(
        (ans, twap, clamped),
        (U256::from(AAPL_TWAP), U256::from(AAPL_TWAP), false)
    );
    // the last sub-window, the first, either way
    w.mock_observe_ticks([AAPL_TICK - 20_000, AAPL_TICK, AAPL_TICK], MIN_LIQUIDITY);
    let (_, _, ans, ..) = c.state().unwrap();
    assert_eq!(ans, U256::from(AAPL_TWAP));
    w.mock_observe_ticks([AAPL_TICK, AAPL_TICK, AAPL_TICK + 20_000], MIN_LIQUIDITY);
    let (_, _, ans, ..) = c.state().unwrap();
    assert_eq!(ans, U256::from(AAPL_TWAP));
}

#[test]
fn a_move_held_through_two_sub_windows_is_priced() {
    // The pool fell 300 ticks (-2.96%) twenty minutes ago and stayed there.
    let w = World::new();
    let c = w.deploy();
    w.mock_feed(FRIDAY_ANSWER as i128, NOW - 40 * 3600);
    w.mock_observe_ticks([AAPL_TICK, AAPL_TICK + 300, AAPL_TICK + 300], MIN_LIQUIDITY);
    let (session, _, ans, _, _, twap, _, clamped, _) = c.state().unwrap();
    assert_eq!(session, SESSION_ONCHAIN_TWAP);
    assert_eq!(
        (ans, twap, clamped),
        (U256::from(HELD_TWAP), U256::from(HELD_TWAP), false)
    );
    // ten minutes in, only one sub-window has it: not priced yet
    w.mock_observe_ticks([AAPL_TICK, AAPL_TICK, AAPL_TICK + 300], MIN_LIQUIDITY);
    let (_, _, ans, ..) = c.state().unwrap();
    assert_eq!(ans, U256::from(AAPL_TWAP));
    // three different sub-windows: the middle one, whatever the order
    w.mock_observe_ticks(
        [AAPL_TICK + 600, AAPL_TICK - 300, AAPL_TICK + 300],
        MIN_LIQUIDITY,
    );
    let (_, _, ans, ..) = c.state().unwrap();
    assert_eq!(ans, U256::from(HELD_TWAP));
}

// ---- the quiet tier ----------------------------------------------------------------

#[test]
fn a_quiet_feed_holds_the_pool_to_the_narrow_band() {
    // A weekday: the feed last printed seven hours ago because the price has
    // not moved its 0.5% threshold since. The pool says -5.3%; within the
    // heartbeat it is held to -1%.
    let w = World::new();
    let c = w.deploy();
    w.mock_feed(FRIDAY_ANSWER as i128, NOW - 7 * 3600);
    w.mock_observe(AAPL_TICK + 500, MIN_LIQUIDITY);
    let narrow =
        U256::from(FRIDAY_ANSWER) * U256::from(10_000 - QUIET_BAND_BPS) / U256::from(10_000u64);
    let (session, reason, ans, _, _, twap, _, clamped, _) = c.state().unwrap();
    assert_eq!((session, reason), (SESSION_ONCHAIN_TWAP, REASON_NONE));
    assert_eq!((ans, clamped), (narrow, true));
    assert_eq!(
        twap,
        U256::from(DOWN_TWAP),
        "the unclamped pool price is reported"
    );
    assert_eq!(c.price().unwrap(), narrow * u(MORPHO_SCALE));
    // a pool inside the narrow band passes through
    w.mock_observe(AAPL_TICK, MIN_LIQUIDITY);
    let (_, _, ans, _, _, _, _, clamped, _) = c.state().unwrap();
    assert_eq!((ans, clamped), (U256::from(AAPL_TWAP), false));
}

#[test]
fn the_band_widens_only_once_the_heartbeat_has_passed() {
    let w = World::new();
    let c = w.deploy();
    w.mock_observe(AAPL_TICK + 500, MIN_LIQUIDITY);
    let narrow =
        U256::from(FRIDAY_ANSWER) * U256::from(10_000 - QUIET_BAND_BPS) / U256::from(10_000u64);
    w.mock_feed(FRIDAY_ANSWER as i128, NOW - HEARTBEAT);
    let (_, _, ans, _, _, _, _, clamped, _) = c.state().unwrap();
    assert_eq!((ans, clamped), (narrow, true), "at the heartbeat: narrow");
    // One second later the feed has missed its heartbeat: the market is closed,
    // and the pool's -5.3% is inside the wide band.
    w.mock_feed(FRIDAY_ANSWER as i128, NOW - HEARTBEAT - 1);
    let (session, _, ans, _, _, _, _, clamped, _) = c.state().unwrap();
    assert_eq!(session, SESSION_ONCHAIN_TWAP);
    assert_eq!((ans, clamped), (U256::from(DOWN_TWAP), false));
}

#[test]
fn an_empty_quiet_tier_goes_straight_to_the_wide_band() {
    let w = World::new();
    let c = w.deploy_with_tier(LIVE_MAX_AGE, QUIET_BAND_BPS);
    w.mock_observe(AAPL_TICK + 500, MIN_LIQUIDITY);
    w.mock_feed(FRIDAY_ANSWER as i128, NOW - LIVE_MAX_AGE - 1);
    let (session, _, ans, _, _, _, _, clamped, _) = c.state().unwrap();
    assert_eq!(session, SESSION_ONCHAIN_TWAP);
    assert_eq!((ans, clamped), (U256::from(DOWN_TWAP), false));
}

#[test]
fn a_weekend_read_uses_the_wide_band_from_the_first_hour() {
    // NOW is Friday 2027-01-15 08:00 UTC. The same seven-hour-old print and the
    // same -5.3% pool: held to -1% on the Friday, passed through on the
    // Saturday and the Sunday, when the market is closed whatever the feed's age.
    let w = World::new();
    let c = w.deploy();
    w.mock_observe(AAPL_TICK + 500, MIN_LIQUIDITY);
    let narrow =
        U256::from(FRIDAY_ANSWER) * U256::from(10_000 - QUIET_BAND_BPS) / U256::from(10_000u64);
    for (day, now, want, clamped_want) in [
        ("Friday", NOW, narrow, true),
        ("Saturday", NOW + 86_400, U256::from(DOWN_TWAP), false),
        ("Sunday", NOW + 2 * 86_400, U256::from(DOWN_TWAP), false),
        ("Monday", NOW + 3 * 86_400, narrow, true),
    ] {
        w.vm.set_block_timestamp(now);
        w.mock_feed(FRIDAY_ANSWER as i128, now - 7 * 3600);
        let (session, _, ans, _, _, _, _, clamped, _) = c.state().unwrap();
        assert_eq!(session, SESSION_ONCHAIN_TWAP, "{day}");
        assert_eq!((ans, clamped), (want, clamped_want), "{day}");
    }
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
    let (session, reason, ans, _, _, twap, liq, _, pool) = c.state().unwrap();
    assert_eq!((session, reason), (SESSION_NO_DATA, REASON_POOL_TOO_THIN));
    assert_eq!((ans, twap), (U256::ZERO, U256::ZERO));
    assert_eq!(liq, MIN_LIQUIDITY - 1);
    assert_eq!(pool, POOL, "the refused pool is still named");
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
    let (session, reason, _, _, _, _, liq, _, _) = c.state().unwrap();
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
    // no liquidity-seconds recorded: the pool answered, so it is the venue,
    // and an empty pool is a thin one (it never hands over to a standby)
    w.mock_observe_raw(vec![I56::ZERO; 4], vec![U160::from(5u64); 4]);
    let (session, reason, _, _, _, _, liq, _, pool) = c.state().unwrap();
    assert_eq!((session, reason), (SESSION_NO_DATA, REASON_POOL_TOO_THIN));
    assert_eq!((liq, pool), (0, POOL));
    // a mean tick outside Uniswap's range
    let (ticks, spl) = observation(0, 1_000_000, 0, [MIN_LIQUIDITY; 3]);
    w.mock_observe_raw(ticks, spl);
    let (session, reason, ..) = c.state().unwrap();
    assert_eq!(
        (session, reason),
        (SESSION_NO_DATA, REASON_TWAP_UNAVAILABLE)
    );
}

#[test]
fn a_pool_price_that_rounds_to_zero_per_share_is_refused() {
    // One raw unit at 1e-8 USD (tick 460517) and two shares per raw unit: the
    // per-share price floors to 0, which is not a price.
    let w = World::new();
    let c = w.deploy();
    w.mock_multiplier(2 * ONE_X, NOW - 40 * 86_400);
    w.mock_feed(1, NOW - 40 * 3600);
    w.mock_observe(460_517, MIN_LIQUIDITY);
    let (session, reason, ans, ..) = c.state().unwrap();
    assert_eq!(
        (session, reason),
        (SESSION_NO_DATA, REASON_TWAP_UNAVAILABLE)
    );
    assert_eq!(ans, U256::ZERO);
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
    // Largest positive int256. LIVE: price() would overflow the 1e16 Morpho scale,
    // and says so with its own reason; the Chainlink surfaces still answer.
    w.mock_feed_raw(I256::MAX, NOW - 60);
    assert!(matches!(
        c.price().expect_err("overflow"),
        AfterHoursError::NoData(NoData {
            reason: REASON_PRICE_OVERFLOW
        })
    ));
    let (_, answer, ..) = c.latest_round_data().unwrap();
    assert_eq!(answer, I256::MAX, "the feed itself still passes through");
    // Stale: the band arithmetic would overflow, so the oracle refuses before
    // reading any pool; the refusal carries no TWAP and names no pool.
    w.mock_feed_raw(I256::MAX, NOW - 40 * 3600);
    let before = w.vm.call_log().len();
    let (session, reason, _, _, _, twap, liq, _, pool) = c.state().unwrap();
    assert_eq!((session, reason), (SESSION_NO_DATA, REASON_FEED_INVALID));
    assert_eq!((twap, liq, pool), (U256::ZERO, 0, Address::ZERO));
    assert!(
        w.vm.call_log()
            .split_off(before)
            .iter()
            .all(|(to, _)| *to != POOL),
        "no pool read when the anchor itself is unusable"
    );
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
    w.vm.mock_static_call(FEED, latestRoundDataCall {}.abi_encode(), Err(Vec::new()));
    let err = w.try_deploy().expect_err("feed without latestRoundData()");
    assert!(matches!(err, AfterHoursError::CallFailed(CallFailed { target }) if target == FEED));
}

/// A sub-window in which the price left every position for one second:
/// Uniswap accumulates seconds * 2^128 / max(L, 1), so that second adds 2^128
/// and the sub-window's harmonic mean collapses to about its length.
fn dipped(deep: u128) -> U160 {
    U160::from((U256::from(599u64) << 128usize) / U256::from(deep) + (U256::from(1u64) << 128usize))
}

#[test]
fn one_second_out_of_range_no_longer_switches_pricing_off() {
    // Before: one second at zero liquidity collapsed the whole window's mean and
    // refused for 30 minutes, a cheap and repeatable way to stop liquidations.
    // Now it collapses one sub-window; the median of three still prices.
    let w = World::new();
    let c = w.deploy();
    w.mock_feed(FRIDAY_ANSWER as i128, NOW - 40 * 3600);
    let deep: u128 = 1_600_000_000_000_000_000; // 1.6e18, today's pool
    let (ticks, mut spl) = observation(1_000_000, AAPL_TICK, 0, [deep; 3]);
    // the dip in the middle sub-window
    let d0 = spl[1];
    spl[2] = d0.wrapping_add(dipped(deep));
    spl[3] = spl[2].wrapping_add(spl[1]);
    w.mock_observe_raw(ticks, spl);
    let (session, reason, ans, _, _, _, liq, _, _) = c.state().unwrap();
    assert_eq!((session, reason), (SESSION_ONCHAIN_TWAP, REASON_NONE));
    assert_eq!(ans, U256::from(AAPL_TWAP));
    assert_eq!(liq, deep, "the median sub-window is a deep one");
}

#[test]
fn empty_liquidity_in_two_of_three_sub_windows_refuses() {
    let w = World::new();
    let c = w.deploy();
    w.mock_feed(FRIDAY_ANSWER as i128, NOW - 40 * 3600);
    let deep: u128 = 1_600_000_000_000_000_000;
    let (ticks, _) = observation(1_000_000, AAPL_TICK, 0, [deep; 3]);
    let first = dipped(deep);
    let spl = vec![
        U160::ZERO,
        first,
        first.wrapping_add(dipped(deep)),
        first.wrapping_add(dipped(deep)).wrapping_add(U160::from(
            (U256::from(600u64) << 128usize) / U256::from(deep),
        )),
    ];
    w.mock_observe_raw(ticks, spl);
    let (session, reason, _, _, _, _, liq, _, pool) = c.state().unwrap();
    assert_eq!((session, reason), (SESSION_NO_DATA, REASON_POOL_TOO_THIN));
    assert!(
        liq < 1000,
        "median collapses to about a sub-window's length: {liq}"
    );
    assert_eq!(pool, POOL);
}

#[test]
fn liquidity_accumulator_wraps_around_uint160() {
    // The pool's secondsPerLiquidityCumulativeX128 is an unchecked uint160
    // accumulator; a window that straddles the wrap must still read correctly.
    let w = World::new();
    let c = w.deploy();
    w.mock_feed(FRIDAY_ANSWER as i128, NOW - 40 * 3600);
    let (ticks, _) = observation(1_000_000, AAPL_TICK, 0, [MIN_LIQUIDITY; 3]);
    let third = U160::from((U256::from(600u64) << 128usize) / U256::from(MIN_LIQUIDITY));
    let s0 = U160::MAX - U160::from(5u64);
    let spl = vec![
        s0,
        s0.wrapping_add(third),
        s0.wrapping_add(third).wrapping_add(third),
        s0.wrapping_add(third)
            .wrapping_add(third)
            .wrapping_add(third),
    ];
    assert!(spl[1] < spl[0], "the test straddles the wrap");
    w.mock_observe_raw(ticks, spl);
    let (session, _, ans, _, _, _, liq, _, _) = c.state().unwrap();
    assert_eq!(session, SESSION_ONCHAIN_TWAP);
    assert_eq!(liq, MIN_LIQUIDITY);
    assert_eq!(ans, U256::from(AAPL_TWAP));
}

#[test]
fn absurdly_deep_liquidity_saturates_instead_of_overflowing() {
    let w = World::new();
    let c = w.deploy();
    w.mock_feed(FRIDAY_ANSWER as i128, NOW - 40 * 3600);
    let (ticks, _) = observation(1_000_000, AAPL_TICK, 0, [MIN_LIQUIDITY; 3]);
    // deltas of 1: 600 * 2^128 liquidity per sub-window, far beyond u128
    w.mock_observe_raw(
        ticks,
        vec![
            U160::from(9u64),
            U160::from(10u64),
            U160::from(11u64),
            U160::from(12u64),
        ],
    );
    let (session, _, _, _, _, _, liq, _, _) = c.state().unwrap();
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
        c.get_round_data(U80::from(ROUND))
            .expect_err("current round is refused while paused"),
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
fn history_is_served_even_when_the_stock_token_cannot_be_read() {
    // If an upgrade of the issuer's token dropped a function this oracle reads,
    // pricing stops for good; the feed's history does not depend on the token.
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
    w.vm.mock_static_call(STOCK, effectiveAtCall {}.abi_encode(), Err(Vec::new()));
    assert!(matches!(
        c.state().expect_err("token read fails"),
        AfterHoursError::CallFailed(CallFailed { target }) if target == STOCK
    ));
    assert!(matches!(
        c.get_round_data(U80::from(ROUND)).expect_err("the current round needs the token"),
        AfterHoursError::CallFailed(CallFailed { target }) if target == STOCK
    ));
    assert_eq!(c.get_round_data(U80::from(ROUND - 1)).unwrap(), old);
}

// ---- share multiplier ---------------------------------------------------------------

#[test]
fn price_values_raw_units_with_the_share_multiplier() {
    // One raw AAPL unit is 1.00056608 shares: the feed prices a share, Morpho
    // lends against raw units.
    let w = World::new();
    let c = w.deploy();
    w.mock_multiplier(AAPL_MULT, NOW - 40 * 86_400);
    w.mock_feed(FRIDAY_ANSWER as i128, NOW - 120);
    let (_, answer, ..) = c.latest_round_data().unwrap();
    assert_eq!(
        answer,
        I256::try_from(FRIDAY_ANSWER).unwrap(),
        "Chainlink-shaped answers stay per share"
    );
    assert_eq!(
        c.price().unwrap(),
        U256::from(FRIDAY_ANSWER) * u(MORPHO_SCALE) * U256::from(AAPL_MULT) / U256::from(ONE_X)
    );
}

#[test]
fn the_pool_average_is_reported_per_share() {
    let w = World::new();
    let c = w.deploy();
    w.mock_multiplier(AAPL_MULT, NOW - 40 * 86_400);
    w.mock_feed(FRIDAY_ANSWER as i128, NOW - 40 * 3600);
    let per_share = U256::from(AAPL_TWAP) * U256::from(ONE_X) / U256::from(AAPL_MULT);
    let (session, _, ans, _, _, twap, ..) = c.state().unwrap();
    assert_eq!(session, SESSION_ONCHAIN_TWAP);
    assert_eq!((twap, ans), (per_share, per_share));
    // and price() turns it back into raw units, within the two floors
    let raw = c.price().unwrap();
    let exact = U256::from(AAPL_TWAP) * u(MORPHO_SCALE);
    assert!(raw <= exact && exact - raw <= u(MORPHO_SCALE) * U256::from(2u64));
}

#[test]
fn a_dividend_after_the_last_print_moves_pricing_to_the_pool() {
    // The multiplier grew after the feed's last print: the print is in
    // pre-dividend shares, so it is not passed through even though it is fresh.
    let w = World::new();
    let c = w.deploy();
    let mult: u128 = 1_003_000_000_000_000_000; // a 0.3% dividend reinvested
    w.mock_multiplier(mult, NOW - 60);
    w.mock_feed(FRIDAY_ANSWER as i128, NOW - 120);
    let per_share = U256::from(AAPL_TWAP) * U256::from(ONE_X) / U256::from(mult);
    let (session, reason, ans, _, _, twap, _, clamped, _) = c.state().unwrap();
    assert_eq!((session, reason), (SESSION_ONCHAIN_TWAP, REASON_NONE));
    assert_eq!((ans, twap, clamped), (per_share, per_share, false));
    // once the feed prints after the change, it is live again
    w.mock_feed(FRIDAY_ANSWER as i128, NOW - 30);
    let (session, ..) = c.state().unwrap();
    assert_eq!(session, SESSION_LIVE_FEED);
}

#[test]
fn a_distribution_beyond_the_narrow_band_is_held_to_it_not_refused() {
    // A 3% special distribution on a weekday: the pool per share sits 3.4% under
    // the pre-distribution print. Outside the 1% band, so the answer is held at
    // -1%; only a move past the wide band (a split) means the print knows nothing.
    let w = World::new();
    let c = w.deploy();
    let mult: u128 = 1_030_000_000_000_000_000;
    w.mock_multiplier(mult, NOW - 60);
    w.mock_feed(FRIDAY_ANSWER as i128, NOW - 120);
    let narrow =
        U256::from(FRIDAY_ANSWER) * U256::from(10_000 - QUIET_BAND_BPS) / U256::from(10_000u64);
    let (session, reason, ans, _, _, twap, _, clamped, _) = c.state().unwrap();
    assert_eq!((session, reason), (SESSION_ONCHAIN_TWAP, REASON_NONE));
    assert_eq!((ans, clamped), (narrow, true));
    assert_eq!(
        twap,
        U256::from(AAPL_TWAP) * U256::from(ONE_X) / U256::from(mult)
    );
}

#[test]
fn a_split_after_the_last_print_refuses_until_the_feed_prints() {
    let w = World::new();
    let c = w.deploy();
    // 2-for-1: every token of raw balance is now two shares; the last print is pre-split.
    w.mock_multiplier(2 * ONE_X, NOW - 60);
    w.mock_feed(FRIDAY_ANSWER as i128, NOW - 120);
    let (session, reason, ans, _, _, twap, _, _, pool) = c.state().unwrap();
    assert_eq!(
        (session, reason),
        (SESSION_NO_DATA, REASON_MULTIPLIER_CHANGED),
        "the pool per share is half the pre-split print: refuse, never clamp"
    );
    assert_eq!((ans, twap, pool), (U256::ZERO, U256::ZERO, POOL));
    assert!(matches!(
        c.price().expect_err("split"),
        AfterHoursError::NoData(NoData {
            reason: REASON_MULTIPLIER_CHANGED
        })
    ));
    // The feed prints the post-split price: live again, and a raw token is worth
    // what it was worth before the split.
    let half = FRIDAY_ANSWER / 2;
    w.mock_feed(half as i128, NOW - 30);
    let (session, ..) = c.state().unwrap();
    assert_eq!(session, SESSION_LIVE_FEED);
    assert_eq!(
        c.price().unwrap(),
        U256::from(FRIDAY_ANSWER) * u(MORPHO_SCALE)
    );
}

#[test]
fn a_scheduled_multiplier_changes_nothing_before_it_takes_effect() {
    // The token keeps answering the old multiplier until effectiveAt.
    let w = World::new();
    let c = w.deploy();
    w.mock_multiplier(ONE_X, NOW + 3600);
    w.mock_feed(FRIDAY_ANSWER as i128, NOW - 120);
    let (session, ..) = c.state().unwrap();
    assert_eq!(session, SESSION_LIVE_FEED);
}

#[test]
fn an_unreadable_or_zero_multiplier_is_a_failed_read() {
    let w = World::new();
    let c = w.deploy();
    w.mock_multiplier(0, 0);
    assert!(matches!(
        c.state().expect_err("zero multiplier"),
        AfterHoursError::CallFailed(CallFailed { target }) if target == STOCK
    ));
    w.vm.mock_static_call(STOCK, uiMultiplierCall {}.abi_encode(), Err(Vec::new()));
    assert!(matches!(
        c.state().expect_err("reverting multiplier"),
        AfterHoursError::CallFailed(CallFailed { target }) if target == STOCK
    ));
}

// ---- several pools ------------------------------------------------------------------

#[test]
fn the_primary_prices_and_a_deeper_standby_cannot_take_over() {
    let w = World::new();
    // POOL2 (stock as token0) quotes 500 ticks away with 3x the primary's depth:
    // exactly what someone deepening a shallow tier would set up.
    w.mock_pool2(POOL2_TICK, MIN_LIQUIDITY * 3);
    let c = w.deploy_two_pools();
    assert_eq!(c.pools(), vec![POOL, POOL2]);
    w.mock_feed(FRIDAY_ANSWER as i128, NOW - 40 * 3600);

    let before = w.vm.call_log().len();
    let (session, _, ans, _, _, _, liq, _, pool) = c.state().unwrap();
    assert_eq!(
        (session, pool, liq),
        (SESSION_ONCHAIN_TWAP, POOL, MIN_LIQUIDITY),
        "the venue is fixed: the primary prices even when a standby looks deeper"
    );
    assert_eq!(ans, U256::from(AAPL_TWAP));
    assert!(
        w.vm.call_log()
            .split_off(before)
            .iter()
            .all(|(to, _)| *to != POOL2),
        "a standby is not even read while the primary can be observed"
    );

    // The primary thins below the floor: refuse; never move to the deep standby.
    w.mock_observe(AAPL_TICK, MIN_LIQUIDITY / 2);
    let (session, reason, ans, _, _, twap, liq, _, pool) = c.state().unwrap();
    assert_eq!((session, reason), (SESSION_NO_DATA, REASON_POOL_TOO_THIN));
    assert_eq!(
        (pool, liq),
        (POOL, MIN_LIQUIDITY / 2),
        "the refusal names the primary"
    );
    assert_eq!((ans, twap), (U256::ZERO, U256::ZERO));
}

#[test]
fn a_standby_prices_only_while_the_primary_cannot_be_observed() {
    let w = World::new();
    w.mock_pool2(POOL2_TICK, MIN_LIQUIDITY);
    let c = w.deploy_two_pools();
    w.mock_feed(FRIDAY_ANSWER as i128, NOW - 40 * 3600);

    // Primary reverts "OLD": the standby answers with its own price.
    w.mock_observe_reverts();
    let (session, _, ans, _, _, twap, liq, clamped, pool) = c.state().unwrap();
    assert_eq!(
        (session, pool, liq),
        (SESSION_ONCHAIN_TWAP, POOL2, MIN_LIQUIDITY)
    );
    assert_eq!(twap, U256::from(POOL2_TWAP), "stock-as-token0 conversion");
    assert_eq!(ans, U256::from(POOL2_TWAP));
    assert!(!clamped);

    // Primary answers with the wrong shape: same.
    w.mock_observe_raw(vec![I56::ZERO], vec![U160::ZERO]);
    let (_, _, _, _, _, _, _, _, pool) = c.state().unwrap();
    assert_eq!(pool, POOL2);

    // Standby thin while it is the only observable pool: refuse, naming it.
    w.mock_pool2(POOL2_TICK, MIN_LIQUIDITY - 1);
    let (session, reason, _, _, _, _, liq, _, pool) = c.state().unwrap();
    assert_eq!((session, reason), (SESSION_NO_DATA, REASON_POOL_TOO_THIN));
    assert_eq!((pool, liq), (POOL2, MIN_LIQUIDITY - 1));

    // Nothing observable: refuse, naming no pool.
    w.mock_pool2_reverts();
    let (session, reason, _, _, _, _, liq, _, pool) = c.state().unwrap();
    assert_eq!(
        (session, reason),
        (SESSION_NO_DATA, REASON_TWAP_UNAVAILABLE)
    );
    assert_eq!((pool, liq), (Address::ZERO, 0));

    // The primary comes back: it prices again, whatever the standby does.
    w.mock_observe(AAPL_TICK, MIN_LIQUIDITY);
    w.mock_pool2_wrong_shape();
    let (session, _, ans, _, _, _, _, _, pool) = c.state().unwrap();
    assert_eq!(
        (session, pool, ans),
        (SESSION_ONCHAIN_TWAP, POOL, U256::from(AAPL_TWAP))
    );
}

#[test]
fn initialize_rejects_duplicate_pools() {
    let w = World::new();
    assert_eq!(
        config_reason(w.try_deploy_with(vec![POOL, POOL]).expect_err("dup")),
        CONFIG_DUPLICATE_POOL
    );
    w.mock_pool2(POOL2_TICK, MIN_LIQUIDITY);
    assert_eq!(
        config_reason(
            w.try_deploy_with(vec![POOL, POOL2, POOL])
                .expect_err("dup at the end")
        ),
        CONFIG_DUPLICATE_POOL
    );
}

#[test]
fn initialize_bounds_the_pool_list() {
    let w = World::new();
    assert_eq!(
        config_reason(w.try_deploy_with(vec![]).expect_err("no pools")),
        CONFIG_POOL_COUNT
    );
    assert_eq!(
        config_reason(
            w.try_deploy_with(vec![POOL, POOL, POOL, POOL])
                .expect_err("four pools")
        ),
        CONFIG_POOL_COUNT
    );
}

#[test]
fn initialize_rejects_pools_with_different_quotes() {
    let w = World::new();
    let other_quote = Address::repeat_byte(0x77);
    w.mock_pool2(-AAPL_TICK, MIN_LIQUIDITY);
    w.vm.mock_static_call(
        POOL2,
        token1Call {}.abi_encode(),
        Ok(other_quote.abi_encode()),
    );
    assert_eq!(
        config_reason(
            w.try_deploy_with(vec![POOL, POOL2])
                .expect_err("quote mismatch")
        ),
        CONFIG_POOL_QUOTE_MISMATCH
    );
}

#[test]
fn initialize_checks_every_pool_for_history() {
    let w = World::new();
    w.mock_pool2(-AAPL_TICK, MIN_LIQUIDITY);
    w.vm.mock_static_call(
        POOL2,
        observeCall {
            secondsAgos: points(),
        }
        .abi_encode(),
        Err(b"OLD".to_vec()),
    );
    assert_eq!(
        config_reason(
            w.try_deploy_with(vec![POOL, POOL2])
                .expect_err("second pool has no history")
        ),
        CONFIG_POOL_NOT_OBSERVABLE
    );
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

// ---- properties (proptest) ----------------------------------------------------------
//
// Random inputs through the same paths the chain will exercise: the math must
// stay monotonic, the answer must stay inside the band, and nothing may panic
// whatever the feed or the pool returns.

mod props {
    use super::*;
    use crate::tickmath;
    use proptest::prelude::*;

    /// The band a read with a print `age` seconds old is held to. Property
    /// runs read at NOW, a Friday, so the weekend rule never applies here
    /// (`a_weekend_read_uses_the_wide_band_from_the_first_hour` covers it).
    fn band_for(age: u64) -> u64 {
        if age <= HEARTBEAT {
            QUIET_BAND_BPS
        } else {
            MAX_DEV_BPS
        }
    }

    fn lower_upper(feed: U256, band_bps: u64) -> (U256, U256) {
        (
            feed * U256::from(10_000 - band_bps) / U256::from(10_000u64),
            feed * U256::from(10_000 + band_bps) / U256::from(10_000u64),
        )
    }

    /// Garbage and near-valid inputs from the feed and the pool never panic, every
    /// session is consistent with its price surfaces, and the run proves it reached
    /// every session and every refusal reason instead of stopping at the first gate.
    #[test]
    fn garbage_never_panics_and_every_session_is_reached() {
        use proptest::test_runner::{Config, TestRunner};
        use std::cell::RefCell;

        // per 600 s sub-window
        let spl_for = |liq: u128| ((U256::from(600u64) << 128usize) / U256::from(liq)).to::<u128>();
        // pools between 1e14 and 1e20 of liquidity, around the 1e15 floor
        let healthy_spl = spl_for(100_000_000_000_000_000_000)..=spl_for(100_000_000_000_000);
        let strategy = (
            prop_oneof![
                4 => 1i128..=1_000_000_000_000i128,
                1 => any::<i128>(),
                1 => -1_000_000i128..=0i128,
            ],
            prop_oneof![
                6 => (NOW - 500_000)..=(NOW + 10),
                1 => Just(0u64),
                1 => any::<u64>(),
            ],
            prop::bool::weighted(0.1),
            -(1i64 << 50)..=(1i64 << 50),
            prop_oneof![
                4 => (-400_000i64..=400_000).prop_map(|t| t * i64::from(TWAP_WINDOW)),
                1 => any::<i64>(),
            ],
            any::<u128>(),
            prop_oneof![4 => healthy_spl, 1 => any::<u128>(), 1 => Just(0u128)],
            prop_oneof![
                3 => Just(ONE_X),
                1 => 500_000_000_000_000_000u128..=4_000_000_000_000_000_000u128,
            ],
            prop::bool::weighted(0.2),
        );
        let sessions = RefCell::new([0usize; 4]);
        let reasons = RefCell::new([0usize; 6]);
        let mut runner = TestRunner::new(Config {
            cases: 3000,
            ..Config::default()
        });
        runner
            .run(
                &strategy,
                |(
                    answer,
                    updated_at,
                    paused,
                    cum_then,
                    cum_delta,
                    spl_then,
                    spl_delta,
                    mult,
                    rebase,
                )| {
                    let w = World::new();
                    let c = w.deploy();
                    w.mock_paused(paused);
                    // rebase: a new multiplier took effect a second after the print
                    let effective_at = if rebase {
                        updated_at.saturating_add(1)
                    } else {
                        0
                    };
                    w.mock_multiplier(mult, effective_at);
                    w.mock_feed_raw(I256::try_from(answer).unwrap(), updated_at);
                    let clamp56 = |v: i64| v.clamp(-(1i64 << 55) + 1, (1i64 << 55) - 1);
                    let at = |k: i64| {
                        I56::try_from(clamp56(cum_then.saturating_add(cum_delta / 3 * k))).unwrap()
                    };
                    let ticks = vec![
                        at(0),
                        at(1),
                        at(2),
                        I56::try_from(clamp56(cum_then.saturating_add(cum_delta))).unwrap(),
                    ];
                    let s0 = U160::from(spl_then);
                    let d = U160::from(spl_delta);
                    let spl = vec![
                        s0,
                        s0.wrapping_add(d),
                        s0.wrapping_add(d).wrapping_add(d),
                        s0.wrapping_add(d).wrapping_add(d).wrapping_add(d),
                    ];
                    w.mock_observe_raw(ticks, spl);

                    let (session, reason, ans, feed_answer, _, twap, _, clamped, pool) =
                        c.state().unwrap();
                    prop_assert!(session <= SESSION_NO_DATA && reason <= REASON_MULTIPLIER_CHANGED);
                    if session == SESSION_LIVE_FEED || session == SESSION_ONCHAIN_TWAP {
                        // Morpho values raw units: per-share answer times the multiplier
                        if let Ok(p) = c.price() {
                            prop_assert_eq!(
                                p,
                                ans * u(MORPHO_SCALE) * U256::from(mult) / U256::from(ONE_X)
                            );
                        }
                    }
                    match session {
                        SESSION_LIVE_FEED => {
                            prop_assert!(answer > 0);
                            prop_assert_eq!(ans, U256::from(answer as u128));
                            prop_assert_eq!(
                                c.latest_answer().unwrap(),
                                I256::try_from(answer).unwrap()
                            );
                        }
                        SESSION_ONCHAIN_TWAP => {
                            // a valid round is never in the future
                            let (lower, upper) =
                                lower_upper(feed_answer, band_for(NOW - updated_at));
                            prop_assert!(ans >= lower && ans <= upper);
                            prop_assert!(ans > U256::ZERO, "a price is never zero");
                            prop_assert_eq!(clamped, twap < lower || twap > upper);
                            prop_assert_eq!(pool, POOL);
                            prop_assert_eq!(c.latest_answer().unwrap(), I256::from_raw(ans));
                        }
                        SESSION_PAUSED => {
                            prop_assert_eq!(ans, U256::ZERO);
                            prop_assert!(matches!(
                                c.latest_round_data(),
                                Err(AfterHoursError::IssuerPaused(_))
                            ));
                        }
                        _ => {
                            prop_assert_eq!(ans, U256::ZERO);
                            // prop_assert! turns its expression into a format string,
                            // so the struct pattern's braces stay outside it.
                            let refused_with_reason = matches!(
                                c.price(),
                                Err(AfterHoursError::NoData(NoData { reason: r })) if r == reason
                            );
                            prop_assert!(
                                refused_with_reason,
                                "price() must refuse with NoData({})",
                                reason
                            );
                        }
                    }
                    sessions.borrow_mut()[session as usize] += 1;
                    if session == SESSION_NO_DATA {
                        reasons.borrow_mut()[reason as usize] += 1;
                    }
                    Ok(())
                },
            )
            .unwrap();
        let s = sessions.into_inner();
        let r = reasons.into_inner();
        println!("sessions reached {s:?} (LIVE, TWAP, PAUSED, NO_DATA); NO_DATA reasons {r:?}");
        for (i, n) in s.iter().enumerate() {
            assert!(*n >= 20, "session {i} reached only {n} times: {s:?}");
        }
        for (code, n) in r.iter().enumerate().skip(1) {
            assert!(
                *n >= 5,
                "NO_DATA reason {code} reached only {n} times: {r:?}"
            );
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(256))]

        #[test]
        fn ratio_is_monotonic_in_the_tick(a in -400_000i32..400_000, b in -400_000i32..400_000) {
            let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
            let ra = tickmath::ratio_q96(lo).unwrap();
            let rb = tickmath::ratio_q96(hi).unwrap();
            prop_assert!(ra <= rb, "1.0001^tick must not decrease: {lo} -> {ra}, {hi} -> {rb}");
        }

        #[test]
        fn price_is_monotonic_in_the_tick(a in -300_000i32..300_000, b in -300_000i32..300_000) {
            let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
            let p = |tick, is0| {
                tickmath::stock_price(tickmath::ratio_q96(tick).unwrap(), is0, 18, 6, 8).unwrap()
            };
            // stock as token1: a higher tick means more stock per quote, a lower price
            prop_assert!(p(lo, false) >= p(hi, false));
            // stock as token0: a higher tick means more quote per stock, a higher price
            prop_assert!(p(lo, true) <= p(hi, true));
        }

        #[test]
        fn closed_market_answer_is_the_pinned_twap_or_the_exact_band_edge(
            tick_offset in -6_000i64..6_000,
            feed in 20_000_000_000u64..50_000_000_000,
            liq_mult in 1u128..50,
            age in 21_601u64..431_999,
        ) {
            let w = World::new();
            let c = w.deploy();
            w.mock_feed(feed as i128, NOW - age);
            let tick = AAPL_TICK + tick_offset;
            w.mock_observe(tick, MIN_LIQUIDITY * liq_mult);
            let (session, reason, ans, _, _, twap, liq, clamped, pool) = c.state().unwrap();
            prop_assert_eq!((session, reason), (SESSION_ONCHAIN_TWAP, REASON_NONE));
            prop_assert_eq!((liq, pool), (MIN_LIQUIDITY * liq_mult, POOL));
            // the TWAP is the pool's price, cross-checked in floating point
            let exact = tickmath::stock_price(tickmath::ratio_q96(tick as i32).unwrap(), false, 18, 6, 8).unwrap();
            prop_assert_eq!(twap, exact);
            let approx = 1e20_f64 / 1.0001_f64.powi(tick as i32);
            let got = twap.to::<u128>() as f64;
            prop_assert!(((got - approx) / approx).abs() < 1e-9, "twap {got} vs float {approx}");
            // the answer is the TWAP inside the band (narrow within the
            // heartbeat), else exactly the edge it crossed
            let (lower, upper) = lower_upper(U256::from(feed), band_for(age));
            if twap < lower {
                prop_assert_eq!((ans, clamped), (lower, true));
            } else if twap > upper {
                prop_assert_eq!((ans, clamped), (upper, true));
            } else {
                prop_assert_eq!((ans, clamped), (twap, false));
            }
            // every price surface agrees with state()
            let (_, rd_answer, ..) = c.latest_round_data().unwrap();
            prop_assert_eq!(rd_answer, I256::from_raw(ans));
            prop_assert_eq!(c.price().unwrap(), ans * u(MORPHO_SCALE));
        }

        #[test]
        fn the_price_is_the_median_sub_window(
            a in -3_000i64..3_000,
            b in -3_000i64..3_000,
            d in -3_000i64..3_000,
        ) {
            let w = World::new();
            let c = w.deploy();
            w.mock_feed(FRIDAY_ANSWER as i128, NOW - 40 * 3600);
            w.mock_observe_ticks([AAPL_TICK + a, AAPL_TICK + b, AAPL_TICK + d], MIN_LIQUIDITY);
            let mut sorted = [a, b, d];
            sorted.sort_unstable();
            let median = i32::try_from(AAPL_TICK + sorted[1]).unwrap();
            let want = tickmath::stock_price(tickmath::ratio_q96(median).unwrap(), false, 18, 6, 8).unwrap();
            let (session, reason, _, _, _, twap, ..) = c.state().unwrap();
            prop_assert_eq!((session, reason), (SESSION_ONCHAIN_TWAP, REASON_NONE));
            prop_assert_eq!(twap, want);
        }

        #[test]
        fn the_venue_is_the_first_observable_pool_never_the_deepest(
            primary_mode in 0u8..3,
            primary_liq in (MIN_LIQUIDITY / 4)..(MIN_LIQUIDITY * 4),
            standby_mode in 0u8..3,
            standby_liq in (MIN_LIQUIDITY / 4)..(MIN_LIQUIDITY * 40),
        ) {
            // mode 0 = observable with the given depth, 1 = reverts "OLD", 2 = wrong shape
            let w = World::new();
            w.mock_pool2(POOL2_TICK, MIN_LIQUIDITY);
            let c = w.deploy_two_pools();
            w.mock_feed(FRIDAY_ANSWER as i128, NOW - 40 * 3600);
            match primary_mode {
                0 => w.mock_observe(AAPL_TICK, primary_liq),
                1 => w.mock_observe_reverts(),
                _ => w.mock_observe_raw(vec![I56::ZERO], vec![U160::ZERO]),
            }
            match standby_mode {
                0 => w.mock_pool2(POOL2_TICK, standby_liq),
                1 => w.mock_pool2_reverts(),
                _ => w.mock_pool2_wrong_shape(),
            }
            let (session, reason, ans, _, _, _, liq, _, pool) = c.state().unwrap();
            let expect = |p: Address, l: u128, price: u64| -> (u8, u8, U256, u128, Address) {
                if l >= MIN_LIQUIDITY {
                    (SESSION_ONCHAIN_TWAP, REASON_NONE, U256::from(price), l, p)
                } else {
                    (SESSION_NO_DATA, REASON_POOL_TOO_THIN, U256::ZERO, l, p)
                }
            };
            let want = if primary_mode == 0 {
                expect(POOL, primary_liq, AAPL_TWAP)
            } else if standby_mode == 0 {
                expect(POOL2, standby_liq, POOL2_TWAP)
            } else {
                (SESSION_NO_DATA, REASON_TWAP_UNAVAILABLE, U256::ZERO, 0, Address::ZERO)
            };
            prop_assert_eq!((session, reason, ans, liq, pool), want);
        }
    }
}
