//! Original serde is a test oracle only; it never provides production admission.
use super::*;
use crate::trading::{paper_book_v2, paper_book_v2_execution, paper_ledger};
use paper_ledger::{AccountBinding, Projection};

#[derive(Clone, Copy, Default)]
struct Form {
    choice: usize,
    buffered: bool,
    unit_maps: bool,
    arrays: bool,
    escaped: bool,
    width: usize,
}
fn quote(s: &str) -> String {
    serde_json::to_string(s).unwrap()
}
fn fields_json(fields: &[Field], f: Form, origin: Origin, array: bool) -> String {
    let parts: Vec<String> = fields
        .iter()
        .map(|field| {
            let value = fixture(field.shape, f, origin);
            if array {
                value
            } else {
                format!("{}:{}", quote(field.name), value)
            }
        })
        .collect();
    if array {
        format!("[{}]", parts.join(","))
    } else {
        format!("{{{}}}", parts.join(","))
    }
}
fn payload(body: Body, f: Form, origin: Origin, external: bool) -> String {
    match body {
        Body::Unit => {
            if external && origin == Origin::Buffered && f.unit_maps {
                "{}".into()
            } else {
                "null".into()
            }
        }
        Body::Value(shape) => fixture(shape, f, origin),
        Body::Record(fields, _) => fields_json(fields, f, origin, external && f.arrays),
    }
}
fn fixture(shape: &Shape, f: Form, origin: Origin) -> String {
    match *shape {
        Shape::Scalar(s) => match s {
            Scalar::String => {
                if f.escaped {
                    r#""left\n\u4e2d\uD83D\uDE00right""#.into()
                } else {
                    quote("nonempty-evidence")
                }
            }
            Scalar::Date => quote("2026-10-04"),
            Scalar::DateTime => quote("2026-10-04T01:02:03Z"),
            Scalar::Bool => "true".into(),
            Scalar::Integer => "1".into(),
            Scalar::Float => "0.25".into(),
        },
        Shape::Option(s) => fixture(s, f, origin),
        Shape::Seq(s) => format!(
            "[{}]",
            (0..f.width.max(1))
                .map(|_| fixture(s, f, origin))
                .collect::<Vec<_>>()
                .join(",")
        ),
        Shape::Map(k, v) => {
            let entries = (0..f.width.max(1))
                .map(|i| {
                    let key = match k {
                        Shape::Scalar(Scalar::Date) => quote(&format!("2026-10-{:02}", i + 1)),
                        Shape::Scalar(Scalar::String) => quote(&format!("map-key-{i}")),
                        _ => fixture(k, f, origin),
                    };
                    format!("{}:{}", key, fixture(v, f, origin))
                })
                .collect::<Vec<_>>();
            format!("{{{}}}", entries.join(","))
        }
        Shape::Tuple(a, b) => format!("[{},{}]", fixture(a, f, origin), fixture(b, f, origin)),
        Shape::Record(fields, _, array) => fields_json(fields, f, origin, array && f.arrays),
        Shape::External(variants) => {
            let v = &variants[f.choice % variants.len()];
            if matches!(v.body, Body::Unit) && !(origin == Origin::Buffered && f.unit_maps) {
                quote(v.name)
            } else {
                format!("{{{}:{}}}", quote(v.name), payload(v.body, f, origin, true))
            }
        }
        Shape::Adjacent(tag, content, variants) => {
            let v = &variants[f.choice % variants.len()];
            let mode = if f.buffered { Origin::Buffered } else { origin };
            let value = payload(v.body, f, mode, false);
            if f.buffered {
                format!(
                    "{{{}:{},{}:{}}}",
                    quote(content),
                    value,
                    quote(tag),
                    quote(v.name)
                )
            } else {
                format!(
                    "{{{}:{},{}:{}}}",
                    quote(tag),
                    quote(v.name),
                    quote(content),
                    value
                )
            }
        }
    }
}
fn paired<T: Root + serde::de::DeserializeOwned>(
    bytes: &[u8],
    work: &mut CodecMechanics<'_, '_>,
    copy: bool,
) {
    let original: T = serde_json::from_slice(bytes).expect("original accepts finite fixture");
    let expected = serde_json::to_vec(&original).unwrap();
    let before = work.used();
    let decoded = decode_core::<T>(bytes, work).expect("paid lower decode");
    assert!(work.used() > before);
    assert_eq!(encode_core(&decoded, work).unwrap(), expected);
    if copy {
        let before = work.used();
        let copied = decoded.paid_copy(work).unwrap();
        assert!(work.used() > before);
        assert_eq!(encode_core(&copied, work).unwrap(), expected);
    }
}
pub(super) fn exercise_root<T: Root + serde::de::DeserializeOwned>(
    case: CodecFixtureCase,
    work: &mut CodecMechanics<'_, '_>,
) {
    if special_root::<T>(case, work) {
        return;
    }
    if matches!(
        case,
        CodecFixtureCase::RejectNoncanonical | CodecFixtureCase::RejectUnknownCommand
    ) {
        if !matches!(T::ROOT, RootKind::ExecutionFact) {
            return;
        }
        let mut bytes = fixture(
            &T::SHAPE,
            Form {
                choice: 2,
                buffered: true,
                unit_maps: true,
                ..Form::default()
            },
            Origin::Direct,
        );
        if matches!(case, CodecFixtureCase::RejectUnknownCommand) {
            bytes = bytes.replacen("\"expected\":", "\"unknown\":null,\"expected\":", 1);
            reject::<T>(bytes.as_bytes(), work);
            assert_eq!(work.hits.decoder, 0);
        } else {
            assert!(serde_json::from_str::<T>(&bytes).is_ok());
            assert!(decode_record_core::<T>(bytes.as_bytes(), work).is_err());
            assert!(work.hits.decoder > 0);
            assert!(work.hits.outputs > 0);
        }
        return;
    }
    if matches!(
        case,
        CodecFixtureCase::RejectDirectUnitMap
            | CodecFixtureCase::RejectAdjacentUnitMap
            | CodecFixtureCase::RejectBufferedIgnored
            | CodecFixtureCase::RejectTypedDepth
    ) {
        if !matches!(T::ROOT, RootKind::ExecutionFact) {
            return;
        }
        let f = Form {
            choice: if matches!(case, CodecFixtureCase::RejectDirectUnitMap) {
                2
            } else {
                0
            },
            buffered: !matches!(case, CodecFixtureCase::RejectDirectUnitMap),
            ..Form::default()
        };
        let mut bytes = fixture(&T::SHAPE, f, Origin::Direct);
        match case {
            CodecFixtureCase::RejectDirectUnitMap => {
                bytes = bytes.replacen(
                    "\"record\":\"LessThanWholeLot\"",
                    "\"record\":{\"LessThanWholeLot\":{}}",
                    1,
                )
            }
            CodecFixtureCase::RejectAdjacentUnitMap => {
                bytes = bytes.replacen("\"record\":null", "\"record\":{}", 1)
            }
            _ => {
                let ignored = if matches!(case, CodecFixtureCase::RejectTypedDepth) {
                    format!("{}0{}", "[".repeat(127), "]".repeat(127))
                } else {
                    "1e99999".into()
                };
                bytes = bytes.replacen(
                    "\"manifest\":",
                    &format!("\"ignored\":{},\"manifest\":", ignored),
                    1,
                );
            }
        }
        reject::<T>(bytes.as_bytes(), work);
        if matches!(case, CodecFixtureCase::RejectBufferedIgnored) {
            assert_eq!(work.failure_kind(), Some(K::FloatRange));
        }
        if matches!(case, CodecFixtureCase::RejectTypedDepth) {
            assert_eq!(work.failure_kind(), Some(K::TypedRecursionLimit));
        }
        assert_eq!(work.hits.decoder, 0);
        return;
    }
    let choices = if matches!(
        case,
        CodecFixtureCase::Variants | CodecFixtureCase::BufferedUnits
    ) {
        40
    } else {
        1
    };
    for choice in 0..choices {
        let form = Form {
            choice,
            buffered: matches!(case, CodecFixtureCase::BufferedUnits),
            unit_maps: matches!(case, CodecFixtureCase::BufferedUnits),
            arrays: matches!(case, CodecFixtureCase::ScalarBoundaries),
            escaped: matches!(case, CodecFixtureCase::Scratch),
            width: if matches!(case, CodecFixtureCase::Copies) {
                9
            } else {
                1
            },
        };
        let bytes = fixture(&T::SHAPE, form, Origin::Direct);
        paired::<T>(
            bytes.as_bytes(),
            work,
            matches!(
                case,
                CodecFixtureCase::Copies
                    | CodecFixtureCase::Variants
                    | CodecFixtureCase::BufferedUnits
            ),
        );
    }
}
fn binding() -> String {
    r#"{"account_id":"a","epoch_id":"e","manifest_hash":"m"}"#.into()
}
fn reject<T: Root + serde::de::DeserializeOwned>(bytes: &[u8], work: &mut CodecMechanics<'_, '_>) {
    assert!(
        serde_json::from_slice::<T>(bytes).is_err(),
        "original rejection"
    );
    assert!(decode_core::<T>(bytes, work).is_err());
    let failure = work.finish().unwrap_err();
    let used = work.used();
    let hits = work.hits;
    assert_eq!(
        decode_core::<AccountBinding>(binding().as_bytes(), work).err(),
        Some(failure)
    );
    assert_eq!(work.used(), used);
    assert_eq!(work.hits, hits);
}
fn hash_fixture(work: &mut CodecMechanics<'_, '_>) {
    let bytes = fixture(
        &<paper_ledger::SeedManifest as Value>::SHAPE,
        Form::default(),
        Origin::Direct,
    );
    let seed: paper_ledger::SeedManifest = serde_json::from_str(&bytes).unwrap();
    assert_eq!(
        digest_core(HashInput::SeedBinding(&seed), work).unwrap(),
        seed.binding().unwrap().manifest_hash
    );
    let bytes = fixture(
        &<paper_book_v2_execution::ExecutionManifest as Value>::SHAPE,
        Form::default(),
        Origin::Direct,
    );
    let manifest: paper_book_v2_execution::ExecutionManifest =
        serde_json::from_str(&bytes).unwrap();
    assert_eq!(
        digest_core(HashInput::ExecutionManifest(&manifest), work).unwrap(),
        manifest.identity().unwrap()
    );

    let raw = b"old canonical bytes\n";
    let expected = hex::encode(Sha256::digest(raw));
    assert_eq!(digest_core(HashInput::Raw(raw), work).unwrap(), expected);
    let mut h = Sha256::new();
    h.update(b"paper-book-v2-cutover-manifest/v1\n");
    h.update(raw);
    assert_eq!(
        digest_core(HashInput::Cutover(raw), work).unwrap(),
        hex::encode(h.finalize())
    );
    let encoded = serde_json::to_vec(&("PAPER_EVENT_V1", "a", 7_i64, "c", "p", "payload")).unwrap();
    assert_eq!(
        digest_core(
            HashInput::V1Event {
                account: "a",
                seq: 7,
                command: "c",
                previous: "p",
                payload: "payload"
            },
            work
        )
        .unwrap(),
        hex::encode(Sha256::digest(encoded))
    );
    for genesis in [true, false] {
        let encoded = serde_json::to_vec(&(
            "a",
            if genesis { 1_i64 } else { 7 },
            "c",
            "p",
            raw.as_slice(),
        ))
        .unwrap();
        let mut h = Sha256::new();
        h.update(if genesis {
            b"paper-book-v2-genesis-event/v1\n".as_slice()
        } else {
            b"paper-parent-event/v1\n".as_slice()
        });
        h.update(encoded);
        let input = if genesis {
            HashInput::GenesisEvent {
                account: "a",
                command: "c",
                previous: "p",
                payload: raw,
            }
        } else {
            HashInput::ExecutionEvent {
                account: "a",
                seq: 7,
                command: "c",
                previous: "p",
                payload: raw,
            }
        };
        assert_eq!(digest_core(input, work).unwrap(), hex::encode(h.finalize()));
    }
}
pub(super) fn run(case: CodecFixtureCase, work: &mut CodecMechanics<'_, '_>) {
    if special_run(case, work) {
        return;
    }

    match case{
 CodecFixtureCase::AllRoots|CodecFixtureCase::Variants|CodecFixtureCase::BufferedUnits|CodecFixtureCase::ScalarBoundaries|CodecFixtureCase::Copies|CodecFixtureCase::Scratch=>{
  paper_ledger::replay_codec_fixtures(case,work);paper_book_v2::replay_codec_fixtures(case,work);paper_book_v2_execution::replay_codec_fixtures(case,work);
  assert!(work.hits.decoder>=9);assert!(work.hits.strings>0);assert!(work.hits.vectors>0);assert!(work.hits.maps>0);
 },
 CodecFixtureCase::UnknownContent=>{
  let ignored=format!("{}0{}","[".repeat(400),"]".repeat(400));let bytes=format!(r#"{{"account_id":"a","epoch_id":"e","manifest_hash":"m","ignored":{},"other":1e99999,"surrogate":"\uD800"}}"#,ignored);
  paired::<AccountBinding>(bytes.as_bytes(),work,false);
  let defaults=r#"{"cash":0,"lots":[],"marks":{},"fees":0,"realized_pnl":0,"seed_equity":0,"as_of":"2026-10-04T00:00:00Z","closes":{}}"#;paired::<Projection>(defaults.as_bytes(),work,false);
  let positional=r#"[0,[],{},0,0,0,"2026-10-04T00:00:00Z",{}]"#;paired::<Projection>(positional.as_bytes(),work,false);
 },
 CodecFixtureCase::CanonicalHashes=>{hash_fixture(work);let pretty=b" { \"account_id\":\"a\",\"epoch_id\":\"e\",\"manifest_hash\":\"m\" } ";let value=decode_core::<AccountBinding>(pretty,work).unwrap();assert_ne!(encode_core(&value,work).unwrap(),pretty);},
 CodecFixtureCase::Cumulative=>{let bytes=fixture(&Projection::SHAPE,Form{width:9,..Form::default()},Origin::Direct);let p=decode_core::<Projection>(bytes.as_bytes(),work).unwrap();let mut count=0;loop{let before=work.used();match p.paid_copy(work){Ok(_)=>{assert!(work.used()>before);count+=1;},Err(e)=>{assert!(matches!(e,ReplayTerminalFailure::Resource(_)));assert!(count>1);assert_eq!(work.finish(),Err(e));let hits=work.hits;let used=work.used();assert!(p.paid_copy(work).is_err());assert_eq!(work.used(),used);assert_eq!(work.hits,hits);break;}}}},
 CodecFixtureCase::RejectDuplicate=>reject::<Projection>(br#"{"cash":0,"lots":[],"marks":{},"fees":0,"realized_pnl":0,"seed_equity":0,"as_of":"2026-10-04T00:00:00Z","closes":{},"economic_unavailable":null,"economic_unavailable":null}"#,work),
 CodecFixtureCase::RejectSequence=>reject::<AccountBinding>(br#"["a","e"]"#,work),
 CodecFixtureCase::RejectNumeric=>{let bytes=fixture(&Projection::SHAPE,Form::default(),Origin::Direct).replacen("\"cash\":1","\"cash\":9223372036854775808",1);reject::<Projection>(bytes.as_bytes(),work);},
 CodecFixtureCase::RejectDate=>{let bytes=fixture(&Projection::SHAPE,Form::default(),Origin::Direct).replace("2026-10-04T01:02:03Z","not-a-date");reject::<Projection>(bytes.as_bytes(),work);},
 CodecFixtureCase::RejectOutputBudget=>{let bytes=vec![b'x';16*1024*1024];let s=std::str::from_utf8(&bytes).unwrap();let value=work.string(s).unwrap();let hits=work.hits;assert!(work.output(1).is_err());assert_eq!(work.hits,hits);assert!(work.string(&value).is_err());},
 CodecFixtureCase::RejectBufferedIgnored|CodecFixtureCase::RejectTypedDepth|CodecFixtureCase::RejectDirectUnitMap|CodecFixtureCase::RejectAdjacentUnitMap=>paper_book_v2_execution::replay_codec_fixtures(case,work),
 CodecFixtureCase::RejectNoncanonical|CodecFixtureCase::RejectUnknownCommand=>paper_book_v2_execution::replay_codec_fixtures(case,work),
 CodecFixtureCase::Unit(..)|CodecFixtureCase::Boundary(_)|CodecFixtureCase::Compatibility(_)|CodecFixtureCase::SeedFault(_)|CodecFixtureCase::Branch(_)|CodecFixtureCase::OrderedScratch|CodecFixtureCase::UnitScratch(_)|CodecFixtureCase::GenericControl(_)|CodecFixtureCase::OptionPresence(..)|CodecFixtureCase::ReachedLeaf(..)|CodecFixtureCase::AdjacentUnit(..)=>unreachable!("fixed dispatcher"),
 CodecFixtureCase::Qualification=>panic!("handled inside foundation without mechanics authority"),
}
}
macro_rules! fixed_test {
    ($name:ident,$case:ident) => {
        #[test]
        fn $name() {
            crate::database::global_schema_v1::replay_work::codec_fixture(CodecFixtureCase::$case);
        }
    };
}
fixed_test!(all_nine_nonempty_roots_match_original, AllRoots);
fixed_test!(all_reached_enum_variants_match_original, Variants);
fixed_test!(
    buffered_external_unit_maps_propagate_to_nested_fields,
    BufferedUnits
);
fixed_test!(ordinary_positional_arrays_match_original, ScalarBoundaries);
fixed_test!(recursive_copies_pay_grown_collections, Copies);
fixed_test!(escaped_unicode_scratch_and_payload_are_paid, Scratch);
fixed_test!(
    ignored_opaque_depth_numbers_surrogates_and_projection_default,
    UnknownContent
);
fixed_test!(canonical_bytes_and_original_hash_domains, CanonicalHashes);
fixed_test!(
    repeated_copy_exhaustion_is_cumulative_and_latched,
    Cumulative
);
fixed_test!(duplicate_null_is_not_absent, RejectDuplicate);
fixed_test!(missing_positional_slot_is_rejected, RejectSequence);
fixed_test!(integer_range_error_preserves_first_terminal, RejectNumeric);
fixed_test!(chrono_error_preserves_first_terminal, RejectDate);
fixed_test!(
    exact_string_budget_then_next_output_refuses_before_allocation,
    RejectOutputBudget
);
fixed_test!(
    content_first_ignored_overflow_rejects_before_primary,
    RejectBufferedIgnored
);
fixed_test!(content_first_depth_rejects_before_primary, RejectTypedDepth);
fixed_test!(
    direct_external_unit_empty_map_is_rejected,
    RejectDirectUnitMap
);
fixed_test!(
    buffered_adjacent_unit_empty_map_is_rejected,
    RejectAdjacentUnitMap
);
fixed_test!(unissued_production_layout_still_refuses, Qualification);

fixed_test!(
    buffered_lower_acceptance_still_rejects_noncanonical_record,
    RejectNoncanonical
);
fixed_test!(
    command_content_preserves_original_unknown_field_rejection,
    RejectUnknownCommand
);

// Fixed fields/contexts, with an independently chosen nested alternative where
// a shared modulo would hide an actual branch. No runtime policy/limit input.
pub(super) fn snapshot_fixture() -> Vec<u8> {
    let body = fixture(
        &<paper_ledger::SnapshotRevision as Value>::SHAPE,
        Form {
            choice: 4,
            ..Form::default()
        },
        Origin::Direct,
    );
    format!("{{\"DerivedSnapshotV1\":{body}}}").into_bytes()
}
fn replace_field(input: &str, key: &str, value: &str) -> String {
    let marker = format!("{}:", quote(key));
    let start = input.find(&marker).expect("fixed field exists") + marker.len();
    let span = shapes::extent(input.as_bytes(), start);
    format!("{}{}{}", &input[..span.start], value, &input[span.end..])
}
fn oracle<T: Root + serde::de::DeserializeOwned>(
    bytes: &[u8],
    accepted: bool,
    work: &mut CodecMechanics<'_, '_>,
) {
    let old = serde_json::from_slice::<T>(bytes);
    assert_eq!(old.is_ok(), accepted, "original lower acceptance");
    if !accepted {
        reject::<T>(bytes, work);
        return;
    }
    let expected = serde_json::to_vec(&old.unwrap()).unwrap();
    let decoded = decode_core::<T>(bytes, work).unwrap();
    assert_eq!(encode_core(&decoded, work).unwrap(), expected);
    let copied = decoded.paid_copy(work).unwrap();
    assert_eq!(encode_core(&copied, work).unwrap(), expected);
    assert!(
        decode_record_core::<T>(&expected, work).is_ok(),
        "original canonical bytes accepted"
    );
    if T::CANONICAL && bytes != expected {
        assert!(decode_record_core::<T>(bytes, work).is_err());
        assert_eq!(work.failure_kind(), Some(K::Noncanonical));
    }
}
fn all_owners(case: CodecFixtureCase, work: &mut CodecMechanics<'_, '_>) {
    paper_ledger::replay_codec_fixtures(case, work);
    paper_book_v2::replay_codec_fixtures(case, work);
    paper_book_v2_execution::replay_codec_fixtures(case, work);
}
fn special_run(case: CodecFixtureCase, work: &mut CodecMechanics<'_, '_>) -> bool {
    match case {
        CodecFixtureCase::Boundary(_) => unreachable!("foundation handles finite boundaries"),
        CodecFixtureCase::Compatibility(CompatCase::FloatNonfiniteSerialize) => {
            paper_ledger::replay_codec_nonfinite(work);
            true
        }
        CodecFixtureCase::Unit(..)
        | CodecFixtureCase::Compatibility(_)
        | CodecFixtureCase::SeedFault(_)
        | CodecFixtureCase::Branch(_)
        | CodecFixtureCase::UnitScratch(_)
        | CodecFixtureCase::GenericControl(_)
        | CodecFixtureCase::OptionPresence(..)
        | CodecFixtureCase::ReachedLeaf(..)
        | CodecFixtureCase::AdjacentUnit(..) => {
            all_owners(case, work);
            true
        }
        CodecFixtureCase::OrderedScratch => {
            let bytes = shapes::ordered_scratch_bytes();
            let decoded = decode_core::<AccountBinding>(&bytes, work).unwrap();
            let old: AccountBinding = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(decoded, old);
            let t = work.scratch;
            assert_eq!(&t.requests[..t.count], &[9, 18, 36, 72]);
            assert_eq!(t.sum, 135);
            assert_eq!(t.capacity, 72);
            let unicode = bytes.windows(6).position(|w| w == br"\u4E2D").unwrap();
            assert_eq!(
                t.offsets[2],
                unicode + 5,
                "reserve4 must grow before emitting a 3-byte scalar at len15/cap18"
            );
            let ignored = bytes.iter().position(|b| *b == b'[').unwrap();
            assert_eq!(
                t.offsets[3],
                ignored + 37,
                "ignored workspace clears len but retains cap36"
            );
            assert!(t.clears >= 8);
            assert_eq!(work.hits.decoder, 1);
            assert_eq!(work.hits.scratch_plans, 1);
            true
        }
        _ => false,
    }
}
fn special_root<T: Root + serde::de::DeserializeOwned>(
    case: CodecFixtureCase,
    work: &mut CodecMechanics<'_, '_>,
) -> bool {
    match case {
        CodecFixtureCase::OptionPresence(field, form) => {
            option_presence::<T>(field, form, work);
            return true;
        }
        CodecFixtureCase::ReachedLeaf(leaf, origin) => {
            reached_leaf::<T>(leaf, origin, work);
            return true;
        }
        CodecFixtureCase::AdjacentUnit(effect, form, origin) => {
            adjacent_unit::<T>(effect, form, origin, work);
            return true;
        }
        CodecFixtureCase::Branch(branch) => {
            let v1 = matches!(
                branch,
                BranchCase::V1Order | BranchCase::V1Adjudication | BranchCase::V1Snapshot
            );
            if !(if v1 {
                matches!(T::ROOT, RootKind::V1Fact)
            } else {
                matches!(T::ROOT, RootKind::ExecutionFact)
            }) {
                return true;
            }
            let choice = match branch {
                BranchCase::V1Order | BranchCase::ExecutionSubmit => 1,
                BranchCase::V1Adjudication | BranchCase::ExecutionFill => 3,
                BranchCase::V1Snapshot => 4,
                BranchCase::ExecutionEvaluate => 2,
                BranchCase::BufferedOtherDisposition => 0,
            };
            let mut bytes = fixture(
                &T::SHAPE,
                Form {
                    choice,
                    buffered: true,
                    unit_maps: true,
                    width: 3,
                    ..Form::default()
                },
                Origin::Direct,
            );
            if matches!(branch, BranchCase::BufferedOtherDisposition) {
                assert!(bytes.contains("AllocatedToStrategy"));
                bytes = bytes.replace("AllocatedToStrategy", "UnassignedReadOnly");
            }
            paired::<T>(bytes.as_bytes(), work, true);
            assert!(work.hits.strings > 0);
            if matches!(
                branch,
                BranchCase::ExecutionFill | BranchCase::ExecutionEvaluate
            ) {
                assert_eq!(work.hits.vectors, 0, "selected DTO branch has no Vec");
            } else {
                assert!(work.hits.vectors > 0);
            }
            return true;
        }
        CodecFixtureCase::SeedFault(kind) => {
            let map = matches!(kind, SeedFaultCase::Map);
            if !(if map {
                matches!(T::ROOT, RootKind::ExecutionProjection)
            } else {
                matches!(T::ROOT, RootKind::ExecutionFact)
            }) {
                return true;
            }
            let bytes = fixture(
                &T::SHAPE,
                Form {
                    buffered: true,
                    unit_maps: true,
                    ..Form::default()
                },
                Origin::Direct,
            );
            assert!(serde_json::from_str::<T>(&bytes).is_ok());
            let error = decode_core::<T>(bytes.as_bytes(), work).err().unwrap();
            assert!(matches!(error, ReplayTerminalFailure::Resource(_)));
            assert!(work.seeded_failure_matches(kind));
            assert_eq!(work.hits.decoder, 1);
            assert!(work.hits.denial_depth >= 2);
            assert!(work.hits.denial_strings > 0);
            assert_eq!(work.hits.active_seeds, 0);
            match kind {
                SeedFaultCase::UnitMap => {
                    assert_eq!(work.hits.denial_units, 2);
                    assert!(work.hits.denial_depth >= 6);
                    assert_eq!(work.hits.strings, work.hits.denial_strings);
                }
                SeedFaultCase::StringAfterUnit => {
                    assert!(work.hits.denial_units > 0);
                    assert_eq!(work.hits.strings, work.hits.denial_strings);
                }
                SeedFaultCase::Vector => assert_eq!(work.hits.vectors, work.hits.denial_vectors),
                SeedFaultCase::Map => assert_eq!(work.hits.maps, work.hits.denial_maps),
            }
            let hits = work.hits;
            let used = work.used();
            assert_eq!(decode_core::<T>(bytes.as_bytes(), work).err(), Some(error));
            assert_eq!(work.used(), used);
            assert_eq!(work.hits, hits);
            return true;
        }
        CodecFixtureCase::GenericControl(control) => {
            if !matches!(T::ROOT, RootKind::ExecutionFact) {
                return true;
            }
            let mut bytes = fixture(
                &T::SHAPE,
                Form {
                    buffered: true,
                    ..Form::default()
                },
                Origin::Direct,
            );
            let value = match control {
                GenericControlCase::Finite => "1",
                GenericControlCase::Shallow => "[0]",
                GenericControlCase::InvalidUtf8 => "\"BYTE\"",
            };
            bytes = bytes.replacen(
                "\"manifest\":",
                &format!("\"ignored\":{},\"manifest\":", value),
                1,
            );
            let mut bytes = bytes.into_bytes();
            if matches!(control, GenericControlCase::InvalidUtf8) {
                let pos = bytes.windows(4).position(|w| w == b"BYTE").unwrap();
                bytes.splice(pos..pos + 4, [0xff]);
            }
            reject::<T>(&bytes, work);
            assert_eq!(
                work.failure_kind(),
                Some(if matches!(control, GenericControlCase::InvalidUtf8) {
                    K::MalformedJson
                } else {
                    K::UnknownField
                })
            );
            assert_eq!(work.hits.decoder, 0);
            return true;
        }
        CodecFixtureCase::Unit(family, wire, origin) => {
            if !matches!(T::ROOT, RootKind::ExecutionFact) {
                return true;
            }
            let (choice, field, tag) = match family {
                UnitFamily::ProfitPolicy => {
                    (0, "profit_policy", "ReinvestWithinFixedAuthorizedBudget")
                }
                UnitFamily::LotDisposition => (0, "disposition", "UnassignedReadOnly"),
                UnitFamily::Side => (1, "side", "Sell"),
                UnitFamily::TimeInForce => (1, "time_in_force", "DaySession"),
                UnitFamily::ParentStatus => (1, "status", "PartiallyFilled"),
                UnitFamily::NoFillReason => (2, "record", "LessThanWholeLot"),
            };
            let buffered = matches!(origin, UnitOrigin::Buffered);
            let bytes = fixture(
                &T::SHAPE,
                Form {
                    choice,
                    buffered,
                    ..Form::default()
                },
                Origin::Direct,
            );
            let value = match wire {
                UnitRepresentation::Bare => quote(tag),
                UnitRepresentation::Null => format!("{{{}:null}}", quote(tag)),
                UnitRepresentation::EmptyMap => format!("{{{}:{{}}}}", quote(tag)),
                UnitRepresentation::NonemptyMap => format!("{{{}:{{\"x\":0}}}}", quote(tag)),
                UnitRepresentation::Array => format!("{{{}:[]}}", quote(tag)),
            };
            let bytes = replace_field(&bytes, field, &value);
            let accepted = matches!(wire, UnitRepresentation::Bare | UnitRepresentation::Null)
                || (buffered && matches!(wire, UnitRepresentation::EmptyMap));
            oracle::<T>(bytes.as_bytes(), accepted, work);
            return true;
        }
        CodecFixtureCase::UnitScratch(wire) => {
            if !matches!(T::ROOT, RootKind::ExecutionFact) {
                return true;
            }
            let bytes = fixture(
                &T::SHAPE,
                Form {
                    choice: 2,
                    buffered: true,
                    ..Form::default()
                },
                Origin::Direct,
            );
            let value = match wire {
                UnitWireCase::Bare => "\"LessThanWholeLot\"",
                UnitWireCase::Null => "{\"LessThanWholeLot\":null}",
                UnitWireCase::EmptyMap => "{\"LessThanWholeLot\":{}}",
                UnitWireCase::EscapedMap => "{\"\\u004cessThanWholeLot\":{}}",
            };
            let bytes = replace_field(&bytes, "record", value);
            let old: T = serde_json::from_str(&bytes).unwrap();
            let decoded = decode_core::<T>(bytes.as_bytes(), work).unwrap();
            assert_eq!(
                serde_json::to_vec(&decoded).unwrap(),
                serde_json::to_vec(&old).unwrap()
            );
            assert_eq!(
                work.hits.unit_maps,
                usize::from(matches!(
                    wire,
                    UnitWireCase::EmptyMap | UnitWireCase::EscapedMap
                ))
            );
            let expected = if matches!(wire, UnitWireCase::EscapedMap) {
                24
            } else {
                0
            };
            assert_eq!(work.scratch.sum, expected);
            if expected > 0 {
                assert_eq!(&work.scratch.requests[..work.scratch.count], &[8, 16]);
            }
            return true;
        }
        CodecFixtureCase::Compatibility(c) => {
            compatibility::<T>(c, work);
            return true;
        }
        _ => false,
    }
}
fn compatibility<T: Root + serde::de::DeserializeOwned>(
    case: CompatCase,
    work: &mut CodecMechanics<'_, '_>,
) {
    use CompatCase as C;
    let v1 = matches!(
        case,
        C::FloatOptionNone
            | C::I32Min
            | C::I32Max
            | C::I32Under
            | C::I32Over
            | C::FloatNegativeZero
            | C::FloatSubnormal
            | C::FloatExponent
            | C::FloatOverflow
    );
    let manifest = matches!(case, C::U8Max | C::U8Over | C::U8Negative);
    let binding = matches!(case, C::IgnoredUtf8 | C::TypedUtf8);
    let fact = matches!(
        case,
        C::UnitDirectNull
            | C::UnitBufferedNull
            | C::UnitDirectMap
            | C::UnitBufferedMap
            | C::UnitDirectNonempty
            | C::UnitDirectArray
            | C::UnitBufferedNonempty
            | C::UnitBufferedArray
            | C::AdjacentAbsent
            | C::AdjacentNull
            | C::AdjacentArray
            | C::AdjacentArrayMissing
            | C::AdjacentArrayExtra
            | C::AdjacentNumericTag
            | C::AdjacentMapTag
            | C::AdjacentContentArray
    );
    let exec = matches!(case, C::BoolFalse | C::BoolWrong);
    let selected = if matches!(case, C::OptionalDateNone) {
        matches!(T::ROOT, RootKind::Seed)
    } else if v1 {
        matches!(T::ROOT, RootKind::V1Fact)
    } else if manifest {
        matches!(T::ROOT, RootKind::ExecutionManifest)
    } else if binding {
        matches!(T::ROOT, RootKind::Binding)
    } else if fact {
        matches!(T::ROOT, RootKind::ExecutionFact)
    } else if exec {
        matches!(T::ROOT, RootKind::ExecutionProjection)
    } else {
        matches!(T::ROOT, RootKind::Projection)
    };
    if !selected {
        return;
    }
    let choice = if v1 {
        4
    } else if matches!(
        case,
        C::UnitDirectNull
            | C::UnitBufferedNull
            | C::UnitDirectMap
            | C::UnitBufferedMap
            | C::UnitDirectNonempty
            | C::UnitDirectArray
            | C::UnitBufferedNonempty
            | C::UnitBufferedArray
    ) {
        2
    } else {
        0
    };
    let buffered = matches!(
        case,
        C::UnitBufferedNull | C::UnitBufferedMap | C::UnitBufferedNonempty | C::UnitBufferedArray
    );
    let mut bytes = fixture(
        &T::SHAPE,
        Form {
            choice,
            buffered,
            ..Form::default()
        },
        Origin::Direct,
    );
    let mut accepted = true;
    let edit = match case {
        C::I64Min => Some(("cash", "-9223372036854775808")),
        C::I64Max => Some(("cash", "9223372036854775807")),
        C::I64Under => {
            accepted = false;
            Some(("cash", "-9223372036854775809"))
        }
        C::I64Over => {
            accepted = false;
            Some(("cash", "9223372036854775808"))
        }
        C::U32Zero => Some(("quantity", "0")),
        C::U32Max => Some(("quantity", "4294967295")),
        C::U32Over => {
            accepted = false;
            Some(("quantity", "4294967296"))
        }
        C::U32Negative => {
            accepted = false;
            Some(("quantity", "-1"))
        }
        C::I32Min => Some(("id", "-2147483648")),
        C::I32Max => Some(("id", "2147483647")),
        C::I32Under => {
            accepted = false;
            Some(("id", "-2147483649"))
        }
        C::I32Over => {
            accepted = false;
            Some(("id", "2147483648"))
        }
        C::U8Max => Some(("fee_descriptor", "[0,255]")),
        C::U8Over => {
            accepted = false;
            Some(("fee_descriptor", "[256]"))
        }
        C::U8Negative => {
            accepted = false;
            Some(("fee_descriptor", "[-1]"))
        }
        C::FloatNegativeZero => Some(("total_pnl", "-0.0")),
        C::FloatSubnormal => Some(("total_pnl", "5e-324")),
        C::FloatExponent => Some(("total_pnl", "1e-300")),
        C::FloatOverflow => {
            accepted = false;
            Some(("total_pnl", "1e99999"))
        }
        C::BoolFalse => Some(("listed", "false")),
        C::BoolWrong => {
            accepted = false;
            Some(("listed", "1"))
        }
        C::OptionsNone => Some(("reported_cost", "null")),
        C::OptionalDateNone => Some(("sellable_from", "null")),
        C::FloatOptionNone => Some(("sharpe_ratio", "null")),
        C::DateRelaxed => Some(("acquired_on", "\"2026-1-4\"")),
        C::DateInvalid => {
            accepted = false;
            Some(("acquired_on", "\"2026-02-30\""))
        }
        C::DateTimeOffset => Some(("as_of", "\"2026-10-04T09:02:03+08:00\"")),
        _ => None,
    };
    if let Some((key, value)) = edit {
        bytes = replace_field(&bytes, key, value);
    }
    match case {
        C::OptionsNone => {
            bytes = replace_field(&bytes, "economic_unavailable", "null");
        }
        C::DuplicateDecodedMap => {
            let value = fixture(
                &<paper_ledger::Mark as Value>::SHAPE,
                Form::default(),
                Origin::Direct,
            );
            let one = replace_field(&bytes, "marks", &format!("{{\"A\":{value}}}"));
            let before = work.used();
            let _ = decode_core::<T>(one.as_bytes(), work).unwrap();
            let cost = work.used() - before;
            let hit = work.hits.maps;
            let later = replace_field(&value, "price", "2");
            bytes = replace_field(
                &bytes,
                "marks",
                &format!("{{\"A\":{value},\"\\u0041\":{later}}}"),
            );
            let before = work.used();
            let new = decode_core::<T>(bytes.as_bytes(), work).unwrap();
            assert!(work.used() - before > cost);
            assert_eq!(work.hits.maps - hit, 3);
            let original: T = serde_json::from_str(&bytes).unwrap();
            assert_eq!(
                serde_json::to_vec(&new).unwrap(),
                serde_json::to_vec(&original).unwrap()
            );
        }
        C::IgnoredUtf8 | C::TypedUtf8 => {
            let mut raw = if matches!(case, C::IgnoredUtf8) {
                bytes.replacen("}", ",\"ignored\":\"BYTE\"}", 1)
            } else {
                accepted = false;
                replace_field(&bytes, "account_id", "\"BYTE\"")
            }
            .into_bytes();
            let at = raw.windows(4).position(|w| w == b"BYTE").unwrap();
            raw.splice(at..at + 4, [0xff]);
            oracle::<T>(&raw, accepted, work);
            return;
        }
        C::UnitDirectNull | C::UnitBufferedNull => {
            bytes = replace_field(&bytes, "record", "{\"LessThanWholeLot\":null}")
        }
        C::UnitDirectMap | C::UnitBufferedMap => {
            bytes = replace_field(&bytes, "record", "{\"LessThanWholeLot\":{}}");
            accepted = matches!(case, C::UnitBufferedMap);
        }
        C::UnitDirectNonempty | C::UnitBufferedNonempty => {
            bytes = replace_field(&bytes, "record", "{\"LessThanWholeLot\":{\"x\":0}}");
            accepted = false;
        }
        C::UnitDirectArray | C::UnitBufferedArray => {
            bytes = replace_field(&bytes, "record", "{\"LessThanWholeLot\":[]}");
            accepted = false;
        }
        C::AdjacentAbsent => bytes = replace_field(&bytes, "effect", "{\"effect\":\"Opened\"}"),
        C::AdjacentNull => {}
        C::AdjacentArray => bytes = replace_field(&bytes, "effect", "[\"Opened\",null]"),
        C::AdjacentArrayMissing => {
            bytes = replace_field(&bytes, "effect", "[\"Opened\"]");
            accepted = false;
        }
        C::AdjacentArrayExtra => {
            bytes = replace_field(&bytes, "effect", "[\"Opened\",null,0]");
            accepted = false;
        }
        C::AdjacentNumericTag => {
            bytes = replace_field(&bytes, "effect", "[0,null]");
            accepted = false;
        }
        C::AdjacentMapTag => {
            bytes = replace_field(
                &bytes,
                "effect",
                "{\"effect\":{\"Opened\":null},\"record\":null}",
            )
        }
        C::AdjacentContentArray => {
            let manifest = fixture(
                &<paper_book_v2_execution::ExecutionManifest as Value>::SHAPE,
                Form::default(),
                Origin::Direct,
            );
            bytes = replace_field(
                &bytes,
                "request",
                &format!("{{\"operation\":\"Open\",\"request\":[{manifest}]}}"),
            );
            accepted = false;
        }
        _ => {}
    }
    oracle::<T>(bytes.as_bytes(), accepted, work);
}

macro_rules! finite_test {
    ($name:ident,$case:expr) => {
        #[test]
        fn $name() {
            crate::database::global_schema_v1::replay_work::codec_fixture($case);
        }
    };
}
finite_test!(
    boundary_frames_exact,
    CodecFixtureCase::Boundary(BoundaryCase::FramesExact)
);
finite_test!(
    boundary_frames_short,
    CodecFixtureCase::Boundary(BoundaryCase::FramesShort)
);
finite_test!(
    boundary_escrow8_exact,
    CodecFixtureCase::Boundary(BoundaryCase::Escrow8Exact)
);
finite_test!(
    boundary_escrow8_short,
    CodecFixtureCase::Boundary(BoundaryCase::Escrow8Short)
);
finite_test!(
    boundary_escrow4_exact,
    CodecFixtureCase::Boundary(BoundaryCase::Escrow4Exact)
);
finite_test!(
    boundary_escrow4_short,
    CodecFixtureCase::Boundary(BoundaryCase::Escrow4Short)
);
finite_test!(
    boundary_scratch_exact,
    CodecFixtureCase::Boundary(BoundaryCase::ScratchExact)
);
finite_test!(
    boundary_scratch_short,
    CodecFixtureCase::Boundary(BoundaryCase::ScratchShort)
);
finite_test!(
    boundary_vector_exact,
    CodecFixtureCase::Boundary(BoundaryCase::VectorExact)
);
finite_test!(
    boundary_vector_short,
    CodecFixtureCase::Boundary(BoundaryCase::VectorShort)
);
finite_test!(
    boundary_map_exact,
    CodecFixtureCase::Boundary(BoundaryCase::MapExact)
);
finite_test!(
    boundary_map_short,
    CodecFixtureCase::Boundary(BoundaryCase::MapShort)
);
finite_test!(
    boundary_output_exact,
    CodecFixtureCase::Boundary(BoundaryCase::OutputExact)
);
finite_test!(
    boundary_output_short,
    CodecFixtureCase::Boundary(BoundaryCase::OutputShort)
);
finite_test!(
    boundary_length_overflow,
    CodecFixtureCase::Boundary(BoundaryCase::LengthOverflow)
);
finite_test!(
    seeded_resource_string_after_unit,
    CodecFixtureCase::SeedFault(SeedFaultCase::StringAfterUnit)
);
finite_test!(
    seeded_resource_vector,
    CodecFixtureCase::SeedFault(SeedFaultCase::Vector)
);
finite_test!(
    seeded_resource_map,
    CodecFixtureCase::SeedFault(SeedFaultCase::Map)
);
finite_test!(
    independent_copy_buffered_other_disposition,
    CodecFixtureCase::Branch(BranchCase::BufferedOtherDisposition)
);
finite_test!(
    independent_copy_v1_order,
    CodecFixtureCase::Branch(BranchCase::V1Order)
);
finite_test!(
    independent_copy_v1_adjudication,
    CodecFixtureCase::Branch(BranchCase::V1Adjudication)
);
finite_test!(
    independent_copy_v1_snapshot,
    CodecFixtureCase::Branch(BranchCase::V1Snapshot)
);
finite_test!(
    independent_copy_execution_submit,
    CodecFixtureCase::Branch(BranchCase::ExecutionSubmit)
);
finite_test!(
    independent_copy_execution_fill,
    CodecFixtureCase::Branch(BranchCase::ExecutionFill)
);
finite_test!(
    independent_copy_execution_evaluate,
    CodecFixtureCase::Branch(BranchCase::ExecutionEvaluate)
);
finite_test!(
    unit_scratch_bare,
    CodecFixtureCase::UnitScratch(UnitWireCase::Bare)
);
finite_test!(
    unit_scratch_null,
    CodecFixtureCase::UnitScratch(UnitWireCase::Null)
);
finite_test!(
    unit_scratch_empty_map,
    CodecFixtureCase::UnitScratch(UnitWireCase::EmptyMap)
);
finite_test!(
    unit_scratch_escaped_map,
    CodecFixtureCase::UnitScratch(UnitWireCase::EscapedMap)
);
finite_test!(
    generic_control_finite,
    CodecFixtureCase::GenericControl(GenericControlCase::Finite)
);
finite_test!(
    generic_control_shallow,
    CodecFixtureCase::GenericControl(GenericControlCase::Shallow)
);
finite_test!(
    generic_control_invalid_utf8,
    CodecFixtureCase::GenericControl(GenericControlCase::InvalidUtf8)
);
finite_test!(
    compatibility_bool_false,
    CodecFixtureCase::Compatibility(CompatCase::BoolFalse)
);
finite_test!(
    compatibility_bool_wrong,
    CodecFixtureCase::Compatibility(CompatCase::BoolWrong)
);
finite_test!(
    compatibility_i64_min,
    CodecFixtureCase::Compatibility(CompatCase::I64Min)
);
finite_test!(
    compatibility_i64_max,
    CodecFixtureCase::Compatibility(CompatCase::I64Max)
);
finite_test!(
    compatibility_i64_under,
    CodecFixtureCase::Compatibility(CompatCase::I64Under)
);
finite_test!(
    compatibility_i64_over,
    CodecFixtureCase::Compatibility(CompatCase::I64Over)
);
finite_test!(
    compatibility_u32_max,
    CodecFixtureCase::Compatibility(CompatCase::U32Max)
);
finite_test!(
    compatibility_u32_over,
    CodecFixtureCase::Compatibility(CompatCase::U32Over)
);
finite_test!(
    compatibility_u32_negative,
    CodecFixtureCase::Compatibility(CompatCase::U32Negative)
);
finite_test!(
    compatibility_i32_min,
    CodecFixtureCase::Compatibility(CompatCase::I32Min)
);
finite_test!(
    compatibility_i32_max,
    CodecFixtureCase::Compatibility(CompatCase::I32Max)
);
finite_test!(
    compatibility_i32_under,
    CodecFixtureCase::Compatibility(CompatCase::I32Under)
);
finite_test!(
    compatibility_i32_over,
    CodecFixtureCase::Compatibility(CompatCase::I32Over)
);
finite_test!(
    compatibility_u8_max,
    CodecFixtureCase::Compatibility(CompatCase::U8Max)
);
finite_test!(
    compatibility_u8_over,
    CodecFixtureCase::Compatibility(CompatCase::U8Over)
);
finite_test!(
    compatibility_u8_negative,
    CodecFixtureCase::Compatibility(CompatCase::U8Negative)
);
finite_test!(
    compatibility_float_negative_zero,
    CodecFixtureCase::Compatibility(CompatCase::FloatNegativeZero)
);
finite_test!(
    compatibility_float_subnormal,
    CodecFixtureCase::Compatibility(CompatCase::FloatSubnormal)
);
finite_test!(
    compatibility_float_exponent,
    CodecFixtureCase::Compatibility(CompatCase::FloatExponent)
);
finite_test!(
    compatibility_float_overflow,
    CodecFixtureCase::Compatibility(CompatCase::FloatOverflow)
);
finite_test!(
    compatibility_float_nonfinite_serialize,
    CodecFixtureCase::Compatibility(CompatCase::FloatNonfiniteSerialize)
);
finite_test!(
    compatibility_options_none,
    CodecFixtureCase::Compatibility(CompatCase::OptionsNone)
);
finite_test!(
    compatibility_date_relaxed,
    CodecFixtureCase::Compatibility(CompatCase::DateRelaxed)
);
finite_test!(
    compatibility_date_invalid,
    CodecFixtureCase::Compatibility(CompatCase::DateInvalid)
);
finite_test!(
    compatibility_date_time_offset,
    CodecFixtureCase::Compatibility(CompatCase::DateTimeOffset)
);
finite_test!(
    compatibility_duplicate_decoded_map,
    CodecFixtureCase::Compatibility(CompatCase::DuplicateDecodedMap)
);
finite_test!(
    compatibility_ignored_utf8,
    CodecFixtureCase::Compatibility(CompatCase::IgnoredUtf8)
);
finite_test!(
    compatibility_typed_utf8,
    CodecFixtureCase::Compatibility(CompatCase::TypedUtf8)
);
finite_test!(
    compatibility_unit_direct_null,
    CodecFixtureCase::Compatibility(CompatCase::UnitDirectNull)
);
finite_test!(
    compatibility_unit_buffered_null,
    CodecFixtureCase::Compatibility(CompatCase::UnitBufferedNull)
);
finite_test!(
    compatibility_unit_direct_map,
    CodecFixtureCase::Compatibility(CompatCase::UnitDirectMap)
);
finite_test!(
    compatibility_unit_buffered_map,
    CodecFixtureCase::Compatibility(CompatCase::UnitBufferedMap)
);
finite_test!(
    compatibility_unit_buffered_nonempty,
    CodecFixtureCase::Compatibility(CompatCase::UnitBufferedNonempty)
);
finite_test!(
    compatibility_unit_buffered_array,
    CodecFixtureCase::Compatibility(CompatCase::UnitBufferedArray)
);
finite_test!(
    compatibility_adjacent_absent,
    CodecFixtureCase::Compatibility(CompatCase::AdjacentAbsent)
);
finite_test!(
    compatibility_adjacent_null,
    CodecFixtureCase::Compatibility(CompatCase::AdjacentNull)
);
finite_test!(
    compatibility_adjacent_array,
    CodecFixtureCase::Compatibility(CompatCase::AdjacentArray)
);
finite_test!(
    compatibility_adjacent_array_missing,
    CodecFixtureCase::Compatibility(CompatCase::AdjacentArrayMissing)
);
finite_test!(
    compatibility_adjacent_array_extra,
    CodecFixtureCase::Compatibility(CompatCase::AdjacentArrayExtra)
);
finite_test!(
    compatibility_adjacent_numeric_tag,
    CodecFixtureCase::Compatibility(CompatCase::AdjacentNumericTag)
);
finite_test!(
    compatibility_adjacent_map_tag,
    CodecFixtureCase::Compatibility(CompatCase::AdjacentMapTag)
);
finite_test!(
    compatibility_adjacent_content_array,
    CodecFixtureCase::Compatibility(CompatCase::AdjacentContentArray)
);
finite_test!(
    ordered_scratch_full_requests_retain_capacity,
    CodecFixtureCase::OrderedScratch
);
#[test]
fn unit_origin_profit_policy() {
    for (wire, origin) in [
        (UnitRepresentation::Bare, UnitOrigin::Direct),
        (UnitRepresentation::Bare, UnitOrigin::Buffered),
        (UnitRepresentation::Null, UnitOrigin::Direct),
        (UnitRepresentation::Null, UnitOrigin::Buffered),
        (UnitRepresentation::EmptyMap, UnitOrigin::Direct),
        (UnitRepresentation::EmptyMap, UnitOrigin::Buffered),
    ] {
        crate::database::global_schema_v1::replay_work::codec_fixture(CodecFixtureCase::Unit(
            UnitFamily::ProfitPolicy,
            wire,
            origin,
        ));
    }
}
#[test]
fn unit_origin_lot_disposition() {
    for (wire, origin) in [
        (UnitRepresentation::Bare, UnitOrigin::Direct),
        (UnitRepresentation::Bare, UnitOrigin::Buffered),
        (UnitRepresentation::Null, UnitOrigin::Direct),
        (UnitRepresentation::Null, UnitOrigin::Buffered),
        (UnitRepresentation::EmptyMap, UnitOrigin::Direct),
        (UnitRepresentation::EmptyMap, UnitOrigin::Buffered),
    ] {
        crate::database::global_schema_v1::replay_work::codec_fixture(CodecFixtureCase::Unit(
            UnitFamily::LotDisposition,
            wire,
            origin,
        ));
    }
}
#[test]
fn unit_origin_side() {
    for (wire, origin) in [
        (UnitRepresentation::Bare, UnitOrigin::Direct),
        (UnitRepresentation::Bare, UnitOrigin::Buffered),
        (UnitRepresentation::Null, UnitOrigin::Direct),
        (UnitRepresentation::Null, UnitOrigin::Buffered),
        (UnitRepresentation::EmptyMap, UnitOrigin::Direct),
        (UnitRepresentation::EmptyMap, UnitOrigin::Buffered),
    ] {
        crate::database::global_schema_v1::replay_work::codec_fixture(CodecFixtureCase::Unit(
            UnitFamily::Side,
            wire,
            origin,
        ));
    }
}
#[test]
fn unit_origin_time_in_force() {
    for (wire, origin) in [
        (UnitRepresentation::Bare, UnitOrigin::Direct),
        (UnitRepresentation::Bare, UnitOrigin::Buffered),
        (UnitRepresentation::Null, UnitOrigin::Direct),
        (UnitRepresentation::Null, UnitOrigin::Buffered),
        (UnitRepresentation::EmptyMap, UnitOrigin::Direct),
        (UnitRepresentation::EmptyMap, UnitOrigin::Buffered),
    ] {
        crate::database::global_schema_v1::replay_work::codec_fixture(CodecFixtureCase::Unit(
            UnitFamily::TimeInForce,
            wire,
            origin,
        ));
    }
}
#[test]
fn unit_origin_parent_status() {
    for (wire, origin) in [
        (UnitRepresentation::Bare, UnitOrigin::Direct),
        (UnitRepresentation::Bare, UnitOrigin::Buffered),
        (UnitRepresentation::Null, UnitOrigin::Direct),
        (UnitRepresentation::Null, UnitOrigin::Buffered),
        (UnitRepresentation::EmptyMap, UnitOrigin::Direct),
        (UnitRepresentation::EmptyMap, UnitOrigin::Buffered),
    ] {
        crate::database::global_schema_v1::replay_work::codec_fixture(CodecFixtureCase::Unit(
            UnitFamily::ParentStatus,
            wire,
            origin,
        ));
    }
}
#[test]
fn unit_origin_no_fill_reason() {
    for (wire, origin) in [
        (UnitRepresentation::Bare, UnitOrigin::Direct),
        (UnitRepresentation::Bare, UnitOrigin::Buffered),
        (UnitRepresentation::Null, UnitOrigin::Direct),
        (UnitRepresentation::Null, UnitOrigin::Buffered),
        (UnitRepresentation::EmptyMap, UnitOrigin::Direct),
        (UnitRepresentation::EmptyMap, UnitOrigin::Buffered),
    ] {
        crate::database::global_schema_v1::replay_work::codec_fixture(CodecFixtureCase::Unit(
            UnitFamily::NoFillReason,
            wire,
            origin,
        ));
    }
}

finite_test!(
    compatibility_u32_zero,
    CodecFixtureCase::Compatibility(CompatCase::U32Zero)
);
finite_test!(
    compatibility_optional_date_none,
    CodecFixtureCase::Compatibility(CompatCase::OptionalDateNone)
);
finite_test!(
    compatibility_float_option_none,
    CodecFixtureCase::Compatibility(CompatCase::FloatOptionNone)
);

finite_test!(
    compatibility_unit_direct_nonempty,
    CodecFixtureCase::Compatibility(CompatCase::UnitDirectNonempty)
);
finite_test!(
    compatibility_unit_direct_array,
    CodecFixtureCase::Compatibility(CompatCase::UnitDirectArray)
);

finite_test!(
    seeded_resource_unit_map_q8_unwind,
    CodecFixtureCase::SeedFault(SeedFaultCase::UnitMap)
);

fn option_presence<T: Root + serde::de::DeserializeOwned>(
    field: OptionField,
    form: OptionForm,
    work: &mut CodecMechanics<'_, '_>,
) {
    let (selected, shape, missing) = match field {
        OptionField::LotReportedCost => (
            matches!(T::ROOT, RootKind::Projection),
            &<paper_ledger::Lot as Value>::SHAPE,
            "reported_cost",
        ),
        OptionField::SeedSellableFrom => (
            matches!(T::ROOT, RootKind::Seed),
            &<paper_ledger::SeedLot as Value>::SHAPE,
            "sellable_from",
        ),
    };
    if !selected {
        return;
    }
    let Shape::Record(fields, _, true) = *shape else {
        panic!("fixed positional record");
    };
    let slot = fields.iter().position(|f| f.name == missing).unwrap();
    assert!(matches!(fields[slot].shape, Shape::Option(_)));
    assert!(
        fields[slot].optional,
        "ordinary object Option may be absent"
    );
    assert!(
        !fields[slot].positional_default,
        "this is not a serde(default) field"
    );
    let positional = matches!(
        form,
        OptionForm::PositionalNull | OptionForm::PositionalMissing
    );
    let parts = fields
        .iter()
        .enumerate()
        .filter_map(|(index, field)| {
            if (matches!(form, OptionForm::ObjectAbsent) && index == slot)
                || (matches!(form, OptionForm::PositionalMissing) && index >= slot)
            {
                return None;
            }
            let value = if index == slot {
                "null".into()
            } else {
                fixture(field.shape, Form::default(), Origin::Direct)
            };
            Some(if positional {
                value
            } else {
                format!("{}:{}", quote(field.name), value)
            })
        })
        .collect::<Vec<_>>();
    let lot = if positional {
        format!("[{}]", parts.join(","))
    } else {
        format!("{{{}}}", parts.join(","))
    };
    let base = fixture(&T::SHAPE, Form::default(), Origin::Direct);
    let bytes = replace_field(&base, "lots", &format!("[{lot}]"));
    // In SeedLot the missing slot is followed by another required Option slot;
    // truncating at this exact slot makes sellable_from the first missing field.
    oracle::<T>(
        bytes.as_bytes(),
        !matches!(form, OptionForm::PositionalMissing),
        work,
    );
    if matches!(form, OptionForm::PositionalMissing) {
        assert_eq!(work.failure_kind(), Some(K::MissingField));
    }
}

fn reached_leaf<T: Root + serde::de::DeserializeOwned>(
    leaf: ReachedLeafCase,
    origin: UnitOrigin,
    work: &mut CodecMechanics<'_, '_>,
) {
    if !matches!(T::ROOT, RootKind::ExecutionFact) {
        return;
    }
    use ReachedLeafCase as L;
    // Only fields actually reachable through Open/Submit. i32/f64 remain V1-only.
    let (choice, key, value, accepted) = match leaf {
        L::OpenU8Max => (0, "fee_descriptor", "[0,255]", true),
        L::OpenU8Over => (0, "fee_descriptor", "[256]", false),
        L::OpenU32Max => (0, "original_quantity", "4294967295", true),
        L::OpenU32Over => (0, "original_quantity", "4294967296", false),
        L::OpenI64Min => (
            0,
            "authorized_budget_micro_cny",
            "-9223372036854775808",
            true,
        ),
        L::OpenI64Under => (
            0,
            "authorized_budget_micro_cny",
            "-9223372036854775809",
            false,
        ),
        L::OpenDateRelaxed => (0, "effective_from", "\"2026-1-4\"", true),
        L::OpenDateInvalid => (0, "effective_from", "\"2026-02-30\"", false),
        L::OpenNullOption => (0, "chain_id", "null", true),
        L::SubmitU32Max => (1, "quantity", "4294967295", true),
        L::SubmitU32Over => (1, "quantity", "4294967296", false),
        L::SubmitI64Max => (1, "limit_micro_cny", "9223372036854775807", true),
        L::SubmitI64Over => (1, "limit_micro_cny", "9223372036854775808", false),
        L::SubmitBoolFalse => (1, "listed", "false", true),
        L::SubmitBoolWrong => (1, "listed", "1", false),
        L::SubmitDateRelaxed => (1, "session_date", "\"2026-1-4\"", true),
        L::SubmitDateInvalid => (1, "session_date", "\"2026-02-30\"", false),
    };
    let base = fixture(
        &T::SHAPE,
        Form {
            choice,
            buffered: matches!(origin, UnitOrigin::Buffered),
            ..Form::default()
        },
        Origin::Direct,
    );
    // Edit only CommandRecord content, rather than its Effect's repeated descendants.
    let marker = "\"request\":";
    let start = base.find(marker).unwrap() + marker.len();
    let request = shapes::extent(base.as_bytes(), start);
    let changed = replace_field(&base[request.start..request.end], key, value);
    let bytes = format!(
        "{}{}{}",
        &base[..request.start],
        changed,
        &base[request.end..]
    );
    oracle::<T>(bytes.as_bytes(), accepted, work);
    if !accepted {
        let kind = match leaf {
            L::OpenDateInvalid | L::SubmitDateInvalid => K::InvalidDate,
            L::SubmitBoolWrong => K::UnexpectedType,
            _ => K::IntegerRange,
        };
        assert_eq!(work.failure_kind(), Some(kind));
    }
}

fn adjacent_unit<T: Root + serde::de::DeserializeOwned>(
    effect: AdjacentUnitEffect,
    form: AdjacentUnitForm,
    origin: UnitOrigin,
    work: &mut CodecMechanics<'_, '_>,
) {
    if !matches!(T::ROOT, RootKind::ExecutionFact) {
        return;
    }
    let name = match effect {
        AdjacentUnitEffect::Opened => "Opened",
        AdjacentUnitEffect::Cancelled => "Cancelled",
        AdjacentUnitEffect::Expired => "Expired",
        AdjacentUnitEffect::Marks => "Marks",
    };
    let buffered = matches!(origin, UnitOrigin::Buffered);
    let effect = if matches!(form, AdjacentUnitForm::Absent) {
        // An absent content member has no ordering; both cases still traverse the
        // corresponding direct/content-first surrounding CommandRecord fixture.
        format!("{{\"effect\":{}}}", quote(name))
    } else {
        let value = match form {
            AdjacentUnitForm::Null => "null",
            AdjacentUnitForm::EmptyMap => "{}",
            AdjacentUnitForm::Array => "[]",
            AdjacentUnitForm::NonemptyMap => "{\"x\":0}",
            AdjacentUnitForm::Absent => unreachable!(),
        };
        if buffered {
            format!("{{\"record\":{value},\"effect\":{}}}", quote(name))
        } else {
            format!("{{\"effect\":{},\"record\":{value}}}", quote(name))
        }
    };
    let base = fixture(
        &T::SHAPE,
        Form {
            buffered,
            ..Form::default()
        },
        Origin::Direct,
    );
    let bytes = replace_field(&base, "effect", &effect);
    oracle::<T>(
        bytes.as_bytes(),
        matches!(form, AdjacentUnitForm::Absent | AdjacentUnitForm::Null),
        work,
    );
    if !matches!(form, AdjacentUnitForm::Absent | AdjacentUnitForm::Null) {
        assert_eq!(work.failure_kind(), Some(K::UnexpectedType));
    }
}

#[test]
fn option_presence_lot_reported_cost() {
    for form in [
        OptionForm::ObjectAbsent,
        OptionForm::ObjectNull,
        OptionForm::PositionalNull,
        OptionForm::PositionalMissing,
    ] {
        crate::database::global_schema_v1::replay_work::codec_fixture(
            CodecFixtureCase::OptionPresence(OptionField::LotReportedCost, form),
        );
    }
}

#[test]
fn option_presence_seed_sellable_from() {
    for form in [
        OptionForm::ObjectAbsent,
        OptionForm::ObjectNull,
        OptionForm::PositionalNull,
        OptionForm::PositionalMissing,
    ] {
        crate::database::global_schema_v1::replay_work::codec_fixture(
            CodecFixtureCase::OptionPresence(OptionField::SeedSellableFrom, form),
        );
    }
}

#[test]
fn reached_leaf_open_u8_max() {
    for origin in [UnitOrigin::Direct, UnitOrigin::Buffered] {
        crate::database::global_schema_v1::replay_work::codec_fixture(
            CodecFixtureCase::ReachedLeaf(ReachedLeafCase::OpenU8Max, origin),
        );
    }
}

#[test]
fn reached_leaf_open_u8_over() {
    for origin in [UnitOrigin::Direct, UnitOrigin::Buffered] {
        crate::database::global_schema_v1::replay_work::codec_fixture(
            CodecFixtureCase::ReachedLeaf(ReachedLeafCase::OpenU8Over, origin),
        );
    }
}

#[test]
fn reached_leaf_open_u32_max() {
    for origin in [UnitOrigin::Direct, UnitOrigin::Buffered] {
        crate::database::global_schema_v1::replay_work::codec_fixture(
            CodecFixtureCase::ReachedLeaf(ReachedLeafCase::OpenU32Max, origin),
        );
    }
}

#[test]
fn reached_leaf_open_u32_over() {
    for origin in [UnitOrigin::Direct, UnitOrigin::Buffered] {
        crate::database::global_schema_v1::replay_work::codec_fixture(
            CodecFixtureCase::ReachedLeaf(ReachedLeafCase::OpenU32Over, origin),
        );
    }
}

#[test]
fn reached_leaf_open_i64_min() {
    for origin in [UnitOrigin::Direct, UnitOrigin::Buffered] {
        crate::database::global_schema_v1::replay_work::codec_fixture(
            CodecFixtureCase::ReachedLeaf(ReachedLeafCase::OpenI64Min, origin),
        );
    }
}

#[test]
fn reached_leaf_open_i64_under() {
    for origin in [UnitOrigin::Direct, UnitOrigin::Buffered] {
        crate::database::global_schema_v1::replay_work::codec_fixture(
            CodecFixtureCase::ReachedLeaf(ReachedLeafCase::OpenI64Under, origin),
        );
    }
}

#[test]
fn reached_leaf_open_date_relaxed() {
    for origin in [UnitOrigin::Direct, UnitOrigin::Buffered] {
        crate::database::global_schema_v1::replay_work::codec_fixture(
            CodecFixtureCase::ReachedLeaf(ReachedLeafCase::OpenDateRelaxed, origin),
        );
    }
}

#[test]
fn reached_leaf_open_date_invalid() {
    for origin in [UnitOrigin::Direct, UnitOrigin::Buffered] {
        crate::database::global_schema_v1::replay_work::codec_fixture(
            CodecFixtureCase::ReachedLeaf(ReachedLeafCase::OpenDateInvalid, origin),
        );
    }
}

#[test]
fn reached_leaf_open_null_option() {
    for origin in [UnitOrigin::Direct, UnitOrigin::Buffered] {
        crate::database::global_schema_v1::replay_work::codec_fixture(
            CodecFixtureCase::ReachedLeaf(ReachedLeafCase::OpenNullOption, origin),
        );
    }
}

#[test]
fn reached_leaf_submit_u32_max() {
    for origin in [UnitOrigin::Direct, UnitOrigin::Buffered] {
        crate::database::global_schema_v1::replay_work::codec_fixture(
            CodecFixtureCase::ReachedLeaf(ReachedLeafCase::SubmitU32Max, origin),
        );
    }
}

#[test]
fn reached_leaf_submit_u32_over() {
    for origin in [UnitOrigin::Direct, UnitOrigin::Buffered] {
        crate::database::global_schema_v1::replay_work::codec_fixture(
            CodecFixtureCase::ReachedLeaf(ReachedLeafCase::SubmitU32Over, origin),
        );
    }
}

#[test]
fn reached_leaf_submit_i64_max() {
    for origin in [UnitOrigin::Direct, UnitOrigin::Buffered] {
        crate::database::global_schema_v1::replay_work::codec_fixture(
            CodecFixtureCase::ReachedLeaf(ReachedLeafCase::SubmitI64Max, origin),
        );
    }
}

#[test]
fn reached_leaf_submit_i64_over() {
    for origin in [UnitOrigin::Direct, UnitOrigin::Buffered] {
        crate::database::global_schema_v1::replay_work::codec_fixture(
            CodecFixtureCase::ReachedLeaf(ReachedLeafCase::SubmitI64Over, origin),
        );
    }
}

#[test]
fn reached_leaf_submit_bool_false() {
    for origin in [UnitOrigin::Direct, UnitOrigin::Buffered] {
        crate::database::global_schema_v1::replay_work::codec_fixture(
            CodecFixtureCase::ReachedLeaf(ReachedLeafCase::SubmitBoolFalse, origin),
        );
    }
}

#[test]
fn reached_leaf_submit_bool_wrong() {
    for origin in [UnitOrigin::Direct, UnitOrigin::Buffered] {
        crate::database::global_schema_v1::replay_work::codec_fixture(
            CodecFixtureCase::ReachedLeaf(ReachedLeafCase::SubmitBoolWrong, origin),
        );
    }
}

#[test]
fn reached_leaf_submit_date_relaxed() {
    for origin in [UnitOrigin::Direct, UnitOrigin::Buffered] {
        crate::database::global_schema_v1::replay_work::codec_fixture(
            CodecFixtureCase::ReachedLeaf(ReachedLeafCase::SubmitDateRelaxed, origin),
        );
    }
}

#[test]
fn reached_leaf_submit_date_invalid() {
    for origin in [UnitOrigin::Direct, UnitOrigin::Buffered] {
        crate::database::global_schema_v1::replay_work::codec_fixture(
            CodecFixtureCase::ReachedLeaf(ReachedLeafCase::SubmitDateInvalid, origin),
        );
    }
}

#[test]
fn adjacent_unit_opened_forms_and_orders() {
    for origin in [UnitOrigin::Direct, UnitOrigin::Buffered] {
        for form in [
            AdjacentUnitForm::Absent,
            AdjacentUnitForm::Null,
            AdjacentUnitForm::EmptyMap,
            AdjacentUnitForm::Array,
            AdjacentUnitForm::NonemptyMap,
        ] {
            crate::database::global_schema_v1::replay_work::codec_fixture(
                CodecFixtureCase::AdjacentUnit(AdjacentUnitEffect::Opened, form, origin),
            );
        }
    }
}

#[test]
fn adjacent_unit_cancelled_forms_and_orders() {
    for origin in [UnitOrigin::Direct, UnitOrigin::Buffered] {
        for form in [
            AdjacentUnitForm::Absent,
            AdjacentUnitForm::Null,
            AdjacentUnitForm::EmptyMap,
            AdjacentUnitForm::Array,
            AdjacentUnitForm::NonemptyMap,
        ] {
            crate::database::global_schema_v1::replay_work::codec_fixture(
                CodecFixtureCase::AdjacentUnit(AdjacentUnitEffect::Cancelled, form, origin),
            );
        }
    }
}

#[test]
fn adjacent_unit_expired_forms_and_orders() {
    for origin in [UnitOrigin::Direct, UnitOrigin::Buffered] {
        for form in [
            AdjacentUnitForm::Absent,
            AdjacentUnitForm::Null,
            AdjacentUnitForm::EmptyMap,
            AdjacentUnitForm::Array,
            AdjacentUnitForm::NonemptyMap,
        ] {
            crate::database::global_schema_v1::replay_work::codec_fixture(
                CodecFixtureCase::AdjacentUnit(AdjacentUnitEffect::Expired, form, origin),
            );
        }
    }
}

#[test]
fn adjacent_unit_marks_forms_and_orders() {
    for origin in [UnitOrigin::Direct, UnitOrigin::Buffered] {
        for form in [
            AdjacentUnitForm::Absent,
            AdjacentUnitForm::Null,
            AdjacentUnitForm::EmptyMap,
            AdjacentUnitForm::Array,
            AdjacentUnitForm::NonemptyMap,
        ] {
            crate::database::global_schema_v1::replay_work::codec_fixture(
                CodecFixtureCase::AdjacentUnit(AdjacentUnitEffect::Marks, form, origin),
            );
        }
    }
}
