# Recorded HistoricalBars test fixtures

These are offline `TEST_CODE` parser fixtures extracted from the public Windows
handoff `windows-evidence-20261002.2/historical/`. They are not new acquisitions
or Gateway captures. The receipts' original limits **15 / 15 / 1** remain intact;
none is relabeled as the exact-window Gateway's 11-trading-date limit.

The complete source receipt hashes are retained in each fixture. The delivery
manifest SHA-256 is
`e93488ff6443e8c5171592858203053ea5d394625178560b6913b5ee5808d059`.
All 66 delivery files, the manifest digest, all record data hashes and the known
QueryResponse reencoding hashes were checked before extraction.

| Fixture | Complete source receipt SHA-256 | Observed rows |
| --- | --- | --- |
| bars-688277 | `ea8da734a8967ff70d942f2ca96e9252f2fb0a189b72c06cc91beb8aaa3a35d2` | 1 |
| bars-688561 | `60a262b44f3fc3b9f80ed981fa3d8d0706a154a037cebee5ca9d03b4fd0e4670` | 11 |
| bars-688561-limit1 | `86d19e8debfbb938c85a6517aa914e781593da7093e7408108bba5daf964850c` | 1 |

`data_utf8` is the original UTF-8 record data; hashes bind those bytes, not the
decoded convenience JSON or a parser reserialization. The retained protobuf
is explicitly **prost-reencoded known fields**, not original HTTP/2 wire or
native upstream HTTP bodies. Request JSON is the receipt's parsed request view;
its separately retained payload digest is not a claim that this view is the
original request bytes. Health/Capabilities are in the complete source receipts.

All three calls observed the unchanged production service 0.2.0, source
`67c832e43f36f188e4d769f409691c0b1d9a2ea2`, descriptor
`abf28a3e0028488a7579da4d961e1a7c1408482bdc0500122c1956d225e480cf`, binary
`9302a3036c7ed272fdf24dd84b4a66881ab682649e9af76b68ef62b2ad083854`.
The successor public bundle has `deployment_build_identity=null` and supplies
no successor deployment evidence.

The underlying row provider is Tonghuashun; the outer provider is HithinkFinance.
Volumes are fractional lots of 100 shares; amounts are CNY. Row source dates and
batch source timestamps are not publication/correction instants. All three
responses carry `complete=true`, including the limit 1 truncation control.
Missing dates remain Unknown. Eleven observed dates matching a calendar vector
still do not certify coverage, native identity, source revisions, or PIT.
