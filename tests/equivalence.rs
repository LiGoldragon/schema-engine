//! The keystone equivalence witness: one declared schema unit, ingested through the
//! LEGACY front end (`schema-language` migration) and through the NATIVE front end
//! (`core-schema` six-slot document decode), BOTH bound through one central identity
//! authority (a live `sema-storage` runtime), yields IDENTICAL `CoreSchema` content
//! identity.
//!
//! This is the equivalence the whole authority-provided universe path exists to
//! guarantee: the two front ends parse the same source into different private name
//! tables (their interior identifiers disagree), but binding through the one authority
//! and rebuilding via `CoreUniverse::from_assignment` — which re-stamps every interior
//! name into a canonical order — collapses that difference. Two ingestions of one
//! declared schema-whole that received the same assignment build byte-identical Core
//! content.
//!
//! This is the in-process engine-level floor: the authority is a real `sema-storage`
//! `Runtime` (real bind-or-mint, real durable engine, real two-law enforcement) driven
//! in-process rather than over a socket. The socket-live four-process pipeline is
//! witnessed separately in `language-engine-witness`.
//!
//! WATCH (bead .31, psyche-pending): the witness runs on a purpose-authored fixture
//! rather than `spirit-min` because `spirit-min` triggers two front-end representation
//! divergences that make exact parity IMPOSSIBLE until the psyche rules — proven by
//! ingesting `spirit-min` through both front ends. First, `.31` proper — a direct
//! string scalar (`Topic.String`, `Description.String`): the legacy front end
//! (`schema-language`) recognises `String` as the string scalar leaf, while the native
//! decode (`core-schema`) recognises only `Text`, so it reads `String` as a `Plain`
//! reference to a user type. Native `Text` vs legacy `String`; not reconciled here (the
//! spelling is the psyche's to rule). Second, a single-field braced type
//! (`Summary.{ Description }`): the legacy front end lowers it to a newtype, the native
//! decode keeps a single-field struct. Every other `spirit-min` declaration —
//! multi-field structs, Plain cross-references, enums, and both interface roots —
//! already agrees across the two front ends.

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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn legacy_and_native_ingest_to_identical_core_identity() {
    let temporary = tempfile::tempdir().expect("temp dir");
    let runtime = Runtime::open(&temporary.path().join("authority.sema"))
        .await
        .expect("open the central authority");

    // One schema-whole handle: both front ends present it, so the authority binds the
    // SAME universe and identities to each.
    let whole = SchemaWholeHandle(b"equivalence:min".to_vec());

    let legacy = ParsedSchema::from_legacy(EQUIVALENCE_MIN).expect("legacy front end");
    let native = ParsedSchema::from_native(EQUIVALENCE_MIN).expect("native front end");

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
         CoreSchema content identity",
    );

    runtime.shutdown().await.expect("shutdown authority");
}
