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
TICK=218301
EXPECT_TWAP=33096497304       # floor of the exact 33096497304.69 (scripts/measure/tick_vectors.py)
MORPHO_SCALE=10000000000000000  # 10^(36 + 6 - 18 - 8)

pass=0; fail=0
expect() { # label actual expected
  if [ "$2" = "$3" ]; then pass=$((pass + 1)); echo "  ok   $1 = $2"
  else fail=$((fail + 1)); echo "  FAIL $1: got '$2' expected '$3'"; fi
}
num() { echo "$1" | sed 's/ .*//'; }             # cast appends "[1e15]" to large numbers
send() { cast send --json --rpc-url "$RPC" --private-key "$KEY" "$@" | jq -e '.status == "0x1"' > /dev/null; }
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
spl_delta() { pyint "($WINDOW << 128) // $1"; }

echo "== mocks (forge)"
cd "$HERE"
forge build --silent
FEED=$(forge create --json --rpc-url "$RPC" --private-key "$KEY" --broadcast src/Mocks.sol:MockFeed --constructor-args "Robinhood AAPL / USD" 8 | jq -r .deployedTo)
STOCK=$(forge create --json --rpc-url "$RPC" --private-key "$KEY" --broadcast src/Mocks.sol:MockToken --constructor-args "AAPL" 18 | jq -r .deployedTo)
USDG=$(forge create --json --rpc-url "$RPC" --private-key "$KEY" --broadcast src/Mocks.sol:MockToken --constructor-args "USDG" 6 | jq -r .deployedTo)
POOL=$(forge create --json --rpc-url "$RPC" --private-key "$KEY" --broadcast src/Mocks.sol:MockPool --constructor-args "$USDG" "$STOCK" | jq -r .deployedTo)
echo "  feed $FEED stock $STOCK usdg $USDG pool $POOL"
T=$(now)
send "$FEED" "set(uint80,int256,uint256,uint256)" $((ROUND - 1)) 31000000000 $((T - 9000)) $((T - 9000))
send "$FEED" "set(uint80,int256,uint256,uint256)" "$ROUND" "$FRIDAY" "$((T - 120))" "$((T - 120))"
CUM_THEN=1000000
CUM_NOW=$((CUM_THEN + TICK * WINDOW))
SPL_THEN=7777777
SPL_NOW=$(pyint "$SPL_THEN + $(spl_delta $MIN_LIQ)")
send "$POOL" "setObservation(int56,int56,uint160,uint160)" "$CUM_THEN" "$CUM_NOW" "$SPL_THEN" "$SPL_NOW"

echo "== deploy AfterHours (cargo stylus)"
cd "$ROOT"
strip() { sed -E 's/\x1b\[[0-9;]*m//g'; }
if ! cargo stylus deploy --endpoint "$RPC" --private-key "$KEY" --no-verify > "$HERE/deploy.log" 2>&1; then
  echo "  cargo stylus deploy failed:"; strip < "$HERE/deploy.log" | grep -v Compiling | tail -15; exit 1
fi
strip < "$HERE/deploy.log" | grep -iE "contract size|data fee|deployed code|activat" || true
ADDR=$(strip < "$HERE/deploy.log" | grep -oiE 'deployed code at address:? *0x[0-9a-fA-F]{40}' | grep -oE '0x[0-9a-fA-F]{40}' | tail -1 || true)
test -n "$ADDR" || { echo "no address in deploy output"; exit 1; }
echo "  AfterHours at $ADDR"

echo "== initialize"
reverts_with "read before initialize" "NotInitialized()" "$ADDR" "state()(uint8,uint8,uint256,uint256,uint256,uint256,uint128,bool)"
send "$ADDR" "initialize(address,address,address,uint64,uint32,uint64,uint128,uint64)" "$FEED" "$POOL" "$STOCK" "$LIVE_MAX_AGE" "$WINDOW" "$DEV_BPS" "$MIN_LIQ" "$ANCHOR"
mapfile -t c < <(call "$ADDR" "config()(bool,address,address,address,address,address,bool,uint8,uint8,uint8,uint64,uint32,uint64,uint128,uint64)")
expect "initialized" "${c[0]}" "true"
expect "quote derived from the pool" "$(echo "${c[5]}" | tr A-Z a-z)" "$(echo "$USDG" | tr A-Z a-z)"
expect "stockIsToken0" "${c[6]}" "false"
expect "decimals" "${c[7]}/${c[8]}/${c[9]}" "8/18/6"
expect "maxAnchorAge" "$(num "${c[14]}")" "$ANCHOR"
expect "description" "$(call "$ADDR" "description()(string)")" "\"Robinhood AAPL / USD (AfterHours)\""
expect "decimals()" "$(call "$ADDR" "decimals()(uint8)")" "8"
if cast send --json --rpc-url "$RPC" --private-key "$KEY" "$ADDR" "initialize(address,address,address,uint64,uint32,uint64,uint128,uint64)" "$FEED" "$POOL" "$STOCK" "$LIVE_MAX_AGE" "$WINDOW" "$DEV_BPS" "$MIN_LIQ" "$ANCHOR" 2>/dev/null | jq -e '.status == "0x1"' > /dev/null 2>&1; then
  fail=$((fail + 1)); echo "  FAIL second initialize succeeded"
else pass=$((pass + 1)); echo "  ok   second initialize rejected"; fi

echo "== LIVE_FEED"
mapfile -t r < <(call "$ADDR" "latestRoundData()(uint80,int256,uint256,uint256,uint80)")
expect "roundId" "$(num "${r[0]}")" "$ROUND"
expect "answer = feed" "$(num "${r[1]}")" "$FRIDAY"
expect "updatedAt = feed's" "$(num "${r[3]}")" "$((T - 120))"
expect "price() = answer x 1e16" "$(num "$(call "$ADDR" "price()(uint256)")")" "$(pyint "$FRIDAY * $MORPHO_SCALE")"
expect "latestAnswer()" "$(num "$(call "$ADDR" "latestAnswer()(int256)")")" "$FRIDAY"
expect "latestRound()" "$(num "$(call "$ADDR" "latestRound()(uint256)")")" "$ROUND"
mapfile -t s < <(call "$ADDR" "state()(uint8,uint8,uint256,uint256,uint256,uint256,uint128,bool)")
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
mapfile -t s < <(call "$ADDR" "state()(uint8,uint8,uint256,uint256,uint256,uint256,uint128,bool)")
expect "session ONCHAIN_TWAP" "${s[0]}/${s[1]}" "1/0"
expect "twap reported" "$(num "${s[5]}")" "$EXPECT_TWAP"
expect "window liquidity" "$(num "${s[6]}")" "$MIN_LIQ"
expect "not clamped" "${s[7]}" "false"
GAS_TWAP=$(cast estimate --rpc-url "$RPC" "$ADDR" "latestRoundData()")
GAS_PRICE_TWAP=$(cast estimate --rpc-url "$RPC" "$ADDR" "price()")

echo "== band clamp (pool 18% below the last print)"
send "$POOL" "setObservation(int56,int56,uint160,uint160)" "$CUM_THEN" "$((CUM_THEN + (TICK + 2000) * WINDOW))" "$SPL_THEN" "$SPL_NOW"
LOWER=$(pyint "$FRIDAY * 9000 // 10000")
mapfile -t s < <(call "$ADDR" "state()(uint8,uint8,uint256,uint256,uint256,uint256,uint128,bool)")
expect "answer clamped to lower band" "$(num "${s[2]}")" "$LOWER"
expect "clamped flag" "${s[7]}" "true"
expect "price() clamped" "$(num "$(call "$ADDR" "price()(uint256)")")" "$(pyint "$LOWER * $MORPHO_SCALE")"

echo "== thin pool over the window"
send "$POOL" "setObservation(int56,int56,uint160,uint160)" "$CUM_THEN" "$CUM_NOW" "$SPL_THEN" "$(pyint "$SPL_THEN + $(spl_delta $((MIN_LIQ - 1)))")"
mapfile -t s < <(call "$ADDR" "state()(uint8,uint8,uint256,uint256,uint256,uint256,uint128,bool)")
expect "session NO_DATA/pool too thin" "${s[0]}/${s[1]}" "3/2"
expect "liquidity reported" "$(num "${s[6]}")" "$((MIN_LIQ - 1))"
reverts_with "latestRoundData while thin" "NoData(uint8)" "$ADDR" "latestRoundData()(uint80,int256,uint256,uint256,uint80)"
reverts_with "price while thin" "NoData(uint8)" "$ADDR" "price()(uint256)"

echo "== pool without history"
send "$POOL" "setRevert(bool)" true
mapfile -t s < <(call "$ADDR" "state()(uint8,uint8,uint256,uint256,uint256,uint256,uint128,bool)")
expect "session NO_DATA/twap unavailable" "${s[0]}/${s[1]}" "3/3"
send "$POOL" "setRevert(bool)" false
send "$POOL" "setObservation(int56,int56,uint160,uint160)" "$CUM_THEN" "$CUM_NOW" "$SPL_THEN" "$SPL_NOW"

echo "== issuer pause"
send "$STOCK" "setPaused(bool)" true
mapfile -t s < <(call "$ADDR" "state()(uint8,uint8,uint256,uint256,uint256,uint256,uint128,bool)")
expect "session PAUSED" "${s[0]}" "2"
reverts_with "latestRoundData while paused" "IssuerPaused()" "$ADDR" "latestRoundData()(uint80,int256,uint256,uint256,uint80)"
mapfile -t g < <(call "$ADDR" "getRoundData(uint80)(uint80,int256,uint256,uint256,uint80)" $((ROUND - 1)))
expect "history still served while paused" "$(num "${g[1]}")" "31000000000"
send "$STOCK" "setPaused(bool)" false

echo "== print older than maxAnchorAge"
T=$(now)
send "$FEED" "set(uint80,int256,uint256,uint256)" "$ROUND" "$FRIDAY" "$((T - ANCHOR - 60))" "$((T - ANCHOR - 60))"
mapfile -t s < <(call "$ADDR" "state()(uint8,uint8,uint256,uint256,uint256,uint256,uint128,bool)")
expect "session NO_DATA/anchor stale" "${s[0]}/${s[1]}" "3/4"
reverts_with "price while anchor stale" "NoData(uint8)" "$ADDR" "price()(uint256)"

echo "== broken feed round"
T=$(now)
send "$FEED" "set(uint80,int256,uint256,uint256)" "$ROUND" 0 "$((T - 60))" "$((T - 60))"
mapfile -t s < <(call "$ADDR" "state()(uint8,uint8,uint256,uint256,uint256,uint256,uint128,bool)")
expect "session NO_DATA/feed invalid" "${s[0]}/${s[1]}" "3/1"

echo
echo "gas per read: latestRoundData LIVE=$GAS_LIVE TWAP=$GAS_TWAP | price() LIVE=$GAS_PRICE_LIVE TWAP=$GAS_PRICE_TWAP"
echo "result: $pass passed, $fail failed"
[ "$fail" = 0 ]
