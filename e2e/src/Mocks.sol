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
    int56 private cumThen;
    int56 private cumNow;
    uint160 private splThen;
    uint160 private splNow;
    bool private revertObserve;

    constructor(address token0_, address token1_) { token0 = token0_; token1 = token1_; }

    /// The mean tick over the window is (cumNow - cumThen) / window; the harmonic
    /// liquidity is window * 2^128 / (splNow - splThen).
    function setObservation(int56 cumThen_, int56 cumNow_, uint160 splThen_, uint160 splNow_) external {
        cumThen = cumThen_; cumNow = cumNow_; splThen = splThen_; splNow = splNow_; revertObserve = false;
    }

    function setRevert(bool on) external { revertObserve = on; }

    function observe(uint32[] calldata secondsAgos)
        external view returns (int56[] memory tickCumulatives, uint160[] memory secondsPerLiquidityCumulativeX128s)
    {
        require(!revertObserve, "OLD");
        // AfterHours must ask for [window, 0]; a reversed or extra request fails here.
        require(secondsAgos.length == 2 && secondsAgos[0] > 0 && secondsAgos[1] == 0, "shape");
        tickCumulatives = new int56[](2);
        secondsPerLiquidityCumulativeX128s = new uint160[](2);
        tickCumulatives[0] = cumThen; tickCumulatives[1] = cumNow;
        secondsPerLiquidityCumulativeX128s[0] = splThen; secondsPerLiquidityCumulativeX128s[1] = splNow;
    }
}

contract MockToken {
    uint8 private dec;
    bool public oraclePaused;
    string public symbol;

    constructor(string memory symbol_, uint8 decimals_) { symbol = symbol_; dec = decimals_; }

    function decimals() external view returns (uint8) { return dec; }
    function setPaused(bool on) external { oraclePaused = on; }
}
