# TDX security-bars truncation capture -- 2026-09-23

This artifact attributes the `[E2103] response length mismatch: security bar row 0 is
truncated` failures that the downstream reported on 2026-09-22 and again on 2026-09-23 to
the servers rather than to the decoder. It is the measurement the fix in
`magic-tdx-rs` was written against, and it is re-runnable: the walk is a probe against the
public registry, not a one-off instrument.

The claim it backs is a byte count a server sent, which cannot be re-derived after the
fact, so the readings are preserved rather than summarised.

## Capture method

`cargo run -p magic-tdx-rs --example e2103_server_walk --locked --offline -- <code> <category> <count> <market> [ip-filter]`

The example connects to each registry entry in turn, sends
`build_security_bars_packet(category, market, code, 0, count, 0)` through
`send_raw_and_recv`, and prints the raw body's length, its declared row count, its first 16
bytes, and the outcome of `parse_security_bars` on it. It does not use the smart client's
pool, so it reports what each server actually returns rather than what the client chose.

Readings below are the default `600519` / `SH` (`market=1`) walk at `count=5`, plus a
`count=800` re-walk of the two groups it separates.

## The signature

Two groups, same request, same code, same market.

A server that serves:

```text
PRIMARY/国泰君安10 117.34.114.16:7709 body_len=17284 declared_count=800 head=[20 03 D1 B1 34 01 B6 8C CC 01 E6 5D 9C 5D E6 5D] -> OK n=800 first=2023-06-09 last=2026-09-23
```

A server that truncates:

```text
KNOWN/华林6 218.106.92.183:7709 body_len=2 declared_count=800 head=[20 03] -> ERR [E2103] response length mismatch: security bar row 0 is truncated
```

The decisive detail is that **both bodies begin `20 03`** -- little-endian 800, the row
count the healthy response declares. The truncating servers send exactly the two-byte
header of a valid 800-row response and then no row bytes at all. This is a truncated
delivery of the same protocol, not a different one, and it is why the failure survives
every request-shape change: the response is decided before the request parameters matter.

`declares_rows_without_payload` in `net::utils` encodes exactly this reading: a body too
short to hold one row (datetime 4 + four variable-length price diffs + vol 4 + amount 4 =
18 bytes minimum) while declaring a non-zero row count.

## Per-server readings, 2026-09-23

87 walked, 18 reachable, 69 unreachable.

Serving (`body_len=109 declared_count=5 ... -> OK n=5`, and `body_len=17284` at
`count=800`):

| server | address |
| --- | --- |
| 国泰君安8 | 117.34.114.14:7709 |
| 国泰君安9 | 117.34.114.15:7709 |
| 国泰君安10 | 117.34.114.16:7709 |
| 国泰君安11 | 117.34.114.17:7709 |
| 国泰君安12 | 117.34.114.18:7709 |
| 国泰君安13 | 117.34.114.20:7709 |
| 国泰君安14 | 117.34.114.27:7709 |

Truncating (`body_len=2 declared_count=800 head=[20 03]`, E2103 on every one):

| server | address | note |
| --- | --- | --- |
| 杭州联通J2 | 60.12.136.250:7709 | was in `PRIMARY_SERVERS` |
| 华林5 | 218.106.92.182:7709 | was in `PRIMARY_SERVERS` |
| 杭州电信J3 | 218.75.126.9:7709 | was in `PRIMARY_SERVERS` |
| 上海电信Z1 | 180.153.18.170:7709 | was in `PRIMARY_SERVERS` |
| 华林7 | 220.178.55.71:7709 | was in `PRIMARY_SERVERS` |
| 杭州电信J2 | 115.238.56.198:7709 | was in `PRIMARY_SERVERS` |
| 杭州电信J1 | 60.191.117.167:7709 | was in `PRIMARY_SERVERS` |
| 杭州电信J4 | 115.238.90.165:7709 | was in `PRIMARY_SERVERS` |
| 华林6 | 218.106.92.183:7709 | was in `PRIMARY_SERVERS` |
| 安信15 | 59.36.5.11:7709 | was in `PRIMARY_SERVERS` |
| 华林8 | 220.178.55.86:7709 | outside `PRIMARY_SERVERS` |

Every one of the ten entries that made up `PRIMARY_SERVERS` before this capture is in the
truncating group. Two of them -- 华林6 and 安信15 -- were added to that list on
2026-08-05 *because they returned complete K-lines when probed then*. The behaviour
drifts, so a static list will rot again; the client-side handling below is what keeps the
next rotation from reaching a caller as E2103.

Unreachable in this network: 国泰君安7 (117.34.114.13), 国泰君安15 (117.34.114.30),
国泰君安16 (117.34.114.31), and 66 further entries.

## Why the route was 100% dead

`connect_to_any` lands on a `PRIMARY_SERVERS` entry, the handshake succeeds, and the bars
request returns the payload-less body. `parse_security_bars(&body, category)?` raised
E2103, and the `?` propagated it **before** the empty-response rotation could run. Rotation
was gated on `parsed.is_empty()` -- that is, on a response that parsed -- so a response
that could not parse never reached it. Every shape the downstream tried returned the same
two bytes, which is why broadening the shape coverage changed nothing.

## The fix this measurement produced

1. `net::utils::declares_rows_without_payload` -- recognises "declared rows, no row bytes".
2. `net::client::get_security_bars` -- treats that as a **server fault**: block the server,
   rotate to another, bounded at `SERVER_FAULT_ATTEMPTS = 8`. If all 8 attempts hit such a
   server, it returns an `Err` rather than an empty `Vec`, so the failure stays explicit.
   `get_security_bars_all` delegates here, so paging inherits it.
3. `protocol::constants::PRIMARY_SERVERS` -- the seven measured-serving servers above.

Known cost, recorded rather than papered over: all seven are in `117.34.114.0/24`, and no
second segment measured as serving in this round, so the priority group has no
cross-carrier redundancy right now. The previous ten remain in `ALL_KNOWN_SERVERS` as
fallback; a client that hits one blocks and rotates past it, so a recovery re-enters
service without a code change.

## Verification against the deployed server

Before the fix, against the running server (binary built 2026-09-22T23:48), a pinned
`Tdx` `HistoricalBars` call with the supported shape returned:

```text
request_id=claude-hist-tdx-nodates 2026-09-23T05:52:27Z
Code: FailedPrecondition
Message: [E2103] response length mismatch: security bar row 0 is truncated
trailer magic-error-detail-bin -> reason_code=source_precondition_failed retryable=false
```

After rebuilding and redeploying the server at 2026-09-23 14:00:33 +08:00, the same shape
returned admitted records:

| instrument | result |
| --- | --- |
| 688277 Shanghai | `admitted`, `provider=Tdx`, `complete=true`, 5 records 2026-09-16..2026-09-22 |
| 300005 Shenzhen | `complete=true`, 5 records, `source_at=2026-09-22` |
| 600018 Shanghai | `complete=true`, 5 records, `source_at=2026-09-22` |
| 688561 Shanghai | `complete=true`, 5 records, `source_at=2026-09-22` |
| 002780 Shenzhen | `complete=true`, 5 records, `source_at=2026-09-22` |
| 688277 Shanghai `limit=800` | `complete=true`, 800 records |

The server logged no security-bars failure across the verification window.

## What this capture does not settle

The downstream's production path reported `服务端内部错误` with
`reason_code=no_verified_batch`, `retryable=true`. Neither string is emitted by this
service: `no_verified_batch` does not appear in the repository, and the reason codes the
server can emit are the closed set `capability_unadmitted`, `source_precondition_failed`,
`invalid_evidence`, `internal`, `provider_route_exhausted`, `provider_route_stopped`. The
server also wrote nothing at all during the downstream's failure window
(2026-09-23 09:30:31-10:46:16 +08:00), so those calls did not reach this instance's
handler. The origin of that wrapper is on the downstream side of the wire and is not
explained here.
