# Legacy provider-host contract snapshot

`market.proto` is copied from the repository's tracked
`contracts/local_bridge_v1/market.proto` snapshot (2026-09-05). The old
`grpc_market_server` built from commit `62f84326` reserves operation IDs
61 and 62 for local `ChainBatch` and `BenchmarkBars` RPCs. The newer ignored
`client-bundle/market.proto` reused those IDs and added response fields that
this old server does not initialize. Pinning this snapshot makes a recovery
build reproducible without changing the current upstream contract.

The build script continues to add the local RPC declarations that are absent
from this snapshot. Do not replace this file with a newer upstream proto
without reconciling the operation IDs and generated response types.
