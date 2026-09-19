# Deployments

| network | chain id | asset | address | initialize params | tx |
|---|---|---|---|---|---|
| (none yet) | | | | | |

Inputs used for AAPL on Robinhood Chain mainnet (4663):

| input | value |
|---|---|
| feed (Chainlink AAPL/USD) | `0x6B22A786bAa607d76728168703a39Ea9C99f2cD0` |
| pool (Uniswap v3 AAPL/USDG 0.05%) | `0xaae0d815ee56e4092a5e5c2911e676fea50b2d6d` |
| stock (AAPL) | `0xaF3D76f1834A1d425780943C99Ea8A608f8a93f9` |
| quote (read from the pool) | USDG `0x5fc5360D0400a0Fd4f2af552ADD042D716F1d168` |
| liveMaxAge | 21600 |
| twapWindow | 1800 |
| maxDeviationBps | 1000 |
| minLiquidity | 200000000000000000 |
