#!/usr/bin/env bash
# The deployment, step by step. The manual `deploy` workflow runs these steps
# against Robinhood Chain; e2e/run.sh runs the same steps against a local Nitro
# node on every push, so the path the mainnet deploy takes is exercised before
# it is needed.
#
#   bash scripts/deploy.sh preflight   # every read initialize will make, before any gas is spent
#   bash scripts/deploy.sh deploy      # cargo stylus deploy + activate
#   bash scripts/deploy.sh initialize  # the one-shot initialize; fails on a reverted receipt
#   bash scripts/deploy.sh verify      # reads the configuration back; fails unless every field matches
#   bash scripts/deploy.sh market      # optional: a Morpho Blue market priced by the new oracle
#
# Inputs come from the environment: RPC, DEPLOYER_KEY, FEED, POOLS (comma-
# separated, primary first), STOCK, EXPECTED_DESCRIPTION, EXPECTED_QUOTE,
# EXPECTED_DECIMALS (feed/stock/quote), LIVE_MAX_AGE, TWAP_WINDOW,
# MAX_DEVIATION_BPS, MIN_LIQUIDITY, MAX_ANCHOR_AGE, HEARTBEAT, QUIET_BAND_BPS;
# REPRODUCIBLE (deploy); ADDR (after deploy); DEPLOYER (verify);
# MORPHO_BLUE, MORPHO_IRM, MORPHO_LLTV (market). preflight prints
# `deployer=0x...` and deploy prints `address=0x...`, also into $GITHUB_OUTPUT
# when it is set.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
POOLS=$(printf '%s' "${POOLS:-}" | tr -d '[:space:]')   # "a, b" and "a,b" are the same list
lower() { tr '[:upper:]' '[:lower:]'; }
num() { echo "$1" | sed 's/ .*//'; }   # cast appends "[2e17]" to large numbers
out() { echo "$1"; if [ -n "${GITHUB_OUTPUT:-}" ]; then echo "$1" >> "$GITHUB_OUTPUT"; fi; }

preflight() {
  local desc role pool sub card addr bal
  desc=$(cast call --rpc-url "$RPC" "$FEED" "description()(string)")
  echo "feed description: $desc"
  test "$desc" = "\"$EXPECTED_DESCRIPTION\"" || { echo "feed is not $EXPECTED_DESCRIPTION"; exit 1; }
  cast call --rpc-url "$RPC" "$FEED" "latestRoundData()(uint80,int256,uint256,uint256,uint80)"
  role=primary
  for pool in ${POOLS//,/ }; do
    echo "$role pool $pool"
    cast call --rpc-url "$RPC" "$pool" "token0()(address)"
    cast call --rpc-url "$RPC" "$pool" "token1()(address)"
    # the four points every read asks for: three sub-windows of the TWAP window
    sub=$((TWAP_WINDOW / 3))
    cast call --rpc-url "$RPC" "$pool" "observe(uint32[])(int56[],uint160[])" "[$TWAP_WINDOW,$((2 * sub)),$sub,0]"
    # slot0: sqrtPriceX96, tick, index, cardinality, cardinalityNext, feeProtocol, unlocked
    card=$(cast call --rpc-url "$RPC" "$pool" "slot0()(uint160,int24,uint16,uint16,uint16,uint8,bool)" | sed -n '4p' | sed 's/ .*//')
    echo "  observation cardinality $card"
    # Uniswap writes at most one observation per second, so a pool holding
    # fewer than window + 1 can be made to answer "OLD" by one write a
    # second. The primary must not be; a standby may (it only covers).
    if [ "$role" = primary ] && [ "$card" -lt $((TWAP_WINDOW + 1)) ]; then
      echo "primary cardinality $card < $((TWAP_WINDOW + 1)). Raise it first (anyone can, gas only):"
      echo "  cast send $pool 'increaseObservationCardinalityNext(uint16)' $((TWAP_WINDOW + 1))"
      echo "then re-run once slot0 shows the new cardinality."
      exit 1
    fi
    role=standby
  done
  # the feed's, the stock's and the quote's decimals, and the stock's pause flag:
  # initialize reads every one of them (the share multiplier is not read: the
  # feed already prices one token of raw balance)
  cast call --rpc-url "$RPC" "$FEED" "decimals()(uint8)"
  cast call --rpc-url "$RPC" "$STOCK" "decimals()(uint8)"
  cast call --rpc-url "$RPC" "$EXPECTED_QUOTE" "decimals()(uint8)"
  cast call --rpc-url "$RPC" "$STOCK" "oraclePaused()(bool)"
  addr=$(cast wallet address --private-key "$DEPLOYER_KEY")
  bal=$(cast balance --rpc-url "$RPC" "$addr")
  echo "deployer $addr balance $bal wei"
  test "$bal" != "0" || { echo "deployer has no gas"; exit 1; }
  out "deployer=$addr"
}

deploy() {
  local flags fee log addr
  # Reproducible (Docker) builds let `cargo stylus verify` / the explorer match the source.
  if [ "${REPRODUCIBLE:-false}" = "true" ]; then flags=""; else flags="--no-verify"; fi
  # cargo-stylus sets maxFeePerGas to the base fee it has just read, so a base fee
  # one step higher by the time the tx lands is refused (mainnet 2026-09-25:
  # maxFeePerGas 35854000 < baseFee 36026000, nothing sent). Cap at twice the
  # current base fee, the headroom `cast send` uses; the chain charges the base fee.
  fee=$(cast base-fee --rpc-url "$RPC")
  flags="$flags --max-fee-per-gas-gwei $(awk -v w="$fee" 'BEGIN { printf "%.9f", 2 * w / 1e9 }')"
  log="${DEPLOY_LOG:-$ROOT/deploy.log}"
  cargo stylus deploy --endpoint "$RPC" --private-key "$DEPLOYER_KEY" $flags 2>&1 | tee "$log"
  # cargo-stylus colours the address (ANSI codes sit between "address:" and "0x").
  addr=$(sed -E 's/\x1b\[[0-9;]*m//g' "$log" | grep -oiE 'deployed code at address:? *0x[0-9a-fA-F]{40}' | grep -oE '0x[0-9a-fA-F]{40}' | tail -1 || true)
  test -n "$addr" || { echo "no address in cargo stylus output"; exit 1; }
  sha256sum target/wasm32-unknown-unknown/release/afterhours_oracle.wasm || true
  out "address=$addr"
}

initialize() {
  local receipt
  receipt=$(cast send --json --rpc-url "$RPC" --private-key "$DEPLOYER_KEY" "$ADDR" \
    "initialize(address,address[],address,uint64,uint32,uint64,uint128,uint64,uint64,uint64)" \
    "$FEED" "[$POOLS]" "$STOCK" "$LIVE_MAX_AGE" "$TWAP_WINDOW" "$MAX_DEVIATION_BPS" "$MIN_LIQUIDITY" "$MAX_ANCHOR_AGE" \
    "$HEARTBEAT" "$QUIET_BAND_BPS")
  echo "$receipt" | jq -r '"initialize tx \(.transactionHash) status \(.status)"'
  echo "$receipt" | jq -e '.status == "0x1"' > /dev/null || { echo "initialize reverted"; exit 1; }
}

verify() {
  local cfg c tier qt got desc blk st want
  echo "AfterHours at $ADDR"
  cfg=$(cast call --rpc-url "$RPC" "$ADDR" "config()(bool,address,address,address,address,address,bool,uint8,uint8,uint8,uint64,uint32,uint64,uint128,uint64)")
  echo "$cfg"
  # cast prints one value per line, in order
  mapfile -t c <<< "$cfg"
  test "${c[0]}" = "true" || { echo "not initialized"; exit 1; }
  test "$(echo "${c[2]}" | lower)" = "$(echo "$FEED" | lower)" || { echo "feed mismatch"; exit 1; }
  test "$(echo "${c[3]}" | lower)" = "$(echo "${POOLS%%,*}" | lower)" || { echo "primary pool mismatch"; exit 1; }
  got=$(cast call --rpc-url "$RPC" "$ADDR" "pools()(address[])" | tr -d ' ' | lower)
  test "$got" = "$(echo "[$POOLS]" | lower)" || { echo "pools mismatch: $got"; exit 1; }
  test "$(echo "${c[4]}" | lower)" = "$(echo "$STOCK" | lower)" || { echo "stock mismatch"; exit 1; }
  test "$(echo "${c[1]}" | lower)" = "$(echo "$DEPLOYER" | lower)" || { echo "initializer is not our deployer: ${c[1]}"; exit 1; }
  test "$(echo "${c[5]}" | lower)" = "$(echo "$EXPECTED_QUOTE" | lower)" || { echo "quote token mismatch: ${c[5]}"; exit 1; }
  test "${c[7]}/${c[8]}/${c[9]}" = "$EXPECTED_DECIMALS" || { echo "decimals mismatch: ${c[7]}/${c[8]}/${c[9]}"; exit 1; }
  test "$(num "${c[10]}")" = "$LIVE_MAX_AGE" || { echo "liveMaxAge mismatch"; exit 1; }
  test "$(num "${c[11]}")" = "$TWAP_WINDOW" || { echo "twapWindow mismatch"; exit 1; }
  test "$(num "${c[12]}")" = "$MAX_DEVIATION_BPS" || { echo "maxDeviationBps mismatch"; exit 1; }
  test "$(num "${c[13]}")" = "$MIN_LIQUIDITY" || { echo "minLiquidity mismatch"; exit 1; }
  test "$(num "${c[14]}")" = "$MAX_ANCHOR_AGE" || { echo "maxAnchorAge mismatch"; exit 1; }
  tier=$(cast call --rpc-url "$RPC" "$ADDR" "quietTier()(uint64,uint64)")
  mapfile -t qt <<< "$tier"
  test "$(num "${qt[0]}")" = "$HEARTBEAT" || { echo "heartbeat mismatch"; exit 1; }
  test "$(num "${qt[1]}")" = "$QUIET_BAND_BPS" || { echo "quietBandBps mismatch"; exit 1; }
  desc=$(cast call --rpc-url "$RPC" "$ADDR" "description()(string)")
  test "$desc" = "\"$EXPECTED_DESCRIPTION (AfterHours)\"" || { echo "description mismatch: $desc"; exit 1; }
  cast call --rpc-url "$RPC" "$ADDR" "decimals()(uint8)"
  cast call --rpc-url "$RPC" "$ADDR" "state()(uint8,uint8,uint256,uint256,uint256,uint256,uint128,bool,address)"
  # While it answers, price() must be the answer times Morpho's scale and nothing else:
  # the feed prices one token of raw balance, so no share multiplier. One block for both.
  blk=$(cast block-number --rpc-url "$RPC")
  mapfile -t st < <(cast call --rpc-url "$RPC" --block "$blk" "$ADDR" "state()(uint8,uint8,uint256,uint256,uint256,uint256,uint128,bool,address)")
  if [ "${st[0]}" = 0 ] || [ "${st[0]}" = 1 ]; then
    want=$(python3 -c "print($(num "${st[2]}") * 10 ** (36 + ${c[9]} - ${c[8]} - ${c[7]}))")
    got=$(num "$(cast call --rpc-url "$RPC" --block "$blk" "$ADDR" "price()(uint256)")")
    test "$got" = "$want" || { echo "price() $got is not answer x 10^(36 + quote - stock - feed decimals) = $want"; exit 1; }
    echo "price() = answer x 10^$((36 + c[9] - c[8] - c[7])) at block $blk"
  fi
  # PAUSED / NO_DATA are legal at deploy time (weekend, thin pool); state() above says why.
  cast call --rpc-url "$RPC" "$ADDR" "latestRoundData()(uint80,int256,uint256,uint256,uint80)" \
    || echo "latestRoundData() reverted: the oracle is refusing to price right now (see state())"
  echo "all fields verified"
}

market() {
  local params id receipt
  # Market creation on Morpho Blue is permissionless once the IRM and LLTV are enabled;
  # loan token = the pool's quote (USDG), collateral = the stock, oracle = this deployment.
  cast call --rpc-url "$RPC" "$MORPHO_BLUE" "isIrmEnabled(address)(bool)" "$MORPHO_IRM" | grep -q true || { echo "IRM not enabled"; exit 1; }
  cast call --rpc-url "$RPC" "$MORPHO_BLUE" "isLltvEnabled(uint256)(bool)" "$MORPHO_LLTV" | grep -q true || { echo "LLTV not enabled"; exit 1; }
  params="($EXPECTED_QUOTE,$STOCK,$ADDR,$MORPHO_IRM,$MORPHO_LLTV)"
  id=$(cast keccak "$(cast abi-encode "f((address,address,address,address,uint256))" "$params")")
  echo "market id $id"
  receipt=$(cast send --json --rpc-url "$RPC" --private-key "$DEPLOYER_KEY" "$MORPHO_BLUE" \
    "createMarket((address,address,address,address,uint256))" "$params")
  echo "$receipt" | jq -r '"createMarket tx \(.transactionHash) status \(.status)"'
  echo "$receipt" | jq -e '.status == "0x1"' > /dev/null || { echo "createMarket reverted"; exit 1; }
  cast call --rpc-url "$RPC" "$MORPHO_BLUE" "idToMarketParams(bytes32)(address,address,address,address,uint256)" "$id"
  if [ -n "${GITHUB_STEP_SUMMARY:-}" ]; then echo "MORPHO_MARKET_ID=$id" >> "$GITHUB_STEP_SUMMARY"; fi
}

case "${1:-}" in
  preflight | deploy | initialize | verify | market) "$1" ;;
  *) echo "usage: bash scripts/deploy.sh preflight|deploy|initialize|verify|market" >&2; exit 2 ;;
esac
