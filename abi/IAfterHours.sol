// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

/// @title AfterHours - a 24/7 price for a tokenized stock on Robinhood Chain
/// @notice Drop-in replacement for a Chainlink stock feed that keeps answering
///         while the US market is closed. Implements Chainlink's
///         AggregatorV3Interface and Morpho Blue's IOracle.
interface IAfterHours {
    // ---- Chainlink AggregatorV3Interface ----------------------------------
    function decimals() external view returns (uint8);
    function description() external view returns (string memory);
    function version() external view returns (uint256);
    /// @dev LIVE_FEED: the feed's round verbatim.
    ///      ONCHAIN_TWAP: answer = bounded pool TWAP, startedAt = last exchange
    ///      print, updatedAt = block.timestamp, roundId = the feed's (it does not
    ///      advance between closed-market reads). Reverts IssuerPaused / NoData.
    function latestRoundData()
        external
        view
        returns (uint80 roundId, int256 answer, uint256 startedAt, uint256 updatedAt, uint80 answeredInRound);
    /// @dev Historical rounds are forwarded to the feed (also while the oracle is
    ///      PAUSED / NO_DATA); the feed's current round is answered exactly like
    ///      latestRoundData().
    function getRoundData(uint80 roundId)
        external
        view
        returns (uint80, int256, uint256, uint256, uint80);

    // ---- Chainlink AggregatorInterface (v2 getters) ------------------------
    function latestAnswer() external view returns (int256);
    function latestTimestamp() external view returns (uint256);
    function latestRound() external view returns (uint256);

    // ---- Morpho Blue IOracle -----------------------------------------------
    /// @dev Quote-token value of one raw stock unit, scaled by 1e36.
    function price() external view returns (uint256);

    // ---- AfterHours ---------------------------------------------------------
    /// @dev session: 0 LIVE_FEED, 1 ONCHAIN_TWAP, 2 PAUSED, 3 NO_DATA
    ///      reason (NO_DATA only): 1 feed invalid, 2 pool too thin, 3 TWAP unavailable,
    ///      4 last feed print older than maxAnchorAge
    ///      answer: the price the oracle stands behind (0 when it refuses)
    ///      twap: raw pool TWAP before the band (0 outside ONCHAIN_TWAP)
    ///      liquidity: harmonic-mean in-range liquidity over the TWAP window of the pool used
    ///      clamped: the TWAP was pulled back to the edge of the band
    ///      pool: the pool that answered (zero outside ONCHAIN_TWAP / pool refusals)
    function state()
        external
        view
        returns (
            uint8 session,
            uint8 reason,
            uint256 answer,
            uint256 feedAnswer,
            uint256 feedUpdatedAt,
            uint256 twap,
            uint128 liquidity,
            bool clamped,
            address pool
        );

    /// @dev The configured pools (1-3), in the order given to initialize.
    function pools() external view returns (address[] memory);

    function config()
        external
        view
        returns (
            bool initialized,
            address initializer,
            address feed,
            address firstPool,
            address stock,
            address quote,
            bool stockIsToken0OfFirstPool,
            uint8 feedDecimals,
            uint8 stockDecimals,
            uint8 quoteDecimals,
            uint64 liveMaxAge,
            uint32 twapWindow,
            uint64 maxDeviationBps,
            uint128 minLiquidity,
            uint64 maxAnchorAge
        );

    /// @dev One-shot configuration, run by the deployment script right after activation.
    ///      Reverts InvalidConfig(reason): 1 liveMaxAge 0, 2 twapWindow 0, 3 band not in
    ///      (0, 10000), 4 minLiquidity 0, 5 stock not in pool, 6 Morpho scale underflow,
    ///      7 decimals > 36, 8 maxAnchorAge <= liveMaxAge, 9 twapWindow > 1 day,
    ///      10 a pool does not answer observe([twapWindow, 0]), 11 not 1-3 pools,
    ///      12 the pools do not share one quote token.
    function initialize(
        address feed,
        address[] calldata pools,
        address stock,
        uint64 liveMaxAge,
        uint32 twapWindow,
        uint64 maxDeviationBps,
        uint128 minLiquidity,
        uint64 maxAnchorAge
    ) external;

    error IssuerPaused();
    error NoData(uint8 reason);
    error CallFailed(address target);
    error InvalidConfig(uint8 reason);
    error AlreadyInitialized();
    error NotInitialized();
}
