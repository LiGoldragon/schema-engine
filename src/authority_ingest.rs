//! The authority-bound ingestion path: parsed schema declarations → central-authority
//! identity binding → canonical `CoreUniverse`.
//!
//! Both front ends — the LEGACY `schema-language` migration and the NATIVE `core-schema`
//! six-slot document decode — land in the same [`ParsedSchema`]: stringless Core
//! declarations plus the name space they were parsed against. From there the path is
//! identical for both, which is exactly what makes the equivalence witness meaningful:
//! compute one [`DeclaredIdentity`] per declaration, bind-or-mint them against the one
//! central authority (`sema-storage`), and build the universe from the authority's
//! assignment via [`CoreUniverse::from_assignment`]. Because that build re-stamps every
//! interior name into a canonical order, two ingestions of one declared schema-whole that
//! received the same assignment produce byte-identical Core content — the keystone.
//!
//! The self-contained migration in [`crate::Dispatch`] (the `IngestTypeSchema` request,
//! which stores the migrated schema directly with its own parse-order name table) is the
//! OFFLINE / LEAN mode: it never consults the authority, so its identifiers are
//! parse-order interned rather than authority-assigned. This authority-bound path is the
//! online mode that realizes the keystone.
//!
//! LEAN `declared-key-and-shape-from-text` (load-bearing): a parsed declaration's bind-
//! or-mint request keys on its RESOLVED declared name (`DeclaredKey` = the name spelling
//! bytes) and fingerprints its RESOLVED structure (`DeclaredShape` = blake3 over role,
//! visibility, kind, and each field/variant's resolved name and resolved reference shape).
//! Resolving through the front end's name table — never folding raw `Identifier` integers
//! — is what makes both the key and the shape a function of the DECLARED content, so the
//! two front ends agree on them for one source. Revision trigger: an authoring surface
//! carrying an explicit per-declaration bind/mint marker distinct from the name.
//! (The elided-string-field spelling ruling of bead .31 has since landed — the derived
//! name canonicalises to `string` on both front ends — and this LEAN held unchanged,
//! because it already resolves field names through each front end's name table rather
//! than folding raw identifiers.)

use content_identity::IdentityHasher;
use core_schema::declaration::{CoreDeclaration, CoreField, CoreVariant};
use core_schema::{
    AssignedKind, AssignedMember, CoreReference, CoreSchema, CoreType, CoreUniverse,
    CoreUniverseId, DeclarationRole, TextualError, TextualSchema, UniverseError, Visibility,
};
use name_table::{NameTable, NameTableError};
use signal_sema_storage::{
    BoundIdentities, DeclaredIdentity, DeclaredKey, DeclaredShape, IdentityIntent,
};

use crate::legacy_ingest::{LegacyIngestError, LegacySchemaIngest};

/// A failure of the authority-bound ingestion path.
#[derive(Debug, thiserror::Error)]
pub enum AuthorityIngestError {
    #[error("legacy front end: {0}")]
    Legacy(#[from] LegacyIngestError),
    #[error("native front end: {0}")]
    Native(#[from] TextualError),
    #[error("name resolution: {0}")]
    Names(#[from] NameTableError),
    #[error("universe build: {0}")]
    Universe(#[from] UniverseError),
    #[error("the authority bound no identity for declared key {0:?}")]
    UnboundDeclaration(Vec<u8>),
}

/// One schema unit parsed from a single front end: its stringless Core declarations and
/// the name space they were parsed against. The two constructors are the two front ends;
/// every method below is front-end agnostic.
pub struct ParsedSchema {
    schema: CoreSchema,
    names: NameTable,
}

impl ParsedSchema {
    /// Parse through the LEGACY front end: `schema-language` lowering, migrated into the
    /// stringless Core substrate.
    pub fn from_legacy(text: &str) -> Result<Self, AuthorityIngestError> {
        let migration = LegacySchemaIngest::migrate_text(text)?;
        Ok(Self {
            schema: migration.schema,
            names: migration.names,
        })
    }

    /// Parse through the NATIVE front end: `core-schema`'s six-slot document decode.
    pub fn from_native(text: &str) -> Result<Self, AuthorityIngestError> {
        let mut names = NameTable::new();
        let schema = TextualSchema::schema_document()?.decode_document(text, &mut names)?;
        Ok(Self { schema, names })
    }

    pub fn schema(&self) -> &CoreSchema {
        &self.schema
    }

    pub fn names(&self) -> &NameTable {
        &self.names
    }

    /// The bind-or-mint declaration set for this parsed schema: one [`DeclaredIdentity`]
    /// per declaration, keyed by resolved declared name, fingerprinted by resolved
    /// structural shape, all [`IdentityIntent::MintOrBind`]. Presented in parse order;
    /// the authority binds by key, so a re-presentation under the same whole returns the
    /// same identities regardless of the order either front end parsed.
    pub fn declared_identities(&self) -> Result<Vec<DeclaredIdentity>, AuthorityIngestError> {
        self.schema
            .declarations()
            .iter()
            .map(|declaration| {
                Ok(DeclaredIdentity {
                    key: self.declared_key(declaration)?,
                    shape: self.declared_shape(declaration)?,
                    intent: IdentityIntent::MintOrBind,
                })
            })
            .collect()
    }

    /// Build the canonical [`CoreUniverse`] from the authority's reply: one assigned
    /// member per declaration at the authority's local identity, then
    /// [`CoreUniverse::from_assignment`] re-stamps every interior name into a canonical
    /// order. The built universe's content identity is a pure function of (assignment,
    /// declaration content) — so two front ends bound to one authority build identical
    /// Core content.
    pub fn build_universe(
        &self,
        bound: &BoundIdentities,
    ) -> Result<CoreUniverse, AuthorityIngestError> {
        let members = self
            .schema
            .declarations()
            .iter()
            .map(|declaration| {
                let key = self.declared_key(declaration)?;
                let local = bound
                    .assignments
                    .iter()
                    .find(|assignment| assignment.key == key)
                    .ok_or_else(|| AuthorityIngestError::UnboundDeclaration(key.0.clone()))?
                    .identity
                    .0;
                let name = self.names.resolve(declaration.identifier())?.clone();
                Ok(AssignedMember::new(
                    local,
                    name,
                    AssignedKind::Declaration(declaration.clone()),
                ))
            })
            .collect::<Result<Vec<_>, AuthorityIngestError>>()?;
        Ok(CoreUniverse::from_assignment(
            CoreUniverseId::new(bound.universe.0),
            members,
            &self.names,
        )?)
    }

    /// The declared key of one declaration — its resolved name spelling (LEAN
    /// `declared-key-is-name`).
    fn declared_key(
        &self,
        declaration: &CoreDeclaration,
    ) -> Result<DeclaredKey, AuthorityIngestError> {
        let name = self.names.resolve(declaration.identifier())?;
        Ok(DeclaredKey(name.as_str().as_bytes().to_vec()))
    }

    /// The structural fingerprint of one declaration, resolved through the name space so
    /// it is a function of the declared structure rather than of raw interned ids.
    fn declared_shape(
        &self,
        declaration: &CoreDeclaration,
    ) -> Result<DeclaredShape, AuthorityIngestError> {
        let role_tag = match declaration.role() {
            DeclarationRole::DataType => 0u8,
            DeclarationRole::InterfaceInput => 1,
            DeclarationRole::InterfaceOutput => 2,
        };
        let visibility_tag = match declaration.visibility() {
            Visibility::Public => 0u8,
            Visibility::Private => 1,
        };
        let mut hasher = IdentityHasher::unprimed();
        hasher.update_raw(&[role_tag, visibility_tag]);
        match declaration.value() {
            CoreType::Newtype(newtype) => {
                hasher.update_raw(&[0]);
                self.fold_reference(&mut hasher, newtype.reference())?;
            }
            CoreType::Struct(structure) => {
                hasher.update_raw(&[1]);
                hasher.update_raw(&(structure.fields().len() as u64).to_le_bytes());
                for field in structure.fields() {
                    self.fold_field(&mut hasher, field)?;
                }
            }
            CoreType::Enumeration(enumeration) => {
                hasher.update_raw(&[2]);
                hasher.update_raw(&(enumeration.variants().len() as u64).to_le_bytes());
                for variant in enumeration.variants() {
                    self.fold_variant(&mut hasher, variant)?;
                }
            }
        }
        Ok(DeclaredShape(hasher.finalize_bytes()))
    }

    fn fold_field(
        &self,
        hasher: &mut IdentityHasher,
        field: &CoreField,
    ) -> Result<(), NameTableError> {
        hasher.update_length_prefixed(self.names.resolve(field.identifier())?.as_str().as_bytes());
        self.fold_reference(hasher, field.reference())
    }

    fn fold_variant(
        &self,
        hasher: &mut IdentityHasher,
        variant: &CoreVariant,
    ) -> Result<(), NameTableError> {
        hasher.update_length_prefixed(
            self.names
                .resolve(variant.identifier())?
                .as_str()
                .as_bytes(),
        );
        match variant.payload() {
            Some(reference) => {
                hasher.update_raw(&[1]);
                self.fold_reference(hasher, reference)
            }
            None => {
                hasher.update_raw(&[0]);
                Ok(())
            }
        }
    }

    fn fold_reference(
        &self,
        hasher: &mut IdentityHasher,
        reference: &CoreReference,
    ) -> Result<(), NameTableError> {
        match reference {
            CoreReference::String => {
                hasher.update_raw(&[0]);
            }
            CoreReference::Integer => {
                hasher.update_raw(&[1]);
            }
            CoreReference::Boolean => {
                hasher.update_raw(&[2]);
            }
            CoreReference::Bytes => {
                hasher.update_raw(&[3]);
            }
            CoreReference::Plain(identifier) => {
                hasher.update_raw(&[4]);
                hasher.update_length_prefixed(self.names.resolve(*identifier)?.as_str().as_bytes());
            }
            CoreReference::SingleTypeApplication {
                projection,
                argument,
            } => {
                hasher.update_raw(&[5, *projection as u8]);
                self.fold_reference(hasher, argument)?;
            }
            CoreReference::MultiTypeApplication {
                projection,
                arguments,
            } => {
                hasher.update_raw(&[6, *projection as u8]);
                hasher.update_raw(&(arguments.len() as u64).to_le_bytes());
                for argument in arguments {
                    self.fold_reference(hasher, argument)?;
                }
            }
            CoreReference::ValueApplication { projection, value } => {
                hasher.update_raw(&[7, *projection as u8]);
                hasher.update_raw(&value.to_le_bytes());
            }
        }
        Ok(())
    }
}
