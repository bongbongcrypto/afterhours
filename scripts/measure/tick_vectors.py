# -*- coding: utf-8 -*-
"""Reference values for src/tickmath.rs, computed with 60-digit decimal
arithmetic so the Rust fixed-point path is checked against something that
shares none of its code. Prints 1.0001^tick in Q96 (truncated) for the ticks
used in the tests, plus the AAPL/USDG price for the pool's layout."""
from decimal import Decimal, ROUND_FLOOR, getcontext

getcontext().prec = 60
Q96 = Decimal(2) ** 96


def ratio(tick):  # token1 raw per token0 raw
    return Decimal("1.0001") ** tick


def floor(x):
    return int(x.to_integral_value(rounding=ROUND_FLOOR))


for tick in [0, 1, -1, 100, -100, 218301, -218301, 443636, -443636, 887271, -887271]:
    print("tick %8d  ratioQ96 = %d" % (tick, floor(ratio(tick) * Q96)))

# AAPL pool: token0 = USDG (6 dec), token1 = AAPL (18 dec), feed 8 dec.
for tick in (218301, 218302):
    px = Decimal(10) ** 26 / (ratio(tick) * Decimal(10) ** 6)
    print("stock=token1 tick %d -> exact %s -> floor %d" % (tick, px, floor(px)))
px = ratio(-218301) * Decimal(10) ** 26 / Decimal(10) ** 6
print("stock=token0 tick -218301 -> floor %d" % floor(px))
