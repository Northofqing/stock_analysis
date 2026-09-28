//! Exact bounded NewsFlashAdmittedRecord/v1 encoding of already admitted values.
//!
//! This proves normalized acquisition content, not original provider wire bytes
//! or provider authorship. Admission remains the parent projector's responsibility.

use super::{provider_id_wire_name, NewsFlashSourceIdentity, RegisteredGlobalNewsFeed};
use crate::data_gateway::{BatchEvidence, GlobalNewsRecord};
use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};
use std::fmt;

const SCHEMA: &str = "NewsFlashAdmittedRecord/v1";
pub const MAX_NEWS_FLASH_RECORD_EVIDENCE_BYTES: usize = 1_048_576;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NewsFlashRecordEvidenceError {
    MissingAdmittedRecord,
    CanonicalBytesLimitExceeded,
    AllocationFailed,
}

/// Immutable evidence; only the parent admission path can capture this value.
#[derive(Clone, PartialEq, Eq)]
pub struct NewsFlashRecordEvidenceV1 {
    canonical_bytes: Vec<u8>,
    content_sha256: String,
    item_id: String,
    registration: RegisteredGlobalNewsFeed,
    source: NewsFlashSourceIdentity,
}

impl fmt::Debug for NewsFlashRecordEvidenceV1 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NewsFlashRecordEvidenceV1")
            .field("schema", &SCHEMA)
            .field("canonical_bytes_len", &self.canonical_bytes.len())
            .field("content_sha256", &self.content_sha256)
            .finish()
    }
}

impl NewsFlashRecordEvidenceV1 {
    pub fn schema(&self) -> &'static str {
        SCHEMA
    }
    pub fn canonical_bytes(&self) -> &[u8] {
        &self.canonical_bytes
    }
    pub fn content_sha256(&self) -> &str {
        &self.content_sha256
    }
    pub fn item_id(&self) -> &str {
        &self.item_id
    }
    pub fn registration(&self) -> RegisteredGlobalNewsFeed {
        self.registration
    }
    pub fn source(&self) -> &NewsFlashSourceIdentity {
        &self.source
    }
}

// Task 5b connects this private entry point to the existing admission path.
#[allow(dead_code)]
pub(super) fn capture_admitted_record(
    registration: RegisteredGlobalNewsFeed,
    record: &GlobalNewsRecord,
    batch: &BatchEvidence,
    source: &NewsFlashSourceIdentity,
) -> Result<NewsFlashRecordEvidenceV1, NewsFlashRecordEvidenceError> {
    let mut counter = Writer {
        bytes: None,
        len: 0,
    };
    encode(&mut counter, registration, record, batch, source)?;
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(counter.len)
        .map_err(|_| NewsFlashRecordEvidenceError::AllocationFailed)?;
    let mut writer = Writer {
        bytes: Some(&mut bytes),
        len: 0,
    };
    encode(&mut writer, registration, record, batch, source)?;
    debug_assert_eq!(writer.len, counter.len);
    let content_sha256 = format!("{:x}", Sha256::digest(&bytes));
    Ok(NewsFlashRecordEvidenceV1 {
        canonical_bytes: bytes,
        content_sha256,
        item_id: record.item_id.clone(),
        registration,
        source: source.clone(),
    })
}

type EncodeResult = Result<(), NewsFlashRecordEvidenceError>;

/// Both passes use exactly the same writer. No user-controlled temporary JSON
/// or escaped string is allocated; only the successful second pass owns bytes.
struct Writer<'a> {
    bytes: Option<&'a mut Vec<u8>>,
    len: usize,
}

impl Writer<'_> {
    fn raw(&mut self, bytes: &[u8]) -> EncodeResult {
        let len = self
            .len
            .checked_add(bytes.len())
            .filter(|len| *len <= MAX_NEWS_FLASH_RECORD_EVIDENCE_BYTES)
            .ok_or(NewsFlashRecordEvidenceError::CanonicalBytesLimitExceeded)?;
        if let Some(output) = self.bytes.as_mut() {
            output.extend_from_slice(bytes);
        }
        self.len = len;
        Ok(())
    }

    fn string(&mut self, value: &str) -> EncodeResult {
        self.raw(b"\"")?;
        // Work in UTF-8 bytes: continuation bytes cannot equal ASCII controls.
        // Stop on the first over-limit byte, even for huge unescaped inputs.
        const HEX: &[u8; 16] = b"0123456789abcdef";
        for byte in value.bytes() {
            match byte {
                b'"' => self.raw(b"\\\"")?,
                b'\\' => self.raw(b"\\\\")?,
                0..=31 => self.raw(&[
                    b'\\',
                    b'u',
                    b'0',
                    b'0',
                    HEX[(byte >> 4) as usize],
                    HEX[(byte & 15) as usize],
                ])?,
                _ => self.raw(&[byte])?,
            }
        }
        self.raw(b"\"")
    }

    fn optional(&mut self, value: Option<&str>) -> EncodeResult {
        match value {
            Some(value) => self.string(value),
            None => self.raw(b"null"),
        }
    }

    fn array(&mut self, values: &[String]) -> EncodeResult {
        self.raw(b"[")?;
        for (index, value) in values.iter().enumerate() {
            if index != 0 {
                self.raw(b",")?;
            }
            self.string(value)?;
        }
        self.raw(b"]")
    }

    fn unsigned(&mut self, mut value: u64) -> EncodeResult {
        let mut digits = [0u8; 20];
        let mut start = digits.len();
        loop {
            start -= 1;
            digits[start] = b'0' + (value % 10) as u8;
            value /= 10;
            if value == 0 {
                break;
            }
        }
        self.raw(&digits[start..])
    }

    fn timestamp(&mut self, value: DateTime<Utc>) -> EncodeResult {
        self.raw(br#"{"nanoseconds":"#)?;
        self.unsigned(u64::from(value.timestamp_subsec_nanos()))?;
        self.raw(br#","unix_seconds":"#)?;
        if value.timestamp() < 0 {
            self.raw(b"-")?;
        }
        self.unsigned(value.timestamp().unsigned_abs())?;
        self.raw(b"}")
    }
}

fn encode(
    w: &mut Writer<'_>,
    registration: RegisteredGlobalNewsFeed,
    record: &GlobalNewsRecord,
    batch: &BatchEvidence,
    source: &NewsFlashSourceIdentity,
) -> EncodeResult {
    w.raw(br#"{"batch_evidence":"#)?;
    w.raw(br#"{"batch_id":"#)?;
    w.string(&batch.batch_id)?;
    w.raw(br#","observed_at":"#)?;
    w.string(&batch.observed_at)?;
    w.raw(br#","provider":"#)?;
    w.string(provider_id_wire_name(batch.provider))?;
    w.raw(br#","source":"#)?;
    w.string(&batch.source)?;
    w.raw(br#","source_at":"#)?;
    w.optional(batch.source_at.as_deref())?;
    w.raw(b"}")?;
    w.raw(br#","record":"#)?;
    w.raw(br#"{"canonical_url":"#)?;
    w.string(&record.canonical_url)?;
    w.raw(br#","content":"#)?;
    w.optional(record.content.as_deref())?;
    w.raw(br#","evidence":"#)?;
    w.raw(br#"{"batch_id":"#)?;
    w.string(record.evidence.batch_id())?;
    w.raw(br#","observed_at":"#)?;
    w.string(record.evidence.observed_at())?;
    w.raw(br#","provider":"#)?;
    w.string(provider_id_wire_name(record.evidence.provider()))?;
    w.raw(br#","source_at":"#)?;
    w.optional(record.evidence.source_at())?;
    w.raw(b"}")?;
    w.raw(br#","instruments":"#)?;
    w.array(&record.instruments)?;
    w.raw(br#","item_id":"#)?;
    w.string(&record.item_id)?;
    w.raw(br#","language":"#)?;
    w.string(&record.language)?;
    w.raw(br#","observed_at":"#)?;
    w.timestamp(record.observed_at)?;
    w.raw(br#","published_at":"#)?;
    w.timestamp(record.published_at)?;
    w.raw(br#","publisher":"#)?;
    w.string(&record.publisher)?;
    w.raw(br#","summary":"#)?;
    w.optional(record.summary.as_deref())?;
    w.raw(br#","title":"#)?;
    w.string(&record.title)?;
    w.raw(br#","topics":"#)?;
    w.array(&record.topics)?;
    w.raw(b"}")?;
    w.raw(br#","registration":"#)?;
    w.raw(br#"{"capability_name":"#)?;
    w.string(registration.capability_name)?;
    w.raw(br#","feed_name":"#)?;
    w.string(registration.feed_name)?;
    w.raw(br#","gateway_provider":"#)?;
    w.string(registration.gateway_provider)?;
    w.raw(br#","max_limit":"#)?;
    w.unsigned(u64::from(registration.max_limit))?;
    w.raw(br#","provider":"#)?;
    w.string(registration.provider.wire_name())?;
    w.raw(br#","provider_id":"#)?;
    w.string(registration.provider_id)?;
    w.raw(br#","source_contract":"#)?;
    w.string(registration.source_contract)?;
    w.raw(br#","upstream_revision":"#)?;
    w.string(registration.upstream_revision)?;
    w.raw(b"}")?;
    w.raw(br#","schema":"#)?;
    w.string(SCHEMA)?;
    w.raw(br#","source_identity":"#)?;
    w.raw(br#"{"batch_id":"#)?;
    w.string(source.batch_id())?;
    w.raw(br#","event_id":"#)?;
    w.string(source.event_id())?;
    w.raw(br#","observed_at":"#)?;
    w.timestamp(source.observed_at())?;
    w.raw(br#","provider":"#)?;
    w.string(source.provider())?;
    w.raw(br#","published_at":"#)?;
    w.timestamp(source.published_at())?;
    w.raw(br#","source":"#)?;
    w.string(source.source())?;
    w.raw(b"}")?;
    w.raw(b"}")
}

#[cfg(test)]
mod tests {
    use super::super::{ordered_news_flash_evidence_sha256, NewsFlashProjectedEvent};
    use super::*;
    use crate::data_gateway::GlobalNewsProvider;
    use crate::market_domain::{ProviderId, SourceEvidence};

    #[derive(Clone)]
    struct Fixture {
        registration: RegisteredGlobalNewsFeed,
        record: GlobalNewsRecord,
        batch: BatchEvidence,
        source: NewsFlashSourceIdentity,
    }

    impl Fixture {
        fn capture(&self) -> Result<NewsFlashRecordEvidenceV1, NewsFlashRecordEvidenceError> {
            capture_admitted_record(self.registration, &self.record, &self.batch, &self.source)
        }
    }

    fn fixture() -> Fixture {
        let provider = GlobalNewsProvider::Eastmoney;
        let published_at = DateTime::from_timestamp(-1, 123_456_789).unwrap();
        let observed_at = DateTime::from_timestamp(0, 987_654_321).unwrap();
        let batch = BatchEvidence {
            provider: ProviderId::Eastmoney,
            source: provider.source().into(),
            source_at: Some("1969-12-31T23:59:59.500000000Z".into()),
            observed_at: "1970-01-01T00:00:00.987654321Z".into(),
            batch_id: "TEST_CODE_BATCH".into(),
        };
        let record = GlobalNewsRecord {
            item_id: "TEST_CODE_ITEM".into(),
            title: "新闻\"\\\n\t\0\u{1f}/\u{2028}\u{2029}e\u{301}".into(),
            summary: Some(String::new()),
            content: None,
            publisher: " 发布者 ".into(),
            canonical_url: "https://example.test/新闻?a=1&b=<2>".into(),
            published_at,
            observed_at,
            instruments: vec![],
            topics: vec!["甲".into(), "".into(), "甲".into()],
            language: "zh-CN".into(),
            evidence: SourceEvidence::new(batch.provider, &batch.observed_at, &batch.batch_id)
                .unwrap()
                .with_source_at("1969-12-31T23:59:59.123456789Z")
                .unwrap(),
        };
        let source = NewsFlashSourceIdentity {
            event_id: EVENT_ID.into(),
            provider: provider.wire_name().into(),
            source: provider.source().into(),
            published_at,
            observed_at,
            batch_id: batch.batch_id.clone(),
        };
        Fixture {
            registration: RegisteredGlobalNewsFeed::for_provider(provider),
            record,
            batch,
            source,
        }
    }

    // Independent vectors: Python json.dumps(sort_keys=True, ensure_ascii=False,
    // separators=(",", ":")), with short control escapes expanded to lowercase
    // Unicode escapes; hashlib.sha256 over the resulting UTF-8 bytes.
    const EVENT_ID: &str = "1572f77c3b14ac7e545782b486a8fd727207ac10f7f9adb0156b0672c7169d8b";
    const LEGACY_SHA: &str = "0a25cb4b9381fbedc898c4060cf8c6b8ac28da6b308a046089271904ae2f9d99";
    const GOLDEN: &str = r###"{"batch_evidence":{"batch_id":"TEST_CODE_BATCH","observed_at":"1970-01-01T00:00:00.987654321Z","provider":"Eastmoney","source":"eastmoney-web","source_at":"1969-12-31T23:59:59.500000000Z"},"record":{"canonical_url":"https://example.test/新闻?a=1&b=<2>","content":null,"evidence":{"batch_id":"TEST_CODE_BATCH","observed_at":"1970-01-01T00:00:00.987654321Z","provider":"Eastmoney","source_at":"1969-12-31T23:59:59.123456789Z"},"instruments":[],"item_id":"TEST_CODE_ITEM","language":"zh-CN","observed_at":{"nanoseconds":987654321,"unix_seconds":0},"published_at":{"nanoseconds":123456789,"unix_seconds":-1},"publisher":" 发布者 ","summary":"","title":"新闻\"\\\u000a\u0009\u0000\u001f/  é","topics":["甲","","甲"]},"registration":{"capability_name":"GlobalNews-Eastmoney","feed_name":"eastmoney_global_news","gateway_provider":"eastmoney","max_limit":20,"provider":"Eastmoney","provider_id":"eastmoney","source_contract":"eastmoney-web","upstream_revision":"75ee2a2bdd3b1ca2b01ce3afbb04aec416e7000e"},"schema":"NewsFlashAdmittedRecord/v1","source_identity":{"batch_id":"TEST_CODE_BATCH","event_id":"1572f77c3b14ac7e545782b486a8fd727207ac10f7f9adb0156b0672c7169d8b","observed_at":{"nanoseconds":987654321,"unix_seconds":0},"provider":"Eastmoney","published_at":{"nanoseconds":123456789,"unix_seconds":-1},"source":"eastmoney-web"}}"###;
    const GOLDEN_SHA: &str = "8f90d8a419cbdb8a5adb226316902f2fbf47ee63f604ca427bfa55d1476f4aad";
    const CHANGED_GOLDEN: &str = r###"{"batch_evidence":{"batch_id":"TEST_CODE_BATCH","observed_at":"1970-01-01T00:00:00.987654321Z","provider":"Eastmoney","source":"eastmoney-web","source_at":"1969-12-31T23:59:59.500000000Z"},"record":{"canonical_url":"https://example.test/新闻?a=1&b=<2>","content":"隐含正文","evidence":{"batch_id":"TEST_CODE_BATCH","observed_at":"1970-01-01T00:00:00.987654321Z","provider":"Eastmoney","source_at":"1969-12-31T23:59:59.123456789Z"},"instruments":[],"item_id":"TEST_CODE_ITEM","language":"zh-CN","observed_at":{"nanoseconds":987654321,"unix_seconds":0},"published_at":{"nanoseconds":123456789,"unix_seconds":-1},"publisher":" 发布者 ","summary":null,"title":"新闻\"\\\u000a\u0009\u0000\u001f/  é","topics":["甲","","甲"]},"registration":{"capability_name":"GlobalNews-Eastmoney","feed_name":"eastmoney_global_news","gateway_provider":"eastmoney","max_limit":20,"provider":"Eastmoney","provider_id":"eastmoney","source_contract":"eastmoney-web","upstream_revision":"75ee2a2bdd3b1ca2b01ce3afbb04aec416e7000e"},"schema":"NewsFlashAdmittedRecord/v1","source_identity":{"batch_id":"TEST_CODE_BATCH","event_id":"1572f77c3b14ac7e545782b486a8fd727207ac10f7f9adb0156b0672c7169d8b","observed_at":{"nanoseconds":987654321,"unix_seconds":0},"provider":"Eastmoney","published_at":{"nanoseconds":123456789,"unix_seconds":-1},"source":"eastmoney-web"}}"###;
    const CHANGED_GOLDEN_SHA: &str =
        "3ba5dd644a23972b2006376ea87f4366cf9f6e2994a3f2439c6f8303f598284a";

    #[test]
    fn n02_record_evidence_literal_goldens_and_legacy_identity() {
        let fixture = fixture();
        let evidence = fixture.capture().unwrap();
        assert_eq!(evidence.canonical_bytes(), GOLDEN.as_bytes());
        assert_eq!(evidence.content_sha256(), GOLDEN_SHA);
        assert_eq!(evidence.schema(), "NewsFlashAdmittedRecord/v1");
        assert_eq!(evidence.item_id(), "TEST_CODE_ITEM");
        assert_eq!(evidence.registration(), fixture.registration);
        assert_eq!(evidence.source(), &fixture.source);
        let mut changed = fixture.clone();
        changed.record.summary = None;
        changed.record.content = Some("隐含正文".into());
        let changed_evidence = changed.capture().unwrap();
        assert_eq!(
            changed_evidence.canonical_bytes(),
            CHANGED_GOLDEN.as_bytes()
        );
        assert_eq!(changed_evidence.content_sha256(), CHANGED_GOLDEN_SHA);
        assert_ne!(GOLDEN_SHA, CHANGED_GOLDEN_SHA);
        for input in [&fixture, &changed] {
            let event = crate::news::aggregator::feed::record_to_market_event(
                input.registration.provider,
                &input.record,
            )
            .unwrap();
            assert_eq!(event.event_id, EVENT_ID);
            let projected = NewsFlashProjectedEvent {
                event,
                source: input.source.clone(),
            };
            assert_eq!(ordered_news_flash_evidence_sha256(&[projected]), LEGACY_SHA);
        }
        assert_eq!(evidence.clone(), evidence);
        let debug = format!("{evidence:?}");
        assert!(debug.contains(GOLDEN_SHA));
        assert!(!debug.contains("新闻"));
        assert!(!debug.contains("TEST_CODE_ITEM"));
    }

    #[test]
    fn n02_record_evidence_every_retained_field_is_sensitive() {
        type Mutation = (&'static str, fn(&mut Fixture));
        let mutations: &[Mutation] = &[
            ("batch.batch_id", |f| {
                f.batch.batch_id.push_str("2");
            }),
            ("batch.observed_at", |f| {
                f.batch.observed_at.push_str("2");
            }),
            ("batch.provider", |f| {
                f.batch.provider = ProviderId::Jin10;
            }),
            ("batch.source", |f| {
                f.batch.source.push_str("2");
            }),
            ("batch.source_at", |f| {
                f.batch.source_at = Some("1969-12-31T23:59:58Z".into());
            }),
            ("record.evidence.batch_id", |f| {
                f.record.evidence = SourceEvidence::new(
                    ProviderId::Eastmoney,
                    f.record.evidence.observed_at(),
                    "TEST_CODE_OTHER",
                )
                .unwrap()
                .with_source_at(f.record.evidence.source_at().unwrap())
                .unwrap();
            }),
            ("record.evidence.observed_at", |f| {
                f.record.evidence = SourceEvidence::new(
                    ProviderId::Eastmoney,
                    "1970-01-01T00:00:01Z",
                    f.record.evidence.batch_id(),
                )
                .unwrap()
                .with_source_at(f.record.evidence.source_at().unwrap())
                .unwrap();
            }),
            ("record.evidence.provider", |f| {
                f.record.evidence = SourceEvidence::new(
                    ProviderId::Jin10,
                    f.record.evidence.observed_at(),
                    f.record.evidence.batch_id(),
                )
                .unwrap()
                .with_source_at(f.record.evidence.source_at().unwrap())
                .unwrap();
            }),
            ("record.evidence.source_at", |f| {
                f.record.evidence = SourceEvidence::new(
                    ProviderId::Eastmoney,
                    f.record.evidence.observed_at(),
                    f.record.evidence.batch_id(),
                )
                .unwrap()
                .with_source_at("1969-12-31T23:59:58Z")
                .unwrap();
            }),
            ("record.canonical_url", |f| {
                f.record.canonical_url.push_str("2");
            }),
            ("record.item_id", |f| {
                f.record.item_id.push_str("2");
            }),
            ("record.language", |f| {
                f.record.language.push_str("2");
            }),
            ("record.publisher", |f| {
                f.record.publisher.push_str("2");
            }),
            ("record.title", |f| {
                f.record.title.push_str("2");
            }),
            ("record.content", |f| {
                f.record.content = Some("changed".into());
            }),
            ("record.summary", |f| {
                f.record.summary = Some("changed".into());
            }),
            ("record.instruments", |f| {
                f.record.instruments.push("changed".into());
            }),
            ("record.topics", |f| {
                f.record.topics.push("changed".into());
            }),
            ("record.observed_at", |f| {
                f.record.observed_at += chrono::Duration::nanoseconds(1);
            }),
            ("record.published_at", |f| {
                f.record.published_at += chrono::Duration::nanoseconds(1);
            }),
            ("source.observed_at", |f| {
                f.source.observed_at += chrono::Duration::nanoseconds(1);
            }),
            ("source.published_at", |f| {
                f.source.published_at += chrono::Duration::nanoseconds(1);
            }),
            ("source.batch_id", |f| {
                f.source.batch_id.push_str("2");
            }),
            ("source.event_id", |f| {
                f.source.event_id.push_str("2");
            }),
            ("source.provider", |f| {
                f.source.provider.push_str("2");
            }),
            ("source.source", |f| {
                f.source.source.push_str("2");
            }),
            ("registration.capability_name", |f| {
                f.registration.capability_name = "TEST_CODE_OTHER";
            }),
            ("registration.feed_name", |f| {
                f.registration.feed_name = "TEST_CODE_OTHER";
            }),
            ("registration.gateway_provider", |f| {
                f.registration.gateway_provider = "TEST_CODE_OTHER";
            }),
            ("registration.provider_id", |f| {
                f.registration.provider_id = "TEST_CODE_OTHER";
            }),
            ("registration.source_contract", |f| {
                f.registration.source_contract = "TEST_CODE_OTHER";
            }),
            ("registration.upstream_revision", |f| {
                f.registration.upstream_revision = "TEST_CODE_OTHER";
            }),
            ("registration.max_limit", |f| {
                f.registration.max_limit += 1;
            }),
            ("registration.provider", |f| {
                f.registration.provider = GlobalNewsProvider::Jin10;
            }),
        ];
        let original = fixture().capture().unwrap();
        for (field, mutate) in mutations {
            let mut changed = fixture();
            mutate(&mut changed);
            let evidence = changed.capture().unwrap();
            assert_ne!(
                evidence.canonical_bytes(),
                original.canonical_bytes(),
                "{field}"
            );
            assert_ne!(
                evidence.content_sha256(),
                original.content_sha256(),
                "{field}"
            );
        }
    }

    #[test]
    fn n02_record_evidence_options_arrays_and_raw_timestamp_spellings() {
        let base = fixture();
        let original = base.capture().unwrap();
        let mutations: &[fn(&mut Fixture)] = &[
            |f| f.record.summary = None,
            |f| f.record.content = Some(String::new()),
            |f| f.record.instruments = vec![String::new()],
            |f| f.record.topics.swap(0, 1),
            |f| {
                f.record.topics.pop();
            },
            |f| f.batch.source_at = None,
            |f| f.batch.source_at = Some(String::new()),
            |f| {
                f.record.evidence = SourceEvidence::new(
                    ProviderId::Eastmoney,
                    f.record.evidence.observed_at(),
                    f.record.evidence.batch_id(),
                )
                .unwrap()
            },
            |f| f.batch.observed_at = "1970-01-01T00:00:00.987654321+00:00".into(),
        ];
        for mutate in mutations {
            let mut changed = base.clone();
            mutate(&mut changed);
            assert_ne!(
                changed.capture().unwrap().content_sha256(),
                original.content_sha256()
            );
        }
        let mut none = base.clone();
        none.batch.source_at = None;
        let mut empty = base.clone();
        empty.batch.source_at = Some(String::new());
        assert_ne!(
            none.capture().unwrap().canonical_bytes(),
            empty.capture().unwrap().canonical_bytes()
        );
        // SourceEvidence normalization happened upstream; its getters are the contract.
        let mut normalized = base.clone();
        normalized.record.evidence = SourceEvidence::new(
            ProviderId::Eastmoney,
            format!(" {} ", base.record.evidence.observed_at()),
            " TEST_CODE_BATCH ",
        )
        .unwrap()
        .with_source_at(format!(" {} ", base.record.evidence.source_at().unwrap()))
        .unwrap();
        assert_eq!(normalized.capture().unwrap(), original);
    }

    fn encoded(write: impl FnOnce(&mut Writer<'_>) -> EncodeResult) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(1024);
        write(&mut Writer {
            bytes: Some(&mut bytes),
            len: 0,
        })
        .unwrap();
        bytes
    }

    #[test]
    fn n02_record_evidence_all_controls_and_literal_unicode() {
        let controls: String = (0..=31).map(char::from).collect();
        assert_eq!(encoded(|w| w.string(&controls)), br#""\u0000\u0001\u0002\u0003\u0004\u0005\u0006\u0007\u0008\u0009\u000a\u000b\u000c\u000d\u000e\u000f\u0010\u0011\u0012\u0013\u0014\u0015\u0016\u0017\u0018\u0019\u001a\u001b\u001c\u001d\u001e\u001f""#);
        assert_eq!(
            encoded(|w| w.string("/<> &中\u{2028}\u{2029}e\u{301}")),
            "\"/<> &中\u{2028}\u{2029}e\u{301}\"".as_bytes()
        );
        assert_ne!(
            encoded(|w| w.string("é")),
            encoded(|w| w.string("e\u{301}"))
        );
    }

    #[test]
    fn n02_record_evidence_timestamp_precision_and_leap_second() {
        assert_eq!(
            encoded(|w| w.timestamp(DateTime::from_timestamp(-1, 123_456_789).unwrap())),
            br#"{"nanoseconds":123456789,"unix_seconds":-1}"#
        );
        assert_eq!(
            encoded(|w| w.timestamp(DateTime::from_timestamp(0, 0).unwrap())),
            br#"{"nanoseconds":0,"unix_seconds":0}"#
        );
        assert_eq!(
            encoded(|w| w.timestamp(DateTime::from_timestamp(59, 1_234_567_890).unwrap())),
            br#"{"nanoseconds":1234567890,"unix_seconds":59}"#
        );
        let utc: DateTime<Utc> = "1970-01-01T00:00:00.987654321Z".parse().unwrap();
        let offset: DateTime<Utc> = "1970-01-01T08:00:00.987654321+08:00".parse().unwrap();
        assert_eq!(
            encoded(|w| w.timestamp(utc)),
            encoded(|w| w.timestamp(offset))
        );
        assert_eq!(encoded(|w| w.unsigned(u64::MAX)), b"18446744073709551615");
    }

    #[test]
    fn n02_record_evidence_complete_byte_limit_and_escaped_utf8() {
        let mut input = fixture();
        let base_len = input.capture().unwrap().canonical_bytes().len();
        // Summary is already Some(""); each added ASCII byte adds exactly one.
        input.record.summary = Some("a".repeat(MAX_NEWS_FLASH_RECORD_EVIDENCE_BYTES - base_len));
        assert_eq!(
            input.capture().unwrap().canonical_bytes().len(),
            MAX_NEWS_FLASH_RECORD_EVIDENCE_BYTES
        );
        input.record.summary.as_mut().unwrap().push('a');
        assert_eq!(
            input.capture().unwrap_err(),
            NewsFlashRecordEvidenceError::CanonicalBytesLimitExceeded
        );
        // One CJK scalar (3 bytes) plus a control escape (6 bytes).
        input.record.summary = Some(format!(
            "{}中\n",
            "a".repeat(MAX_NEWS_FLASH_RECORD_EVIDENCE_BYTES - base_len - 9)
        ));
        assert_eq!(
            input.capture().unwrap().canonical_bytes().len(),
            MAX_NEWS_FLASH_RECORD_EVIDENCE_BYTES
        );
        input.record.summary.as_mut().unwrap().push('a');
        assert_eq!(
            input.capture().unwrap_err(),
            NewsFlashRecordEvidenceError::CanonicalBytesLimitExceeded
        );
        let mut counter = Writer {
            bytes: None,
            len: usize::MAX,
        };
        assert_eq!(
            counter.raw(b"x"),
            Err(NewsFlashRecordEvidenceError::CanonicalBytesLimitExceeded)
        );
        let mut counter = Writer {
            bytes: None,
            len: MAX_NEWS_FLASH_RECORD_EVIDENCE_BYTES - 1,
        };
        assert_eq!(
            counter.string("中"),
            Err(NewsFlashRecordEvidenceError::CanonicalBytesLimitExceeded)
        );
        assert_eq!(counter.len, MAX_NEWS_FLASH_RECORD_EVIDENCE_BYTES);
    }
}
