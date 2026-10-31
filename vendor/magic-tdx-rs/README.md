# magic-tdx-rs

Pure-Rust TDX source driver.

## Stable service facades

The crate exposes typed service entry points in addition to the low-level
clients. All network calls are read-only and return the upstream error type.

| Facade | Coverage |
| --- | --- |
| `TdxService` | Smart failover K-lines with atomic exact pagination beyond the 800-row wire limit, quotes, chunked quotes, normalized Shanghai/Shenzhen security metadata, securities, minute/trade history, finance and XDXR |
| `AsyncTdxService` | Async-pool equivalents, including atomic historical-bar and security-list pagination |
| `BlockService` | Industry, concept, index blocks, block K-lines and quotes |
| `FundService` | Fund/ETF list, bars, quotes, finance and XDXR |
| `FinanceService` | Realtime finance, report files/records, 45 named indicators |
| `ProfileService` | F10 categories, named sections, all sections and complete payloads |

Construct a facade without connecting; configure/connect its underlying client
explicitly before making live requests. For a read-only smoke test covering all
protocol families, run:

```text
cargo run -p magic-tdx-rs --example live_probe --release
```

The probe prints source-backed security name/ST/board evidence and also prints
unavailable listing-date, price-limit-rule, and source-time fields explicitly.
It verifies Beijing market `2` Quote, bars, books, normalized minute data and
current trades. Beijing security metadata is explicitly unsupported because
live servers close the market-2 security-list request; it is never remapped to
Shanghai or Shenzhen.

## Strict packet boundary

TDX binary responses are decoded atomically. A declared batch is returned only
after every declared record and its protocol-specific tail have been checked.
Truncated fixed fields, unterminated variable integers, and missing later
records return `TdxError` with
`ErrorCode::RESPONSE_LENGTH_MISMATCH`; they are never converted into zero-valued
fields or a shorter successful batch.

The public low-level helpers `constants::{get_byte, read_u16, read_u32,
read_f32, read_i32, read_i64}` and `helpers::get_price` return `Result` as of
0.2.0. This is an intentional pre-1.0 safety break: callers must propagate or
handle decoder failure instead of relying on the previous panic/zero fallback.
Valid wire-encoded zero values still decode successfully as zero.

## Production board Provider

`TdxBoardProvider` implements provider-neutral board directory, exact
constituents and reverse membership from the verified `block_fg.dat` and
`block_gn.dat` files. It rejects unsupported categories, malformed or
unverified code prefixes, duplicate source pairs, duplicate reverse requests
and empty normalized output. Provider-scoped board IDs use
`tdx:<industry|concept>:<source name>` and retain `source_at=None` because the
block packet supplies no timestamp.

The block files carry stock codes but not stock names. Applications that display
both use `magic_market_router::join_board_membership_names` with a
`SecurityMetadataProvider` batch. The returned `NamedBoardMembership` retains
TDX evidence for board membership and separate metadata evidence for the stock
name; missing names or identity mismatch fail rather than substituting the code
as a fake name.

```text
MAGIC_TDX_BOARD_SERVER=... \
MAGIC_TDX_BOARD_CATEGORY=concept \
MAGIC_TDX_BOARD_NAME=人工智能 \
cargo run -p magic-tdx-rs --example board_live_probe --release --locked --offline
```
