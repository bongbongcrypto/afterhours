// SPDX-License-Identifier: MIT
pragma solidity ^0.8.28;

/// Test doubles for the end-to-end run on a local Nitro dev node. Each one
/// mimics exactly the surface AfterHours reads and lets the scenario script
/// set the answer.

contract MockFeed {
    struct Round { uint80 roundId; int256 answer; uint256 startedAt; uint256 updatedAt; uint80 answeredInRound; }
    Round public latest;
    mapping(uint80 => Round) public rounds;
    string private desc;
    uint8 private dec;

    constructor(string memory description_, uint8 decimals_) { desc = description_; dec = decimals_; }

    function decimals() external view returns (uint8) { return dec; }
    function description() external view returns (string memory) { return desc; }
    function version() external pure returns (uint256) { return 4; }

    function set(uint80 roundId, int256 answer, uint256 startedAt, uint256 updatedAt) external {
        latest = Round(roundId, answer, startedAt, updatedAt, roundId);
        rounds[roundId] = latest;
    }

    function latestRoundData() external view returns (uint80, int256, uint256, uint256, uint80) {
        return (latest.roundId, latest.answer, latest.startedAt, latest.updatedAt, latest.answeredInRound);
    }

    function getRoundData(uint80 roundId) external view returns (uint80, int256, uint256, uint256, uint80) {
        Round memory r = rounds[roundId];
        require(r.updatedAt != 0, "No data present");
        return (r.roundId, r.answer, r.startedAt, r.updatedAt, r.answeredInRound);
    }
}

contract MockPool {
    address public token0;
    address public token1;
    int56[4] private cum;
    uint160[4] private spl;
    bool private revertObserve;

    constructor(address token0_, address token1_) { token0 = token0_; token1 = token1_; }

    /// `cum_` and `spl_` are the tickCumulative and
    /// secondsPerLiquidityCumulativeX128 at the four points AfterHours asks
    /// for (oldest first): each sub-window's mean tick is
    /// (cum[k+1] - cum[k]) / span and its harmonic liquidity
    /// span * 2^128 / (spl[k+1] - spl[k]).
    function setObservation(int56[4] calldata cum_, uint160[4] calldata spl_) external {
        cum = cum_; spl = spl_; revertObserve = false;
    }

    function setRevert(bool on) external { revertObserve = on; }

    function observe(uint32[] calldata secondsAgos)
        external view returns (int56[] memory tickCumulatives, uint160[] memory secondsPerLiquidityCumulativeX128s)
    {
        require(!revertObserve, "OLD");
        // AfterHours asks for [window, 2w/3, w/3, 0]; any other shape fails here.
        require(
            secondsAgos.length == 4 && secondsAgos[3] == 0 && secondsAgos[2] > 0
                && secondsAgos[1] > secondsAgos[2] && secondsAgos[0] > secondsAgos[1],
            "shape"
        );
        tickCumulatives = new int56[](4);
        secondsPerLiquidityCumulativeX128s = new uint160[](4);
        for (uint256 i = 0; i < 4; i++) {
            tickCumulatives[i] = cum[i];
            secondsPerLiquidityCumulativeX128s[i] = spl[i];
        }
    }
}

contract MockToken {
    uint8 private dec;
    bool public oraclePaused;
    string public symbol;
    /// Robinhood's scaled-UI surface: one raw unit is uiMultiplier / 1e18 shares.
    uint256 public uiMultiplier = 1e18;
    uint256 public effectiveAt;

    constructor(string memory symbol_, uint8 decimals_) { symbol = symbol_; dec = decimals_; }

    function decimals() external view returns (uint8) { return dec; }
    function setPaused(bool on) external { oraclePaused = on; }
    function setMultiplier(uint256 multiplier, uint256 effectiveAt_) external {
        uiMultiplier = multiplier; effectiveAt = effectiveAt_;
    }
}
