//! Source-bound unresolved original-price disputes; this is not a price floor.
//!
//! Retained 2026-09-24 before.db SHA256:
//! a6f36c96047d0ca885d8e1af2764bdf76327f23533fe379bff893b1a5aba144e
//! The same original facts are independently present in BR249 audit 29,
//! source snapshot beebdb8316841a43d38667757113f6755607eddb5fec05187bd7ed0e4b49d215,
//! and the BR255 frozen legacy Filled manifest
//! 53a6051bc7aba1cb1ffdf677afa5ae9a9a1a1c137dabb1b96af46c7ee4be17af.
//! Original candidate-only review SHA256:
//! 83e7216d6692aa35449718e46c69ad97903e0a342a708ae734e76c1089650662.
//! It establishes an unresolved dispute, not authority to delete or reprice.
//!
//! These are existing FillLineage.raw_hash values, obtained with the ordinary
//! release reader from both the retained backup and exact restored rows. Keep
//! that reader's SQLite JSON encoding; Python's SQLite float rendering differs.

use super::{digest, encode, LedgerError};
use diesel::{
    prelude::*,
    sql_types::{BigInt, Double, Nullable, Text},
};

// (existing raw JSON hash, portable original-field/IEEE754 semantic hash).
const ORIGINAL_FACTS: [(&str, &str); 19] = [
    (
        "ff5792673fbacf902a2c378c618489a5b4edce90f6b611d6f688c47100f1e6a9",
        "84569995b85c4d8be3a1884641120c8a4dce9fb241f8cb040c6375fa2c8b2b5c",
    ),
    (
        "f522bbb31ebf944e611f8e022c99e7bf08ef226503b0fe88e3717070728c50c1",
        "11124d997a98a6525ff18c0150d2c3997fc76c4b48765ff6ae8c2335953e95a6",
    ),
    (
        "cedddb099dfd9fdc46361614e497dfdd7b0a29da281ed8158ff80611bf3ba9a7",
        "dac10ac516d0dc37d07b19411e6213294ecbd6a8437d5ec8d96b55e5ae49f2db",
    ),
    (
        "d527b690051591150ee3100ea36bbc6ce547207fe42e5b729aa78291c97cba76",
        "53dbc8c0fe67d3af5f240b43da5bb00710aff43dec1a6aca377645ee48c6b0bb",
    ),
    (
        "e9dac66ba88479cd7427a4f7647f99fdb524438d0863c0e3798ebecd229fdc35",
        "3230ee65314c6b57e80683684a5699a580505c72c5d5ac9d903f9cfab317fbd1",
    ),
    (
        "6ad40cab44246c6c597eed4fabea7f161d804931575eee455f953858ca7a3818",
        "ccdec71adf889b2389363f04de6e08b2f97659974d08945d8b993d1704f29af0",
    ),
    (
        "7a4a7fe41302ab88cbccb8c78e48495ac0ba68ceedc0904288f9dc4defa3b547",
        "37df0b3416f988042c837998837535fa4e3e54312e7b9e4b05a32b00879a0650",
    ),
    (
        "ef05f7bcae49e88fa2ee9b9c854da033af67b2d4768e8ab159cca56925a8cda4",
        "f5232c589a5460cf14992ac9eeaa3c79421447e62c93023f7336504655977078",
    ),
    (
        "e5a32d6df37de731b9d729556195e26f778a56755fd2a16f3671b62d2564d27c",
        "2ec5467e4f19c07d071450cc65c3020d79034fb6c80cfddbe4988a3052dfa0c8",
    ),
    (
        "33758517cedec48754afb6af1d14ceb7509634b89438f2776f535406faa4b15d",
        "91be143f19688ec06896712c8203e1fa3f9b4cd49932c21df3d8dc3011f66518",
    ),
    (
        "c31f66353e0ef2f3654cd8ebd93b4015d19d30b7b749860656c00af57f95a155",
        "1f5b166c037a60584d178750c50b12db2cccab76c80e97f3f8509a6f2194f478",
    ),
    (
        "fbf54bd23f7d35acc1048dd0deb48afb04c711086145751d84385d389ae457fc",
        "60b6f7436e7d6cf93c0fc365b00d05cf77c85da9d4851899d9a8543081ded54c",
    ),
    (
        "0751293620b28262104e5ecb2d40f39ecdb903a8a9d1c4b76a7061eef15f7072",
        "d4d4f4135252543bbc2672f1d090b062584b29b8c19d7d54ae3fd2457bef73ba",
    ),
    (
        "57ddff3cced9004e99cf91138dd52349d1eb804a20fbd09c5a6565f64aff8753",
        "4cd92c54869b4ad22a52bbd515ad8e018df84a3e2eff900d708faf8d6621a2d2",
    ),
    (
        "e0b6775a433712b1a5dbd197ab647770dda93b337eac827092b01769adfcebf7",
        "cedfb39966bdc6a4cd17ad3548ebfb289781269f06b2bbb003820c14446e468a",
    ),
    (
        "1b42b9a7ead73aa199051028826d888aad94441d39265155f21cacd83493a146",
        "1c152d68a3d314d67d24e8c06d79e0dff0342abd1bc8e012d69c1a388b5b304f",
    ),
    (
        "7b5bcd98de54314c13f021da8f759f00f18a058b59eca64971431c5cd66fa418",
        "45d1b5736d99f008ea070415acf377452f8d5e3c7ff6c7f806f579275ad2c8ef",
    ),
    (
        "db77239db7f027bb4d07b19732bff4bd26fa06a710346fecaa406130d561bab4",
        "d84faedb8daf06908677dbe52d45ce6c096d699f3fe13a566b59dd710893fd21",
    ),
    (
        "64d7f078bb2215c5d11b663e67e930ebb61a605bc4b9a3f27c2201f4426172d0",
        "258b39d722e343b05690a1033282ef8576c481f65b14562773efcbeba5fb6d02",
    ),
];

fn original_facts() -> impl Iterator<Item = (&'static str, &'static str)> {
    ORIGINAL_FACTS.into_iter().chain(test_fact())
}

#[cfg(test)]
fn test_fact() -> Option<(&'static str, &'static str)> {
    Some((
        "11d992553a4f5baf1203199efe770ae46f13b3078aa3a3a9a6a17eb8f2c76d27",
        "70cfe380acf988cd727f2381d955641ab93f7b596325165083f590a05b59fefc",
    ))
}
#[cfg(not(test))]
fn test_fact() -> Option<(&'static str, &'static str)> {
    None
}

pub(super) fn is_disputed_original(raw_hash: &str) -> bool {
    original_facts().any(|fact| fact.0 == raw_hash)
}

// The same complete 15 original fields as raw_bytes, with floats obtained
// directly from SQLite, not parsed through a JSON float codec.
#[derive(QueryableByName)]
struct OriginalFact {
    #[diesel(sql_type = BigInt)]
    id: i64,
    #[diesel(sql_type = Text)]
    plan_id: String,
    #[diesel(sql_type = Text)]
    code: String,
    #[diesel(sql_type = Text)]
    name: String,
    #[diesel(sql_type = Text)]
    direction: String,
    #[diesel(sql_type = Double)]
    price: f64,
    #[diesel(sql_type = BigInt)]
    quantity: i64,
    #[diesel(sql_type = Text)]
    status: String,
    #[diesel(sql_type = Nullable<Double>)]
    fill_price: Option<f64>,
    #[diesel(sql_type = Nullable<Text>)]
    not_fill_reason: Option<String>,
    #[diesel(sql_type = Text)]
    virtual_reason: String,
    #[diesel(sql_type = Text)]
    account_mode: String,
    #[diesel(sql_type = Text)]
    data_mode: String,
    #[diesel(sql_type = Text)]
    ts: String,
    #[diesel(sql_type = Text)]
    updated_at: String,
}

pub(super) fn verify_original_codec(
    conn: &mut SqliteConnection,
    id: i64,
    raw_hash: &str,
) -> Result<(), LedgerError> {
    let fact = diesel::sql_query("SELECT id,plan_id,code,name,direction,price,quantity,status,fill_price,not_fill_reason,virtual_reason,account_mode,data_mode,CAST(ts AS TEXT) AS ts,CAST(updated_at AS TEXT) AS updated_at FROM paper_trades WHERE id=?")
        .bind::<BigInt, _>(id).get_result::<OriginalFact>(conn)?;
    let semantic_hash = digest(&encode(&(
        "LegacyOriginalPriceDisputeFactsV1",
        fact.id,
        &fact.plan_id,
        &fact.code,
        &fact.name,
        &fact.direction,
        fact.price.to_bits(),
        fact.quantity,
        &fact.status,
        fact.fill_price.map(f64::to_bits),
        &fact.not_fill_reason,
        &fact.virtual_reason,
        &fact.account_mode,
        &fact.data_mode,
        &fact.ts,
        &fact.updated_at,
    ))?);
    verify_hash_pair(raw_hash, &semantic_hash)
}

fn verify_hash_pair(raw_hash: &str, semantic_hash: &str) -> Result<(), LedgerError> {
    for (original_raw, original_semantic) in original_facts() {
        if semantic_hash == original_semantic && raw_hash != original_raw {
            return Err(LedgerError::EvidenceUnavailable(
                "legacy price dispute raw codec changed; reverify original backup and all original IEEE754 fields before economic use".into(),
            ));
        }
        if raw_hash == original_raw && semantic_hash != original_semantic {
            return Err(LedgerError::IntegrityFailure(
                "legacy price dispute raw and original-field fingerprints disagree".into(),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_price_dispute_codec_change_and_fingerprint_mismatch_refuse_authority() {
        let (raw, semantic) = ORIGINAL_FACTS[0];
        assert!(verify_hash_pair(raw, semantic).is_ok());
        assert!(matches!(
            verify_hash_pair("different-sqlite-json-codec", semantic),
            Err(LedgerError::EvidenceUnavailable(_))
        ));
        assert!(matches!(
            verify_hash_pair(raw, "altered-original-field"),
            Err(LedgerError::IntegrityFailure(_))
        ));
    }
}
