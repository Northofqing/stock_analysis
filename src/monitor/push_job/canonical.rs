//! Domain-private canonical-v1 encoding. This is deliberately not a generic public JSON API.

use std::collections::BTreeMap;

use sha2::{Digest, Sha256};

use super::Sha256Digest;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum CanonicalValue {
    Null,
    String(String),
    Unsigned(u64),
    Array(Vec<CanonicalValue>),
    Object(BTreeMap<&'static str, CanonicalValue>),
}

pub(super) fn canonical_preimage(
    domain: &'static str,
    fields: &BTreeMap<&'static str, CanonicalValue>,
) -> Vec<u8> {
    debug_assert!(domain.is_ascii() && !domain.contains('\0'));
    let mut preimage = Vec::with_capacity(domain.len() + 1 + fields.len() * 32);
    preimage.extend_from_slice(domain.as_bytes());
    preimage.push(0);
    write_json_object(&mut preimage, fields);
    preimage
}

pub(super) fn canonical_digest(
    domain: &'static str,
    fields: &BTreeMap<&'static str, CanonicalValue>,
) -> Sha256Digest {
    raw_digest(&canonical_preimage(domain, fields))
}

pub(super) fn raw_digest(bytes: &[u8]) -> Sha256Digest {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    Sha256Digest::from_bytes(hasher.finalize().into())
}

fn write_json_object(output: &mut Vec<u8>, fields: &BTreeMap<&'static str, CanonicalValue>) {
    output.push(b'{');
    for (index, (key, value)) in fields.iter().enumerate() {
        if index != 0 {
            output.push(b',');
        }
        write_json_string(output, key);
        output.push(b':');
        write_json_value(output, value);
    }
    output.push(b'}');
}

fn write_json_value(output: &mut Vec<u8>, value: &CanonicalValue) {
    match value {
        CanonicalValue::Null => output.extend_from_slice(b"null"),
        CanonicalValue::String(value) => write_json_string(output, value),
        CanonicalValue::Unsigned(value) => output.extend_from_slice(value.to_string().as_bytes()),
        CanonicalValue::Array(values) => {
            output.push(b'[');
            for (index, value) in values.iter().enumerate() {
                if index != 0 {
                    output.push(b',');
                }
                write_json_value(output, value);
            }
            output.push(b']');
        }
        CanonicalValue::Object(fields) => write_json_object(output, fields),
    }
}

fn write_json_string(output: &mut Vec<u8>, value: &str) {
    const HEX: &[u8; 16] = b"0123456789abcdef";

    output.push(b'"');
    for character in value.chars() {
        match character {
            '"' => output.extend_from_slice(br#"\""#),
            '\\' => output.extend_from_slice(br"\\"),
            '\u{0008}' => output.extend_from_slice(br"\b"),
            '\t' => output.extend_from_slice(br"\t"),
            '\n' => output.extend_from_slice(br"\n"),
            '\u{000c}' => output.extend_from_slice(br"\f"),
            '\r' => output.extend_from_slice(br"\r"),
            control if control <= '\u{001f}' => {
                let byte = control as u8;
                output.extend_from_slice(b"\\u00");
                output.push(HEX[usize::from(byte >> 4)]);
                output.push(HEX[usize::from(byte & 0x0f)]);
            }
            other => {
                let mut encoded = [0; 4];
                output.extend_from_slice(other.encode_utf8(&mut encoded).as_bytes());
            }
        }
    }
    output.push(b'"');
}
