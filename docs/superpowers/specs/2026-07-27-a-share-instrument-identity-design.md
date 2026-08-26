# Canonical Current China-Listed Equity and BSE Historical-Alias Design

## 1. Gate A decision and scope

BR-173 introduces one deep identity boundary for six-digit China-listed
equities. Although this file retains the requested “A-share” filename, the
domain type is deliberately named **China-listed equity identity** because
`900xxx` and `200xxx` are B shares. Treating either as an A share would be a
data-identity defect.

This Gate A slice changes documentation only. The future implementation must:

1. resolve exact six-digit equity prefixes in one module;
2. preserve exchange and A/B share class as separate typed facts;
3. validate authoritative provider market evidence without allowing it to
   relabel an incompatible code;
4. resolve a retired Beijing code only through the exact official old/new
   mapping, never through a legacy prefix;
5. keep provider/capability support separate from identity validity;
6. fail closed for unsupported or contradictory identities; and
7. preserve production/test isolation.

Funds, bonds, indices, options, futures, Hong Kong instruments and provider
protocol symbols such as `sh600000` are out of scope. They require their own
asset-class resolver and must not pass through this equity function.

Triggered repository rules are AGENTS 2.1, 2.2, 2.3, 2.5, 2.7 and 2.10.
There is no threshold or `config/*.toml` change.

## 2. Verified inconsistency

The current repository has no single identity owner. At the inspected state:

- `historical_bars.rs`, `market_data.rs`, `review.rs`, `company.rs`,
  `market_capabilities.rs`, `capital.rs`, `sina_instrument_news.rs` and
  `security_lifecycle.rs` map first character `9` to Shanghai and first
  characters `4/8` to Beijing;
- `consensus.rs` maps every first character `9` to Beijing;
- `board.rs` and `research.rs` accept `4/8` as Beijing but reject `9`;
- `magic_tdx.rs` accepts only first character `6` as Shanghai and `0/3` as
  Shenzhen before calling a provider that itself has a typed Beijing market;
- `review.rs` tests currently accept broad `400xxx` and `800xxx` Beijing
  identities; and
- several error strings call every accepted equity an “A-share”, including
  the `900xxx` Shanghai B-share range.

These mappings cannot all be correct at the same time. They also allow a
consumer to obtain a different `InstrumentId` solely by choosing a different
Gateway. They are additionally inconsistent with the current official Beijing
identity: the effective code-compilation guidance assigns listed-company
ordinary shares to the complete `92xxxx` range, while `83xxxx`, `87xxxx` and
`88xxxx` identify NEEQ listed companies. The 2025-09-12 BSE notice requires
all existing listed shares to use their switched codes from 2025-10-09.

The pinned upstream revision inspected for this design is
`b2b68df78156df1d67824e5c44c0cb01b752f55a`. Its Core `InstrumentId`
validates basic string construction but does not validate six-digit segment,
exchange compatibility or A/B share class. Therefore constructing an
`InstrumentId` is not identity admission.

## 3. Considered approaches

### A. One canonical resolver — selected

One module owns syntax, exact current segment mapping, official historical
alias resolution, share class, test namespace, provider-market compatibility
and typed failure. Gateways receive a validated identity and keep only
capability-specific protocol formatting.

This is the smallest interface with the greatest information hiding. A prefix
change or newly admitted segment has one owner and one test matrix.

### B. Shared table with mapping retained in every Gateway — rejected

A common constant would reduce duplicated literals but leave validation
order, provider conflict handling, A/B classification and test stripping
distributed. The present divergence would reappear around the shared table.

### C. Require provider market for every identity — rejected

Many requests must choose a route before a provider response exists. Some
capabilities also do not return a separately verifiable market. Making
provider evidence mandatory would turn valid storage identities into an
availability dependency and encourage fallback inference at call sites.

## 4. Current canonical and historical-alias contract

The resolver checks the complete registered segment, never only the first
character. Shanghai and Shenzhen entries use three digits; current Beijing
listed-company ordinary shares use the complete first-two-digit `92` range.

| Current code segment | Exchange | Share class | Canonical segment |
| --- | --- | --- | --- |
| `600`, `601`, `603`, `605` | Shanghai | A | Shanghai main-board equity |
| `688`, `689` | Shanghai | A | STAR-market equity |
| `900` | Shanghai | B | Shanghai B-share equity |
| `000`, `001`, `002`, `003` | Shenzhen | A | Shenzhen main-board equity |
| `300`, `301` | Shenzhen | A | ChiNext equity |
| `200` | Shenzhen | B | Shenzhen B-share equity |
| `92xxxx` | Beijing | A | Current BSE listed-company ordinary equity |

All other prefixes are rejected as `UnsupportedEquityPrefix`. In particular:

- every six-digit code beginning with `92` is a valid current Beijing equity
  identity;
- provider capabilities proven only for `920xxx` do not thereby support
  `921xxx` through `929xxx`;
- `900xxx` is Shanghai B share and must never become Beijing;
- every other non-`92` 9-leading code is rejected unless it is the registered
  Shanghai `900xxx` B-share segment;
- broad first-character `4` or `8` acceptance is forbidden; and
- a code beginning `43`, `83`, `87` or `88` is never a current BSE identity
  merely because an older provider or test called it Beijing.

An old Beijing code is accepted only through a separately typed
`HistoricalAlias` operation. The operation requires an exact row from the BSE
[new/old code mapping table](https://www.bse.cn/service/code_mapping.html),
including the old code, the current `92xxxx` code and immutable source
evidence. Its result keeps the old code only as alias evidence and returns the
new `92xxxx` code as canonical identity.

Without that exact row:

- `43xxxx`, `83xxxx`, `87xxxx` and `88xxxx` are
  `UnsupportedHistoricalAlias`;
- `83xxxx`, `87xxxx` and `88xxxx` must not be promoted because the current
  official rule also assigns those ranges to NEEQ listed-company ordinary
  shares; and
- provider exchange `Beijing` alone is insufficient to establish a current
  BSE listing.

The table defines canonical identity, not a promise that every provider can
serve every segment.

## 5. Deep interface

The planned module is `src/data_gateway/instrument_identity.rs`. Its public
surface should be equivalent to:

```rust
pub struct CanonicalEquityIdentity {
    storage_code: SixDigitEquityCode,
    instrument: InstrumentId,
    share_class: EquityShareClass,
    segment: EquitySegment,
    historical_alias: Option<HistoricalAliasEvidence>,
    resolution: IdentityResolutionEvidence,
}

pub enum EquityShareClass {
    A,
    B,
}

pub enum ProviderMarketEvidence {
    Absent,
    Verified {
        provider: ProviderId,
        capability: &'static str,
        exchange: Exchange,
        batch_id: String,
        item_id: Option<String>,
        observed_at: String,
    },
}

pub fn resolve_production_equity(
    storage_code: &str,
    market: ProviderMarketEvidence,
) -> Result<CanonicalEquityIdentity, EquityIdentityError>;

pub fn resolve_historical_bse_alias(
    old_code: &str,
    official_mapping: OfficialBseCodeMappingEvidence,
    market: ProviderMarketEvidence,
) -> Result<CanonicalEquityIdentity, EquityIdentityError>;
```

The concrete names may change during Gate B, but the information boundary may
not. `CanonicalEquityIdentity` is the only input accepted by equity request
builders. Its `InstrumentId` is private output constructed after validation;
callers cannot construct a partially validated identity.

An A-share-only consumer calls an explicit constraint such as
`identity.require_a_share(capability)`. A B-share identity then returns
`UnsupportedShareClass`, not a mislabeled A-share `InstrumentId`.

Provider capability admission is a second operation:

```text
canonical identity
  -> provider/capability support registry
  -> Supported(request route) | Unsupported(evidence gap)
```

This prevents “valid security” from being confused with “this endpoint is
verified for the security.”

The normal production request path accepts only a current canonical code.
`resolve_historical_bse_alias` is for migration, historical replay and
provider-returned legacy records; it cannot silently turn an old storage key
into a current request. Its output canonical code is the mapped `92xxxx`
value, while the old code and official mapping identity remain immutable
evidence.

## 6. Resolution and evidence precedence

Resolution is deterministic:

1. select production or test namespace before touching the code;
2. validate non-empty input, exact namespace form, six ASCII digits and
   `AssetClass::Equity`;
3. map a current code through the exact segment table in section 4;
4. for a historical alias, require an exact official mapping row whose old
   code equals the input and whose new code is a current `92xxxx` identity;
5. if authoritative provider market evidence is present, validate its
   provider, capability, batch/item identity and observation time;
6. require the provider exchange to equal the canonical exchange;
7. construct the Core `InstrumentId`, share class and segment; and
8. record whether the accepted exchange was provider-verified,
   canonical-segment-resolved or official-historical-alias-resolved.

Authoritative provider market evidence outranks an unverified hint: it becomes
the recorded resolution source. It does **not** have permission to rewrite the
canonical segment. A provider returning Beijing for `900001`, Shanghai for
`921001`, or a different code produces `ProviderMarketConflict`. A provider
returning `830001` as Beijing still does not prove a current BSE identity:
the record must carry an official `830001 -> 92xxxx` mapping or remain
unsupported. Both facts are retained in the rejection audit; neither silently
wins.

A provider name, endpoint selection, URL prefix or consumer-selected route is
not verified market evidence. Provider evidence is authoritative only when it
comes from the admitted record/batch contract for the same item.

## 7. Test/live isolation

Production resolution rejects any `TEST_CODE_` input before normalization.
Test resolution is available only in test builds and accepts only
`TEST_CODE_` followed by a canonical six-digit code. Test mode must reject a
bare real symbol.

If a provider test fixture needs a protocol code:

1. validate the `TEST_CODE_` namespace;
2. resolve the six-digit suffix through the same canonical table;
3. send only the suffix to the fake/test provider boundary;
4. compare returned code and exchange with the expected test identity; and
5. restore the test namespace only in the isolated test projection.

No production function may call `strip_prefix("TEST_CODE_")`. This identity
guard supplements, and does not replace, the physical test/live account,
database and audit isolation required by AGENTS 2.5.

## 8. Verified provider support

Provider support below is intentionally capability-specific. “Unsupported”
means that the inspected pinned revision does not provide enough concrete
evidence to admit the combination; it is not filled by inference.

| Provider evidence at pinned revision | Admitted conclusion | Unsupported / unresolved |
| --- | --- | --- |
| Magic TDX maps typed Shanghai/Shenzhen/Beijing to protocol markets `1/0/2`; source comments record live `(2, 920118)` identity for quote, bars, minute data, trades and books. | Those capabilities may route an already canonical `920xxx` Beijing identity and must still validate the returned identity. | Beijing security metadata and corporate action are explicitly unsupported. Other current `92xxxx` values, both B-share classes and every historical alias remain `Unsupported` per capability until independently admitted. |
| Baidu bars validation maps `6` to Shanghai, `0/3` to Shenzhen, broad `4/8` and specifically `920` to Beijing; tests cover legacy-looking `430001`, `830001`, current `920001` and exact exchange mismatch. | The provider contract contains explicit Beijing bar validation for `920xxx`. The `430/830` tests prove only provider behavior, not current official BSE identity. | `921xxx` through `929xxx`, `900901`, `200xxx` and every old code without an official mapping remain `Unsupported`. Broad `4/8` acceptance must not bypass official alias resolution. |
| Tencent formats an explicit typed exchange as `sh/sz/bj`; tests use `920118` for Beijing quotes/order books. | `920xxx` has concrete protocol/test evidence for those tested capabilities. | The inspected evidence does not establish every capability, `921xxx` through `929xxx`, either B-share class or any historical alias. Those cells remain `Unsupported`. |
| Sina company news explicitly rejects Beijing. | Shanghai/Shenzhen A-share company-news behavior may continue only after canonical identity validation and existing batch admission. | All Beijing company-news identities and both B-share classes are `Unsupported` absent new evidence. |
| Eastmoney parses provider-returned `SH/SZ/BJ` suffixes, while some `secid` endpoint families explicitly reject Beijing; another local helper broadly maps every 9-leading code to Beijing. | An admitted record’s exact `SH/SZ/BJ` suffix is usable provider market evidence. | The broad 9-leading helper is not authoritative and must not resolve identity. Beijing and B-share support remains endpoint-specific and `Unsupported` without a capability proof. |

The support registry must cite a provider test, live-probe artifact or strict
returned-market contract for every newly admitted cell. Adding a prefix to the
canonical table alone cannot enable a provider.

Therefore the initial registry may recognize all `92xxxx` values as valid
current identities while declaring only evidenced `920xxx`
provider/capability cells `Supported`. Identity-valid `921xxx` through
`929xxx` requests fail as `UnsupportedProviderCapability`, not
`UnsupportedEquityPrefix`.

## 9. Data flow

```text
storage/test symbol
  -> namespace guard
  -> current exact segment resolver
     OR exact official old-code -> 92xxxx historical-alias resolver
  -> optional authoritative provider-market compatibility check
  -> CanonicalEquityIdentity(current code + exchange + share class
                             + segment + optional alias evidence)
  -> provider/capability support admission
  -> provider protocol adapter
  -> returned record/batch exact identity validation
  -> consumer projection + acquisition audit
```

Provider-specific formats remain private to provider adapters. Business
modules store the six-digit canonical code and typed identity evidence; they
do not store or reconstruct `sh/sz/bj`, numeric market IDs, `SECUCODE`
suffixes or other provider symbols.

## 10. Failure model

| Failure | Required behavior |
| --- | --- |
| empty, whitespace, wrong length or non-ASCII digit | `InvalidEquityCode`; no provider call |
| `TEST_CODE_` in production | `TestIdentityInProduction`; no normalization or provider call |
| bare real symbol in isolated test mode | `RealIdentityInTest`; no provider call |
| syntactically valid but unregistered prefix | `UnsupportedEquityPrefix`; no first-character fallback |
| `43/83/87/88` old-looking code without an exact official mapping row | `UnsupportedHistoricalAlias`; do not infer BSE from provider market |
| official mapping old/new code, source identity or content conflicts | `OfficialAliasMappingMismatch`; reject the alias and batch |
| provider exchange/code/asset class contradicts canonical identity | `ProviderMarketConflict` or `ReturnedInstrumentMismatch`; reject the item/batch according to its completeness contract |
| A-only consumer receives `200xxx` or `900xxx` | `UnsupportedShareClass`; never relabel as A share |
| provider/capability cell lacks evidence | `UnsupportedProviderCapability`; do not try another hidden route or old provider |
| provider omits market where its source contract requires it | `ProviderMarketMissing`; do not infer from URL or request |
| identity audit append fails | fail closed; no consumer-visible admitted result |

Missing evidence remains missing. None of these failures becomes a default
exchange, an empty successful batch, a warning followed by computation, or a
cross-provider fallback.

## 11. Audit contract

Every identity decision that reaches acquisition audit records:

- namespace and a safe identity hash;
- canonical six-digit code where the authorized business audit permits it;
- exchange, share class and segment;
- resolution source (`provider_verified`, `canonical_segment` or
  `official_historical_alias`);
- old code, new `92xxxx` code, official mapping row identity, source URL,
  observed time and content hash when a historical alias is used;
- provider and capability, if present;
- provider batch/item identity and observed/source time, if present;
- acceptance or structured failure reason; and
- resolver contract version.

Request and returned record must match code, exchange, asset class, share
class and provider/batch evidence. Audit rows remain append-only, test/live
isolated, hash-chain protected and retained at least five years under AGENTS
2.7 and BR-159.

## 12. Existing-module disposition

| Existing module or behavior | Decision |
| --- | --- |
| BR-064 six-digit boundary | adopt and supersede with BR-173’s exact prefix, share-class, evidence and failure contract |
| `magic_market_core::InstrumentId` | adopt only as a post-validation transport value; reject it as an identity validator |
| admitted provider-returned market/exchange | adopt as higher-grade evidence after exact same-item validation |
| provider protocol formatters | adopt behind each adapter; consumers never format market symbols |
| local mapping functions in `historical_bars`, `market_data`, `magic_tdx`, `consensus`, `review`, `board`, `research`, `company`, `market_capabilities`, `capital`, `sina_instrument_news` and `security_lifecycle` | replace with the canonical resolver, then delete |
| broad `6/9`, `0/2/3`, `4/8` first-character maps | reject |
| `consensus` broad `4/8/9 -> Beijing` map | reject |
| tests that accept `400xxx` or `800xxx` as Beijing equities | replace with explicit unsupported tests |
| tests/providers that accept `43/83/87/88` as Beijing by prefix | retain only as provider-behavior evidence; current identity requires an exact official old/new mapping and canonicalizes to `92xxxx` |
| A-share wording applied to `200xxx`/`900xxx` | reject; carry B-share type or return unsupported |
| index/fund/bond/option identity paths | retain outside this resolver; migration requires a separate design |

There is no compatibility fallback. A consumer is migrated only when it uses
the new identity type and has explicit unsupported/conflict tests; its local
mapping is deleted in the same slice.

## 13. Gate progression and validation

### Gate A — this document

Machine-checkable documentation evidence:

```bash
rg -n "BR-173" docs/business_rules.md \
  docs/superpowers/specs/2026-07-27-a-share-instrument-identity-design.md
git diff --check -- docs/business_rules.md \
  docs/superpowers/specs/2026-07-27-a-share-instrument-identity-design.md
test -e docs/superpowers/specs/2026-07-27-a-share-instrument-identity-design.md
```

Expected result: BR-173 appears in both files, `git diff --check` is silent,
and the design-file existence check exits zero.

The current divergence can be reproduced without a network call:

```bash
rg -n "b'6'.*b'9'|b'4'.*b'8'|b'4'.*b'8'.*b'9'" src/data_gateway
```

Expected before Gate B: multiple incompatible consumer-owned mappings.
Expected after Gate B migration: no equity request constructor owns one of
these broad mappings.

### Gate B — planned implementation

Focused tests must exhaustively cover:

- every accepted prefix in section 4;
- `900xxx` Shanghai B and `200xxx` Shenzhen B;
- representative current `920xxx`, `921xxx` and `929xxx` Beijing identities;
- provider capability `Supported` for evidenced `920xxx` while identity-valid
  `921xxx` through `929xxx` remain `UnsupportedProviderCapability`;
- rejection of non-`92` 9-leading prefixes except Shanghai `900xxx`;
- rejection of `43/83/87/88` as current Beijing identities;
- exact official old/new mapping success, missing mapping, conflicting mapping,
  duplicate mapping and mapped-new-code-not-`92xxxx`;
- rejection of `400xxx`, `800xxx`, malformed and non-equity identities;
- provider exact match, provider conflict and provider-market missing;
- A-only capability rejection of B shares;
- capability-specific `Unsupported`;
- production/test namespace rejection in both directions; and
- returned record and batch identity mismatch.

Planned focused command:

```bash
cargo test --lib data_gateway::instrument_identity::tests::
```

### Gates C and D

After all affected request constructors migrate:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
bash tools/compliance/lib/check_business_rules.sh
bash tools/compliance/check.sh
```

Gate D additionally requires the repository coverage thresholds, real
provider validation for every newly supported capability cell, acquisition
audit evidence, bounded `monitor --review`, isolated `monitor --test`, and a
bounded normal-monitor smoke run. A passing identity unit test cannot promote
an unverified provider capability.

## 14. Rollout and rollback

Rollout is incremental by consumer but fail-closed at every slice:

1. implement the resolver and exhaustive pure tests;
2. add the provider/capability support registry;
3. migrate request constructors one consumer at a time;
4. validate returned record/batch identity and acquisition audit;
5. delete that consumer’s old mapping in the same change;
6. run all Gate B/C checks; and
7. enable newly proven provider cells only after real evidence is recorded.

Rollback is `git revert <identity-slice-commit>`, followed by the normal build,
test and compliance gates. It must not delete audit history or re-enable an
ambiguous mapping behind a feature flag. If a provider cell regresses, mark
only that provider/capability cell `Unsupported`; canonical security identity
does not change because an endpoint is unavailable.

## 15. Official sources

- [北证公告〔2024〕20号：北京证券交易所 全国中小企业股份转让系统证券代码、证券简称编制指引](https://www.bse.cn/jygl_list/200021626.html)
  is effective from 2024-04-22. Article 7 assigns listed-company ordinary
  shares to codes beginning `92` and NEEQ listed-company ordinary shares to
  codes beginning `83`, `87` or `88`.
- [关于北交所存量上市公司代码切换上线的通知](https://www.bse.cn/important_news/200026735.html)
  is dated 2025-09-12 and requires switched codes for existing shares from
  2025-10-09 for trading orders, quote queries and market-participant
  processing.
- [北交所新旧代码对照表](https://www.bse.cn/service/code_mapping.html)
  is the only admitted old/new alias source in this design. An old code absent
  from the exact table is not a current Beijing listing identity.
