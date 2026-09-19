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
    ///      print, updatedAt = block.timestamp. Reverts IssuerPaused / NoData.
    function latestRoundData()
        external
        view
        returns (uint80 roundId, int256 answer, uint256 startedAt, uint256 updatedAt, uint80 answeredInRound);

    // ---- Morpho Blue IOracle -----------------------------------------------
    /// @dev Quote-token value of one raw stock unit, scaled by 1e36.
    function price() external view returns (uint256);

    // ---- AfterHours ---------------------------------------------------------
    /// @dev session: 0 LIVE_FEED, 1 ONCHAIN_TWAP, 2 PAUSED, 3 NO_DATA
    ///      reason (NO_DATA only): 1 feed invalid, 2 pool too thin, 3 TWAP unavailable
    ///      answer: the price the oracle stands behind (0 when it refuses)
    ///      twap: raw pool TWAP before the band (0 outside ONCHAIN_TWAP)
    ///      clamped: the TWAP was pulled back to the edge of the band
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
            bool clamped
        );

    function config()
        external
        view
        returns (
            bool initialized,
            address initializer,
            address feed,
            address pool,
            address stock,
            address quote,
            bool stockIsToken0,
            uint8 feedDecimals,
            uint8 stockDecimals,
            uint8 quoteDecimals,
            uint64 liveMaxAge,
            uint32 twapWindow,
            uint64 maxDeviationBps,
            uint128 minLiquidity
        );

    /// @dev One-shot configuration, run by the deployment script right after activation.
    function initialize(
        address feed,
        address pool,
        address stock,
        uint64 liveMaxAge,
        uint32 twapWindow,
        uint64 maxDeviationBps,
        uint128 minLiquidity
    ) external;

    error IssuerPaused();
    error NoData(uint8 reason);
    error CallFailed(address target);
    error InvalidConfig(uint8 reason);
    error AlreadyInitialized();
    error NotInitialized();
}
