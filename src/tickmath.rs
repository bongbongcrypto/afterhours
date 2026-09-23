//! Fixed-point 1.0001^tick, the Uniswap v3 tick-to-price conversion, in Q96.
//!
//! Uniswap's own TickMath library reaches the same number through a table of
//! twenty magic constants. This module instead raises 1.0001 (held in Q96) to
//! the tick by square-and-multiply with 512-bit intermediates and needs no
//! constants that could be mistyped. For tick >= 0 the relative error stays
//! below 1e-23 over the whole range; for tick < 0 the result is the inverted
//! Q96 value truncated to an integer, exact to 1 ulp (2.2e-20 relative at
//! -218301, the AAPL region; only below about -650000 does truncation bite,
//! far outside any stock/USDG pool). Reference values in the tests are the
//! verbatim output of `scripts/measure/tick_vectors.py`, which computes them
//! independently with 80-digit decimal arithmetic and truncates.

use alloy_primitives::{U256, U512};

/// Uniswap v3's tick bounds: prices of 2^-128 .. 2^128 per raw unit.
pub const MAX_TICK: i32 = 887_272;

/// 2^96, the Q96 representation of 1.
pub const ONE_Q96: U256 = U256::from_limbs([0, 1 << 32, 0, 0]);

/// 1.0001 in Q96, i.e. 2^96 + 2^96 / 10_000 (truncated; error below 1 ulp).
fn base_q96() -> U256 {
    ONE_Q96 + ONE_Q96 / U256::from(10_000u64)
}

/// (a * b) / 2^96 with a 512-bit intermediate, so the product never overflows.
fn mul_q96(a: U256, b: U256) -> U256 {
    let wide: U512 = a.widening_mul(b);
    (wide >> 96usize).to::<U256>()
}

/// 1.0001^tick as a Q96 ratio (token1 raw units per token0 raw unit).
///
/// Returns `None` for ticks outside the Uniswap range or when the ratio would
/// underflow to zero, so callers can fail closed instead of dividing by zero.
pub fn ratio_q96(tick: i32) -> Option<U256> {
    if !(-MAX_TICK..=MAX_TICK).contains(&tick) {
        return None;
    }
    let mut result = ONE_Q96;
    let mut base = base_q96();
    let mut exp = tick.unsigned_abs();
    while exp > 0 {
        if exp & 1 == 1 {
            result = mul_q96(result, base);
        }
        exp >>= 1;
        if exp > 0 {
            base = mul_q96(base, base);
        }
    }
    if tick < 0 {
        // 1 / x in Q96 = 2^96 * 2^96 / x_q96
        let numerator: U512 = U512::from(ONE_Q96) << 96usize;
        let inverted = (numerator / U512::from(result)).to::<U256>();
        if inverted.is_zero() {
            return None;
        }
        return Some(inverted);
    }
    Some(result)
}

/// Time-weighted mean tick from two Uniswap `tickCumulative` samples, rounded
/// toward negative infinity exactly as Uniswap's OracleLibrary does.
pub fn mean_tick(cumulative_then: i64, cumulative_now: i64, window: u32) -> Option<i32> {
    if window == 0 {
        return None;
    }
    let delta = cumulative_now.checked_sub(cumulative_then)?;
    let window = i64::from(window);
    let mut mean = delta / window;
    if delta < 0 && delta % window != 0 {
        mean -= 1;
    }
    i32::try_from(mean).ok()
}

/// Price of one whole stock token in quote units, scaled by 10^feed_decimals.
///
/// `ratio` is token1-per-token0 in Q96. When the stock is token1 the ratio is
/// stock-per-quote and has to be inverted; when it is token0 the ratio is
/// already quote-per-stock.
pub fn stock_price(
    ratio: U256,
    stock_is_token0: bool,
    stock_decimals: u8,
    quote_decimals: u8,
    feed_decimals: u8,
) -> Option<U256> {
    if ratio.is_zero() {
        return None;
    }
    let ten = U512::from(10u64);
    let numerator_pow = ten.pow(U512::from(
        u64::from(stock_decimals) + u64::from(feed_decimals),
    ));
    let quote_pow = ten.pow(U512::from(u64::from(quote_decimals)));
    let ratio = U512::from(ratio);
    let price = if stock_is_token0 {
        // quote_raw per stock_raw = ratio / 2^96
        (ratio * numerator_pow) / (quote_pow << 96usize)
    } else {
        // stock_raw per quote_raw = ratio / 2^96  ->  invert
        (numerator_pow << 96usize) / (ratio * quote_pow)
    };
    if price.is_zero() || price > U512::from(U256::MAX) {
        return None;
    }
    Some(price.to::<U256>())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u(s: &str) -> U256 {
        U256::from_str_radix(s, 10).unwrap()
    }

    // Reference ratios, verbatim from scripts/measure/tick_vectors.py (80-digit decimals, truncated).
    const VECTORS: &[(i32, &str)] = &[
        (0, "79228162514264337593543950336"),
        (1, "79236085330515764027303304731"),
        (-1, "79220240490215316061937756560"),
        (100, "80024378775772204256025656562"),
        (-100, "78439868342809377387252074392"),
        (218301, "239385339738159338582485095309052680650"),
        (-218301, "26221746671089383567"),
        (443636, "1461446703485210103244672773810124308346321380902"),
        (-443636, "4295128738"),
        (
            887271,
            "26955224481606593845066778340113370614845985437494375745877179886964",
        ),
    ];

    fn assert_close(actual: U256, expected: U256, tolerance_ppb: u64) {
        let diff = if actual > expected {
            actual - expected
        } else {
            expected - actual
        };
        // diff / expected <= tolerance / 1e9
        assert!(
            diff * U256::from(1_000_000_000u64) <= expected * U256::from(tolerance_ppb),
            "actual {actual} expected {expected}"
        );
    }

    #[test]
    fn ratio_matches_reference_values() {
        for (tick, expected) in VECTORS {
            let actual = ratio_q96(*tick).expect("in range");
            // 10 parts per billion; the reference itself is truncated to an integer
            assert_close(actual, u(expected), 10);
        }
    }

    #[test]
    fn exact_at_tick_zero() {
        assert_eq!(ratio_q96(0), Some(ONE_Q96));
    }

    #[test]
    fn out_of_range_and_underflow_fail_closed() {
        assert_eq!(ratio_q96(MAX_TICK + 1), None);
        assert_eq!(ratio_q96(-MAX_TICK - 1), None);
        assert_eq!(ratio_q96(-887271), None, "ratio underflows to zero");
        assert!(ratio_q96(MAX_TICK).is_some());
    }

    #[test]
    fn mean_tick_rounds_toward_negative_infinity() {
        assert_eq!(mean_tick(0, 1800, 1800), Some(1));
        assert_eq!(mean_tick(0, 1799, 1800), Some(0));
        assert_eq!(mean_tick(0, -1, 1800), Some(-1));
        assert_eq!(mean_tick(0, -1800, 1800), Some(-1));
        assert_eq!(mean_tick(0, -1801, 1800), Some(-2));
        assert_eq!(mean_tick(100, 100 + 218301 * 1800, 1800), Some(218301));
        assert_eq!(mean_tick(0, 0, 0), None);
    }

    #[test]
    fn stock_price_for_the_aapl_pool_layout() {
        // AAPL/USDG 0.05% pool: token0 = USDG (6 dec), token1 = AAPL (18 dec), feed 8 dec.
        let ratio = ratio_q96(218301).unwrap();
        let price = stock_price(ratio, false, 18, 6, 8).unwrap();
        assert_close(price, u("33096497304"), 10); // floor of $330.964973046...
        let price = stock_price(ratio_q96(218302).unwrap(), false, 18, 6, 8).unwrap();
        assert_close(price, u("33093187985"), 10);
    }

    #[test]
    fn stock_price_when_stock_is_token0() {
        let ratio = ratio_q96(-218301).unwrap();
        let price = stock_price(ratio, true, 18, 6, 8).unwrap();
        assert_close(price, u("33096497304"), 10);
        assert_eq!(
            stock_price(ONE_Q96, true, 18, 6, 8),
            Some(u("100000000000000000000"))
        );
    }

    #[test]
    fn stock_price_rejects_zero_ratio() {
        assert_eq!(stock_price(U256::ZERO, false, 18, 6, 8), None);
    }
}
