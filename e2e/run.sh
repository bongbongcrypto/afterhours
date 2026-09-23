#!/usr/bin/env bash
# End-to-end run of the real wasm on a local Nitro dev node (Stylus enabled):
# deploy the mocks with forge, deploy + initialize AfterHours with cargo-stylus,
# then walk every session through cast and assert the exact numbers the unit
# tests expect. Also records gas per read. Needs: cast, forge, cargo-stylus,
# python3, jq, and a dev node on $RPC funded for $KEY (nitro-devnode).
set -euo pipefail
trap 'echo "  ABORT at line $LINENO (exit $?)"; exit 1' ERR
RPC="${RPC:-http://127.0.0.1:8547}"
KEY="${KEY:?dev private key}"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"

FRIDAY=33252000000            # $332.52, 8 decimals
ROUND=645
LIVE_MAX_AGE=21600
WINDOW=1800
DEV_BPS=1000
MIN_LIQ=1000000000000000      # 1e15
ANCHOR=432000
HEARTBEAT=86400               # the feed's 24 h heartbeat: the quiet tier
QUIET_BPS=100                 # the narrow band inside it
TICK=218301
EXPECT_TWAP=33096497304       # floor of the exact 33096497304.69 (scripts/measure/tick_vectors.py)
POOL2_TWAP=31482440784        # standby, stock as token0, tick -218801: floor of 31482440784.887
DOWN_TWAP=31482440784         # primary at tick 218801 (-5.3%): the same exact price, the same floor
HELD_TWAP=32118396159         # primary at tick 218601 (-3%): floor of 32118396159.78
MORPHO_SCALE=10000000000000000  # 10^(36 + 6 - 18 - 8)
ONE=1000000000000000000         # uiMultiplier for one share per token of raw balance
AAPL_MULT=1000566080000000000   # AAPL's uiMultiplier on 2026-09-23
DEEP=1600000000000000000        # 1.6e18, the real AAPL 0.05% pool's depth

pass=0; fail=0
expect() { # label actual expected
  if [ "$2" = "$3" ]; then pass=$((pass + 1)); echo "  ok   $1 = $2"
  else fail=$((fail + 1)); echo "  FAIL $1: got '$2' expected '$3'"; fi
}
num() { echo "$1" | sed 's/ .*//'; }             # cast appends "[1e15]" to large numbers
send() { cast send --json --rpc-url "$RPC" --private-key "$KEY" -- "$@" | jq -e '.status == "0x1"' > /dev/null; }
call() { cast call --rpc-url "$RPC" "$@"; }
now() { cast block latest --rpc-url "$RPC" -f timestamp; }
reverts_with() { # label selector-sig cmd...
  local label="$1" sig="$2"; shift 2
  local sel; sel=$(cast sig "$sig")
  local out; if out=$(cast call --rpc-url "$RPC" "$@" 2>&1); then fail=$((fail + 1)); echo "  FAIL $label: did not revert ($out)"; return; fi
  if echo "$out" | grep -qi "${sel#0x}"; then pass=$((pass + 1)); echo "  ok   $label reverts $sig"
  else fail=$((fail + 1)); echo "  FAIL $label: reverted without $sig: $out"; fi
}
pyint() { python3 -c "print($1)"; }
# secondsPerLiquidityCumulativeX128 at [1800, 1200, 600, 0] s ago for three
# 600 s sub-windows of liquidity L1 L2 L3; "dip" = one second out of range.
spl4() {
  python3 - "$@" <<'PY'
import sys
s = 7777777
out = [s]
for arg in sys.argv[1:]:
    if arg.startswith("dip:"):
        deep = int(arg[4:])
        s += (599 << 128) // deep + (1 << 128)
    else:
        s += (600 << 128) // int(arg)
    out.append(s)
print("[" + ",".join(str(v) for v in out) + "]")
PY
}
# tickCumulative at [1800, 1200, 600, 0] s ago for three 600 s sub-windows
# averaging ticks T1 T2 T3 (oldest first)
cum4() {
  python3 - "$@" <<'PY'
import sys
c = 1000000
out = [c]
for t in sys.argv[1:]:
    c += int(t) * 600
    out.append(c)
print("[" + ",".join(str(v) for v in out) + "]")
PY
}
# revert data must be InvalidConfig's selector followed by the exact code word
reverts_config() { # label code cmd...
  local label="$1" code="$2"; shift 2
  local want; want="$(cast sig "InvalidConfig(uint8)" | sed 's/^0x//')$(printf '%064x' "$code")"
  local out; if out=$(cast call --rpc-url "$RPC" "$@" 2>&1); then fail=$((fail + 1)); echo "  FAIL $label: did not revert ($out)"; return; fi
  if echo "$out" | tr A-Z a-z | grep -q "$want"; then pass=$((pass + 1)); echo "  ok   $label reverts InvalidConfig($code)"
  else fail=$((fail + 1)); echo "  FAIL $label: expected InvalidConfig($code): $out"; fi
}
# revert data must be the error's selector followed by the exact reason word
reverts_nodata() { # label reason cmd...
  local label="$1" reason="$2"; shift 2
  local want; want="$(cast sig "NoData(uint8)" | sed 's/^0x//')$(printf '%064x' "$reason")"
  local out; if out=$(cast call --rpc-url "$RPC" "$@" 2>&1); then fail=$((fail + 1)); echo "  FAIL $label: did not revert ($out)"; return; fi
  if echo "$out" | tr A-Z a-z | grep -q "$want"; then pass=$((pass + 1)); echo "  ok   $label reverts NoData($reason)"
  else fail=$((fail + 1)); echo "  FAIL $label: expected NoData($reason): $out"; fi
}

echo "== mocks (forge)"
cd "$HERE"
forge build --silent
FEED=$(forge create --json --rpc-url "$RPC" --private-key "$KEY" --broadcast src/Mocks.sol:MockFeed --constructor-args "Robinhood AAPL / USD" 8 | jq -r .deployedTo)
STOCK=$(forge create --json --rpc-url "$RPC" --private-key "$KEY" --broadcast src/Mocks.sol:MockToken --constructor-args "AAPL" 18 | jq -r .deployedTo)
USDG=$(forge create --json --rpc-url "$RPC" --private-key "$KEY" --broadcast src/Mocks.sol:MockToken --constructor-args "USDG" 6 | jq -r .deployedTo)
POOL=$(forge create --json --rpc-url "$RPC" --private-key "$KEY" --broadcast src/Mocks.sol:MockPool --constructor-args "$USDG" "$STOCK" | jq -r .deployedTo)
# a second pool with the stock as token0 (mirrored ticks), shallower
POOL2=$(forge create --json --rpc-url "$RPC" --private-key "$KEY" --broadcast src/Mocks.sol:MockPool --constructor-args "$STOCK" "$USDG" | jq -r .deployedTo)
# a Solidity contract that reads the oracle the way Morpho does (STATICCALL)
CONSUMER=$(forge create --json --rpc-url "$RPC" --private-key "$KEY" --broadcast src/Mocks.sol:OracleConsumer | jq -r .deployedTo)
echo "  feed $FEED stock $STOCK usdg $USDG pool $POOL pool2 $POOL2"
T=$(now)
send "$FEED" "set(uint80,int256,uint256,uint256)" $((ROUND - 1)) 31000000000 $((T - 9000)) $((T - 9000))
send "$FEED" "set(uint80,int256,uint256,uint256)" "$ROUND" "$FRIDAY" "$((T - 120))" "$((T - 120))"
OBS="setObservation(int56[4],uint160[4])"
FLAT=$(cum4 $TICK $TICK $TICK)
SPL_FLOOR=$(spl4 $MIN_LIQ $MIN_LIQ $MIN_LIQ)
send "$POOL" "$OBS" "$FLAT" "$SPL_FLOOR"
# POOL2: mirrored tick 500 lower (a lower price), half the liquidity of POOL
POOL2_FLAT=$(cum4 $((-(TICK + 500))) $((-(TICK + 500))) $((-(TICK + 500))))
send "$POOL2" "$OBS" "$POOL2_FLAT" "$(spl4 $((MIN_LIQ / 2)) $((MIN_LIQ / 2)) $((MIN_LIQ / 2)))"

echo "== deploy AfterHours (cargo stylus)"
cd "$ROOT"
strip() { sed -E 's/\x1b\[[0-9;]*m//g'; }
# REPRODUCIBLE=true runs cargo-stylus' Docker build, the path the mainnet deploy takes by default.
if [ "${REPRODUCIBLE:-false}" = "true" ]; then flags=""; else flags="--no-verify"; fi
if ! cargo stylus deploy --endpoint "$RPC" --private-key "$KEY" $flags > "$HERE/deploy.log" 2>&1; then
  echo "  cargo stylus deploy failed:"; strip < "$HERE/deploy.log" | grep -v Compiling | tail -15; exit 1
fi
strip < "$HERE/deploy.log" | grep -iE "contract size|data fee|deployed code|activat" || true
ADDR=$(strip < "$HERE/deploy.log" | grep -oiE 'deployed code at address:? *0x[0-9a-fA-F]{40}' | grep -oE '0x[0-9a-fA-F]{40}' | tail -1 || true)
test -n "$ADDR" || { echo "no address in deploy output"; exit 1; }
echo "  AfterHours at $ADDR"

echo "== initialize"
reverts_with "read before initialize" "NotInitialized()" "$ADDR" "state()(uint8,uint8,uint256,uint256,uint256,uint256,uint128,bool,address)"
reverts_with "decimals before initialize" "NotInitialized()" "$ADDR" "decimals()(uint8)"
reverts_with "description before initialize" "NotInitialized()" "$ADDR" "description()(string)"
reverts_with "quietTier before initialize" "NotInitialized()" "$ADDR" "quietTier()(uint64,uint64)"
send "$POOL" "setCardinality(uint16)" "$WINDOW"
reverts_config "a primary keeping one observation too few" 16 "$ADDR" "initialize(address,address[],address,uint64,uint32,uint64,uint128,uint64,uint64,uint64)" "$FEED" "[$POOL,$POOL2]" "$STOCK" "$LIVE_MAX_AGE" "$WINDOW" "$DEV_BPS" "$MIN_LIQ" "$ANCHOR" "$HEARTBEAT" "$QUIET_BPS"
send "$POOL" "setCardinality(uint16)" "$((WINDOW + 1))"
send "$ADDR" "initialize(address,address[],address,uint64,uint32,uint64,uint128,uint64,uint64,uint64)" "$FEED" "[$POOL,$POOL2]" "$STOCK" "$LIVE_MAX_AGE" "$WINDOW" "$DEV_BPS" "$MIN_LIQ" "$ANCHOR" "$HEARTBEAT" "$QUIET_BPS"
mapfile -t c < <(call "$ADDR" "config()(bool,address,address,address,address,address,bool,uint8,uint8,uint8,uint64,uint32,uint64,uint128,uint64)")
expect "initialized" "${c[0]}" "true"
expect "quote derived from the pool" "$(echo "${c[5]}" | tr A-Z a-z)" "$(echo "$USDG" | tr A-Z a-z)"
expect "stockIsToken0" "${c[6]}" "false"
expect "decimals" "${c[7]}/${c[8]}/${c[9]}" "8/18/6"
expect "maxAnchorAge" "$(num "${c[14]}")" "$ANCHOR"
mapfile -t qt < <(call "$ADDR" "quietTier()(uint64,uint64)")
expect "quietTier() heartbeat/band" "$(num "${qt[0]}")/$(num "${qt[1]}")" "$HEARTBEAT/$QUIET_BPS"
expect "pools()" "$(call "$ADDR" "pools()(address[])" | tr -d ' ' | tr A-Z a-z)" "$(echo "[$POOL,$POOL2]" | tr A-Z a-z)"
expect "description" "$(call "$ADDR" "description()(string)")" "\"Robinhood AAPL / USD (AfterHours)\""
expect "decimals()" "$(call "$ADDR" "decimals()(uint8)")" "8"
reverts_with "second initialize" "AlreadyInitialized()" "$ADDR" "initialize(address,address[],address,uint64,uint32,uint64,uint128,uint64,uint64,uint64)" "$FEED" "[$POOL,$POOL2]" "$STOCK" "$LIVE_MAX_AGE" "$WINDOW" "$DEV_BPS" "$MIN_LIQ" "$ANCHOR" "$HEARTBEAT" "$QUIET_BPS"

echo "== LIVE_FEED"
mapfile -t r < <(call "$ADDR" "latestRoundData()(uint80,int256,uint256,uint256,uint80)")
expect "roundId" "$(num "${r[0]}")" "$ROUND"
expect "answer = feed" "$(num "${r[1]}")" "$FRIDAY"
expect "updatedAt = feed's" "$(num "${r[3]}")" "$((T - 120))"
expect "price() = answer x 1e16" "$(num "$(call "$ADDR" "price()(uint256)")")" "$(pyint "$FRIDAY * $MORPHO_SCALE")"
expect "latestAnswer()" "$(num "$(call "$ADDR" "latestAnswer()(int256)")")" "$FRIDAY"
expect "latestRound()" "$(num "$(call "$ADDR" "latestRound()(uint256)")")" "$ROUND"
mapfile -t s < <(call "$ADDR" "state()(uint8,uint8,uint256,uint256,uint256,uint256,uint128,bool,address)")
expect "session LIVE_FEED" "${s[0]}/${s[1]}" "0/0"
mapfile -t g < <(call "$ADDR" "getRoundData(uint80)(uint80,int256,uint256,uint256,uint80)" $((ROUND - 1)))
expect "getRoundData(644) forwarded" "$(num "${g[1]}")" "31000000000"
GAS_LIVE=$(cast estimate --rpc-url "$RPC" "$ADDR" "latestRoundData()")
GAS_PRICE_LIVE=$(cast estimate --rpc-url "$RPC" "$ADDR" "price()")

echo "== ONCHAIN_TWAP (feed 40 h old)"
T=$(now)
send "$FEED" "set(uint80,int256,uint256,uint256)" "$ROUND" "$FRIDAY" "$((T - 144000))" "$((T - 144000))"
mapfile -t r < <(call "$ADDR" "latestRoundData()(uint80,int256,uint256,uint256,uint80)")
expect "answer = pool TWAP" "$(num "${r[1]}")" "$EXPECT_TWAP"
expect "startedAt = last print" "$(num "${r[2]}")" "$((T - 144000))"
UPD=$(num "${r[3]}"); NOW2=$(now)
if [ "$UPD" -ge "$((NOW2 - 5))" ] && [ "$UPD" -le "$((NOW2 + 5))" ]; then pass=$((pass + 1)); echo "  ok   updatedAt = now ($UPD)"; else fail=$((fail + 1)); echo "  FAIL updatedAt $UPD vs now $NOW2"; fi
expect "price() = twap x 1e16" "$(num "$(call "$ADDR" "price()(uint256)")")" "$(pyint "$EXPECT_TWAP * $MORPHO_SCALE")"
expect "a Solidity caller gets the same price() (STATICCALL)" "$(num "$(call "$CONSUMER" "priceOf(address)(uint256)" "$ADDR")")" "$(pyint "$EXPECT_TWAP * $MORPHO_SCALE")"
mapfile -t ca < <(call "$CONSUMER" "answerOf(address)(int256,uint256)" "$ADDR")
expect "a Solidity caller gets the same latestRoundData answer" "$(num "${ca[0]}")" "$EXPECT_TWAP"
mapfile -t s < <(call "$ADDR" "state()(uint8,uint8,uint256,uint256,uint256,uint256,uint128,bool,address)")
expect "session ONCHAIN_TWAP" "${s[0]}/${s[1]}" "1/0"
expect "twap reported" "$(num "${s[5]}")" "$EXPECT_TWAP"
expect "window liquidity (median of three sub-windows)" "$(num "${s[6]}")" "$MIN_LIQ"
expect "not clamped" "${s[7]}" "false"
expect "answered by the primary" "$(echo "${s[8]}" | tr A-Z a-z)" "$(echo "$POOL" | tr A-Z a-z)"
GAS_TWAP=$(cast estimate --rpc-url "$RPC" "$ADDR" "latestRoundData()")
GAS_PRICE_TWAP=$(cast estimate --rpc-url "$RPC" "$ADDR" "price()")

echo "== band clamp (pool 18% below the last print)"
send "$POOL" "$OBS" "$(cum4 $((TICK + 2000)) $((TICK + 2000)) $((TICK + 2000)))" "$SPL_FLOOR"
LOWER=$(pyint "$FRIDAY * 9000 // 10000")
mapfile -t s < <(call "$ADDR" "state()(uint8,uint8,uint256,uint256,uint256,uint256,uint128,bool,address)")
expect "answer clamped to lower band" "$(num "${s[2]}")" "$LOWER"
expect "clamped flag" "${s[7]}" "true"
expect "price() clamped" "$(num "$(call "$ADDR" "price()(uint256)")")" "$(pyint "$LOWER * $MORPHO_SCALE")"

lower() { echo "$1" | tr A-Z a-z; }

echo "== a spike inside one sub-window moves nothing (7.4x for ten minutes)"
send "$POOL" "$OBS" "$(cum4 $TICK $((TICK - 20000)) $TICK)" "$SPL_FLOOR"
mapfile -t s < <(call "$ADDR" "state()(uint8,uint8,uint256,uint256,uint256,uint256,uint128,bool,address)")
expect "session ONCHAIN_TWAP" "${s[0]}/${s[1]}" "1/0"
expect "answer = the median sub-window" "$(num "${s[2]}")" "$EXPECT_TWAP"
expect "not clamped" "${s[7]}" "false"

echo "== a move held through two sub-windows is priced"
send "$POOL" "$OBS" "$(cum4 $TICK $((TICK + 300)) $((TICK + 300)))" "$SPL_FLOOR"
mapfile -t s < <(call "$ADDR" "state()(uint8,uint8,uint256,uint256,uint256,uint256,uint128,bool,address)")
expect "answer = the held price (exact)" "$(num "${s[2]}")" "$HELD_TWAP"

echo "== quiet tier: a print younger than the heartbeat holds the pool to +-1% during the regular session"
T=$(now)
send "$FEED" "set(uint80,int256,uint256,uint256)" "$ROUND" "$FRIDAY" "$((T - 25200))" "$((T - 25200))"
send "$POOL" "$OBS" "$(cum4 $((TICK + 500)) $((TICK + 500)) $((TICK + 500)))" "$SPL_FLOOR"
NARROW=$(pyint "$FRIDAY * (10000 - $QUIET_BPS) // 10000")
# the node's clock is real time: the narrow band applies only on a weekday from 14:30 to 20:00 UTC,
# when the US regular session is open in both daylight-saving regimes; at any other hour the wide band does
T=$(now)
SESSION=$(pyint "1 if ($T // 86400 + 3) % 7 < 5 and 52200 <= $T % 86400 < 72000 else 0")
mapfile -t s < <(call "$ADDR" "state()(uint8,uint8,uint256,uint256,uint256,uint256,uint128,bool,address)")
expect "session ONCHAIN_TWAP" "${s[0]}/${s[1]}" "1/0"
if [ "$SESSION" = 1 ]; then
  expect "regular session: answer held at -1% (feed 7 h old)" "$(num "${s[2]}")" "$NARROW"
  expect "clamped flag" "${s[7]}" "true"
else
  expect "outside the regular session: the wide band applies (feed 7 h old)" "$(num "${s[2]}")" "$DOWN_TWAP"
  expect "not clamped" "${s[7]}" "false"
fi
expect "the pool's own -5.3% is still reported" "$(num "${s[5]}")" "$DOWN_TWAP"
T=$(now)
send "$FEED" "set(uint80,int256,uint256,uint256)" "$ROUND" "$FRIDAY" "$((T - HEARTBEAT - 60))" "$((T - HEARTBEAT - 60))"
mapfile -t s < <(call "$ADDR" "state()(uint8,uint8,uint256,uint256,uint256,uint256,uint128,bool,address)")
expect "past the heartbeat the band is +-10%: answer = pool" "$(num "${s[2]}")" "$DOWN_TWAP"
expect "not clamped" "${s[7]}" "false"
T=$(now)
send "$FEED" "set(uint80,int256,uint256,uint256)" "$ROUND" "$FRIDAY" "$((T - 144000))" "$((T - 144000))"
send "$POOL" "$OBS" "$FLAT" "$SPL_FLOOR"

echo "== one second out of range in one sub-window: still priced"
send "$POOL" "$OBS" "$FLAT" "$(spl4 $DEEP dip:$DEEP $DEEP)"
mapfile -t s < <(call "$ADDR" "state()(uint8,uint8,uint256,uint256,uint256,uint256,uint128,bool,address)")
expect "session ONCHAIN_TWAP" "${s[0]}/${s[1]}" "1/0"
expect "median sub-window is a deep one" "$(num "${s[6]}")" "$DEEP"
expect "answer = pool TWAP" "$(num "${s[2]}")" "$EXPECT_TWAP"

echo "== out of range in two of three sub-windows: refuses"
send "$POOL" "$OBS" "$FLAT" "$(spl4 dip:$DEEP dip:$DEEP $DEEP)"
mapfile -t s < <(call "$ADDR" "state()(uint8,uint8,uint256,uint256,uint256,uint256,uint128,bool,address)")
expect "session NO_DATA/pool too thin" "${s[0]}/${s[1]}" "3/2"
reverts_nodata "price with two dips" 2 "$ADDR" "price()(uint256)"
reverts_nodata "the refusal reaches a Solidity caller unchanged" 2 "$CONSUMER" "priceOf(address)(uint256)" "$ADDR"

echo "== the venue is fixed: a standby three times deeper does not take over"
send "$POOL" "$OBS" "$FLAT" "$SPL_FLOOR"
send "$POOL2" "$OBS" "$POOL2_FLAT" "$(spl4 $((MIN_LIQ * 3)) $((MIN_LIQ * 3)) $((MIN_LIQ * 3)))"
mapfile -t s < <(call "$ADDR" "state()(uint8,uint8,uint256,uint256,uint256,uint256,uint128,bool,address)")
expect "session ONCHAIN_TWAP" "${s[0]}/${s[1]}" "1/0"
expect "the primary still answers" "$(lower "${s[8]}")" "$(lower "$POOL")"
expect "at the primary's price" "$(num "${s[2]}")" "$EXPECT_TWAP"

echo "== a thin primary refuses instead of moving venue"
send "$POOL" "$OBS" "$FLAT" "$(spl4 $((MIN_LIQ - 1)) $((MIN_LIQ - 1)) $((MIN_LIQ - 1)))"
mapfile -t s < <(call "$ADDR" "state()(uint8,uint8,uint256,uint256,uint256,uint256,uint128,bool,address)")
expect "session NO_DATA/pool too thin" "${s[0]}/${s[1]}" "3/2"
expect "the refusal names the primary" "$(lower "${s[8]}")" "$(lower "$POOL")"
expect "primary liquidity reported" "$(num "${s[6]}")" "$((MIN_LIQ - 1))"
reverts_nodata "latestRoundData while thin" 2 "$ADDR" "latestRoundData()(uint80,int256,uint256,uint256,uint80)"
reverts_nodata "price while thin" 2 "$ADDR" "price()(uint256)"

echo "== the primary cannot be observed: the standby prices"
send "$POOL" "setRevert(bool)" true
mapfile -t s < <(call "$ADDR" "state()(uint8,uint8,uint256,uint256,uint256,uint256,uint128,bool,address)")
expect "session ONCHAIN_TWAP" "${s[0]}/${s[1]}" "1/0"
expect "answered by the standby" "$(lower "${s[8]}")" "$(lower "$POOL2")"
expect "standby price, stock as token0 (exact)" "$(num "${s[2]}")" "$POOL2_TWAP"
expect "standby liquidity" "$(num "${s[6]}")" "$((MIN_LIQ * 3))"

echo "== no pool can be observed"
send "$POOL2" "setRevert(bool)" true
mapfile -t s < <(call "$ADDR" "state()(uint8,uint8,uint256,uint256,uint256,uint256,uint128,bool,address)")
expect "session NO_DATA/twap unavailable" "${s[0]}/${s[1]}" "3/3"
expect "no pool named" "$(lower "${s[8]}")" "0x0000000000000000000000000000000000000000"
send "$POOL" "setRevert(bool)" false
send "$POOL2" "setRevert(bool)" false
send "$POOL" "$OBS" "$FLAT" "$SPL_FLOOR"

echo "== issuer pause"
send "$STOCK" "setPaused(bool)" true
mapfile -t s < <(call "$ADDR" "state()(uint8,uint8,uint256,uint256,uint256,uint256,uint128,bool,address)")
expect "session PAUSED" "${s[0]}" "2"
reverts_with "latestRoundData while paused" "IssuerPaused()" "$ADDR" "latestRoundData()(uint80,int256,uint256,uint256,uint80)"
mapfile -t g < <(call "$ADDR" "getRoundData(uint80)(uint80,int256,uint256,uint256,uint80)" $((ROUND - 1)))
expect "history still served while paused" "$(num "${g[1]}")" "31000000000"
send "$STOCK" "setPaused(bool)" false

echo "== print older than maxAnchorAge"
T=$(now)
send "$FEED" "set(uint80,int256,uint256,uint256)" "$ROUND" "$FRIDAY" "$((T - ANCHOR - 60))" "$((T - ANCHOR - 60))"
mapfile -t s < <(call "$ADDR" "state()(uint8,uint8,uint256,uint256,uint256,uint256,uint128,bool,address)")
expect "session NO_DATA/anchor stale" "${s[0]}/${s[1]}" "3/4"
reverts_nodata "price while anchor stale" 4 "$ADDR" "price()(uint256)"

echo "== broken feed round"
T=$(now)
send "$FEED" "set(uint80,int256,uint256,uint256)" "$ROUND" 0 "$((T - 60))" "$((T - 60))"
mapfile -t s < <(call "$ADDR" "state()(uint8,uint8,uint256,uint256,uint256,uint256,uint128,bool,address)")
expect "session NO_DATA/feed invalid" "${s[0]}/${s[1]}" "3/1"
reverts_nodata "price with a broken round" 1 "$ADDR" "price()(uint256)"

echo "== share multiplier: price() values raw units, answers stay per share"
T=$(now)
send "$FEED" "set(uint80,int256,uint256,uint256)" "$ROUND" "$FRIDAY" "$((T - 120))" "$((T - 120))"
send "$STOCK" "setMultiplier(uint256,uint256)" "$AAPL_MULT" "$((T - 86400))"
mapfile -t r < <(call "$ADDR" "latestRoundData()(uint80,int256,uint256,uint256,uint80)")
expect "latestRoundData per share (the feed's)" "$(num "${r[1]}")" "$FRIDAY"
expect "price() = feed x 1e16 x multiplier" "$(num "$(call "$ADDR" "price()(uint256)")")" "$(pyint "$FRIDAY * $MORPHO_SCALE * $AAPL_MULT // $ONE")"

echo "== a split after the last print refuses until the feed prints"
T=$(now)
send "$FEED" "set(uint80,int256,uint256,uint256)" "$ROUND" "$FRIDAY" "$((T - 120))" "$((T - 120))"
send "$STOCK" "setMultiplier(uint256,uint256)" "$((2 * ONE))" "$((T - 60))"
mapfile -t s < <(call "$ADDR" "state()(uint8,uint8,uint256,uint256,uint256,uint256,uint128,bool,address)")
expect "session NO_DATA/multiplier changed" "${s[0]}/${s[1]}" "3/5"
reverts_nodata "price after a split" 5 "$ADDR" "price()(uint256)"
T=$(now)
send "$FEED" "set(uint80,int256,uint256,uint256)" "$((ROUND + 1))" "$((FRIDAY / 2))" "$T" "$T"
mapfile -t s < <(call "$ADDR" "state()(uint8,uint8,uint256,uint256,uint256,uint256,uint128,bool,address)")
expect "live again once the feed prints the split price" "${s[0]}/${s[1]}" "0/0"
expect "a raw token is worth what it was before the split" "$(num "$(call "$ADDR" "price()(uint256)")")" "$(pyint "$FRIDAY * $MORPHO_SCALE")"
send "$STOCK" "setMultiplier(uint256,uint256)" "$ONE" 0

echo
echo "gas per read: latestRoundData LIVE=$GAS_LIVE TWAP=$GAS_TWAP | price() LIVE=$GAS_PRICE_LIVE TWAP=$GAS_PRICE_TWAP"
echo "result: $pass passed, $fail failed"
[ "$fail" = 0 ]
