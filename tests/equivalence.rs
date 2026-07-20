//! The keystone equivalence witness: one declared schema unit, ingested through the
//! LEGACY front end (`schema-language` migration) and through the NATIVE front end
//! (`core-schema` six-slot document decode), BOTH bound through one central identity
//! authority (a live `sema-storage` runtime), yields IDENTICAL `EncodedSchema` content
//! identity.
//!
//! This is the equivalence the whole authority-provided universe path exists to
//! guarantee: the two front ends parse the same source into different private name
//! tables (their interior identifiers disagree), but binding through the one authority
//! and rebuilding via `EncodedUniverse::from_assignment` — which re-stamps every interior
//! name into a canonical order — collapses that difference. Two ingestions of one
//! declared schema-whole that received the same assignment build byte-identical encoded-form
//! content.
//!
//! This is the in-process engine-level floor: the authority is a real `sema-storage`
//! `Runtime` (real bind-or-mint, real durable engine, real two-law enforcement) driven
//! in-process rather than over a socket. The socket-live four-process pipeline is
//! witnessed separately in `language-engine-witness`.
//!
//! RESOLVED (beads .31, .36; psyche rulings 2026-07-17): the witness now runs on
//! `spirit-min` ITSELF ([`spirit_min_ingests_to_identical_core_identity`]), the real
//! fixture through both front ends, hash-equal. The two former divergences are closed
//! by the native front end converging onto the rulings: (1) the string scalar's
//! canonical spelling is `String` ("Strings are Strings"), so `Topic.String` /
//! `Description.String` recognise the same string leaf on both sides; (2) a single-field
//! braced type `Summary.{ Description }` lowers to a NEWTYPE on both sides (newtype
//! ruling). The purpose-authored fixture test
//! ([`legacy_and_native_ingest_to_identical_core_identity`]) is KEPT: it guards the
//! canonicalisation independently of the specific `spirit-min` shape.

use core_schema::{EncodedReference, EncodedType};
use schema_engine::ParsedSchema;
use sema_storage::Runtime;
use signal_sema_storage::{BoundIdentities, Reply, Request, SchemaWholeHandle};

/// The shared source both front ends parse. A purpose-authored minimal unit that
/// exercises Plain cross-references, multi-field structs with elided (derived) field
/// names, payload and payload-free enum variants, and both interface roots — the
/// constructs the canonicalisation must neutralise. It deliberately avoids the two
/// constructs the two front ends still represent differently (see the module note on
/// bead .31): a direct `String`/`Text` scalar reference, and a single-field braced type.
const EQUIVALENCE_MIN: &str = include_str!("fixtures/equivalence-min.schema");

/// Bind one parsed schema's declared identities against the authority under `whole`,
/// returning the authority's universe-and-assignments reply.
async fn bind(
    runtime: &Runtime,
    whole: SchemaWholeHandle,
    parsed: &ParsedSchema,
) -> BoundIdentities {
    let declarations = parsed
        .declared_identities()
        .expect("declared identities from parsed schema");
    let reply = runtime
        .request(Request::BindIdentities {
            whole,
            declarations,
        })
        .await
        .expect("bind request");
    let Reply::IdentitiesBound(bound) = reply else {
        panic!("expected IdentitiesBound, got {reply:?}");
    };
    bound
}

/// The real `spirit-min` schema, verbatim — the same fixture the socket-live
/// `language-engine-witness` and the frozen `golden-bridge` carry. It exercises BOTH
/// constructs the two front ends once represented differently: a direct string scalar
/// (`Topic.String`, `Description.String`) and a single-field braced type
/// (`Summary.{ Description }`). With the 2026-07-17 rulings both front ends now agree.
const SPIRIT_MIN: &str = include_str!("fixtures/spirit-min.schema");

/// Ingest one six-slot legacy source and its seven-slot native counterpart, bind each against the one authority under
/// the same schema-whole handle, and assert the two built universes carry identical
/// `EncodedSchema` content identity. This is the whole point of the authority-provided
/// universe path: two front ends that intern into disagreeing private name tables,
/// re-stamped through one assignment, collapse to byte-identical encoded-form content.
async fn assert_front_ends_agree(source: &str, whole_key: &[u8]) {
    let temporary = tempfile::tempdir().expect("temp dir");
    let runtime = Runtime::open(&temporary.path().join("authority.sema"))
        .await
        .expect("open the central authority");

    // One schema-whole handle: both front ends present it, so the authority binds the
    // SAME universe and identities to each.
    let whole = SchemaWholeHandle(whole_key.to_vec());

    let legacy = ParsedSchema::from_legacy(source).expect("legacy front end");
    let native_source = format!("{source}\n[]");
    let native = ParsedSchema::from_native(&native_source).expect("native front end");

    let legacy_bound = bind(&runtime, whole.clone(), &legacy).await;
    let native_bound = bind(&runtime, whole.clone(), &native).await;

    let legacy_universe = legacy
        .build_universe(&legacy_bound)
        .expect("build the legacy-ingested universe");
    let native_universe = native
        .build_universe(&native_bound)
        .expect("build the native-ingested universe");

    let legacy_identity = legacy_universe
        .declared_schema()
        .content_identity()
        .expect("legacy content identity");
    let native_identity = native_universe
        .declared_schema()
        .content_identity()
        .expect("native content identity");

    assert_eq!(
        legacy_identity, native_identity,
        "one authority, two front ends: the same declared schema unit yields identical \
         EncodedSchema content identity",
    );

    runtime.shutdown().await.expect("shutdown authority");
}

/// The purpose-authored fixture: kept as an independent guard on the canonicalisation,
/// deliberately avoiding the two once-divergent constructs (see the module note).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn legacy_and_native_ingest_to_identical_core_identity() {
    assert_front_ends_agree(EQUIVALENCE_MIN, b"equivalence:min").await;
}

/// The keystone, on the REAL fixture: `spirit-min` ITSELF — with its direct `String`
/// scalars and its single-field braced `Summary.{ Description }` — ingests through both
/// front ends to identical `EncodedSchema` content identity, now that the native front end
/// has converged onto the 2026-07-17 rulings.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn spirit_min_ingests_to_identical_core_identity() {
    assert_front_ends_agree(SPIRIT_MIN, b"equivalence:spirit-min").await;
}

/// Directly witness the shape ruling (beads .36, psyche 2026-07-17) that makes the
/// equivalence above hold: the single-field braced declaration `Summary.{ Description }`
/// lowers to a NEWTYPE over `Description` — not a one-field struct. The hash-equal tests
/// above would still pass if BOTH front ends agreed on a struct; this pins the actual
/// shape the ruling requires, on the native decode path and on the legacy one.
fn assert_summary_is_a_newtype_over_description(parsed: &ParsedSchema, front_end: &str) {
    let names = parsed.names();
    let summary = parsed
        .schema()
        .declarations()
        .iter()
        .find(|declaration| {
            names
                .resolve(declaration.identifier())
                .is_ok_and(|name| name.as_str() == "Summary")
        })
        .unwrap_or_else(|| panic!("{front_end}: spirit-min declares Summary"));

    let EncodedType::Newtype(newtype) = summary.value() else {
        panic!(
            "{front_end}: single-field braced Summary.{{ Description }} must lower to a newtype \
             (beads .36 ruling: a single-field brace is a newtype), found {:?}",
            summary.value()
        );
    };

    let EncodedReference::Plain(target) = newtype.reference() else {
        panic!(
            "{front_end}: Summary's newtype target must be the declared type Description, found {:?}",
            newtype.reference()
        );
    };
    assert_eq!(
        names
            .resolve(*target)
            .expect("resolve Summary's newtype target")
            .as_str(),
        "Description",
        "{front_end}: Summary is a newtype over Description",
    );
}

/// The missing round-trip witness (review item on beads .36): the NATIVE `core-schema`
/// six-slot document decode lowers `Summary.{ Description }` from the real `spirit-min`
/// fixture to a newtype over `Description`. The legacy front end is asserted alongside
/// so the both-sides ruling is pinned directly, not only inferred from hash equality.
#[test]
fn native_decode_lowers_single_field_brace_summary_to_a_newtype() {
    let native =
        ParsedSchema::from_native(SPIRIT_MIN).expect("native front end decodes spirit-min");
    assert_summary_is_a_newtype_over_description(&native, "native");

    let legacy =
        ParsedSchema::from_legacy(SPIRIT_MIN).expect("legacy front end decodes spirit-min");
    assert_summary_is_a_newtype_over_description(&legacy, "legacy");
}
