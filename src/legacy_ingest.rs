use core_schema::{
    DeclarationRole, EncodedDeclaration, EncodedEnum, EncodedField, EncodedNewtype,
    EncodedReference, EncodedSchema, EncodedStruct, EncodedType, EncodedVariant,
    MultiTypeReferenceProjection, SingleTypeReferenceProjection, ValueReferenceProjection,
    Visibility,
};
use name_table::{Identifier, Name, NameTable};
use schema_language::{
    Declaration, EnumDeclaration, MultiTypeReferenceProjection as LegacyMultiProjection, Root,
    SchemaEngine, SchemaIdentity, SingleTypeReferenceProjection as LegacySingleProjection,
    TypeDeclaration, TypeReference, ValueReferenceProjection as LegacyValueProjection,
};

#[derive(Debug, thiserror::Error)]
pub enum LegacyIngestError {
    #[error("legacy schema lowering: {0}")]
    Lower(#[from] schema_language::SchemaError),
    #[error("unsupported legacy Path scalar")]
    UnsupportedPath,
    #[error("unsupported user-defined generic application")]
    UnsupportedApplication,
    #[error("unsupported application-form interface root")]
    UnsupportedInterfaceApplication,
}

pub struct LegacyMigration {
    pub schema: EncodedSchema,
    pub names: NameTable,
}

pub struct LegacySchemaIngest {
    names: NameTable,
}

impl LegacySchemaIngest {
    pub fn migrate_text(text: &str) -> Result<LegacyMigration, LegacyIngestError> {
        let source =
            SchemaEngine::default().lower_source(text, SchemaIdentity::new("legacy-edge", "0"))?;
        let mut ingest = Self {
            names: NameTable::new(),
        };
        let mut declarations = source
            .namespace()
            .iter()
            .map(|declaration| ingest.migrate_declaration(declaration))
            .collect::<Result<Vec<_>, _>>()?;
        // The two protocol lines migrate into role-tagged enumeration declarations —
        // the same one representation the native document decode fills — so
        // interface-root-ness survives ingestion and downstream Nomos lowering can
        // gate on it. They stay ordinary declarations (name and variants unchanged),
        // so their projected bytes are identical to before; only the role is new.
        let [input, output] = source.input_and_output();
        declarations.push(ingest.migrate_interface(&input, DeclarationRole::InterfaceInput)?);
        declarations.push(ingest.migrate_interface(&output, DeclarationRole::InterfaceOutput)?);
        Ok(LegacyMigration {
            schema: EncodedSchema::new(declarations),
            names: ingest.names,
        })
    }

    /// Migrate one protocol-line root (`input` / `output`) into its role-tagged
    /// interface-root declaration: the root's enum lowers exactly as any enumeration,
    /// and the resulting declaration carries the interface `role`. An application-form
    /// root has no variant list to carry and is rejected loudly.
    fn migrate_interface(
        &mut self,
        root: &Root,
        role: DeclarationRole,
    ) -> Result<EncodedDeclaration, LegacyIngestError> {
        let enumeration = root
            .as_enum()
            .ok_or(LegacyIngestError::UnsupportedInterfaceApplication)?;
        Ok(EncodedDeclaration::interface(
            role,
            self.migrate_enumeration(enumeration)?,
        ))
    }

    fn migrate_declaration(
        &mut self,
        declaration: &Declaration,
    ) -> Result<EncodedDeclaration, LegacyIngestError> {
        let value = match declaration.value() {
            TypeDeclaration::Newtype(newtype) => EncodedType::Newtype(EncodedNewtype::new(
                self.intern(newtype.name.as_str()),
                self.migrate_reference(&newtype.reference)?,
            )),
            TypeDeclaration::Struct(structure) => {
                let fields = structure
                    .fields
                    .iter()
                    .map(|field| {
                        Ok(EncodedField::new(
                            self.intern(field.name.as_str()),
                            self.migrate_reference(&field.reference)?,
                        ))
                    })
                    .collect::<Result<Vec<_>, LegacyIngestError>>()?;
                EncodedType::Struct(EncodedStruct::new(
                    self.intern(structure.name.as_str()),
                    fields,
                ))
            }
            TypeDeclaration::Enum(enumeration) => self.migrate_enumeration(enumeration)?,
        };
        let visibility = if declaration.is_private() {
            Visibility::Private
        } else {
            Visibility::Public
        };
        Ok(EncodedDeclaration::new(visibility, value))
    }

    fn migrate_enumeration(
        &mut self,
        enumeration: &EnumDeclaration,
    ) -> Result<EncodedType, LegacyIngestError> {
        let variants = enumeration
            .variants
            .iter()
            .map(|variant| {
                Ok(EncodedVariant::new(
                    self.intern(variant.name.as_str()),
                    variant
                        .payload
                        .as_ref()
                        .map(|payload| self.migrate_reference(payload))
                        .transpose()?,
                ))
            })
            .collect::<Result<Vec<_>, LegacyIngestError>>()?;
        Ok(EncodedType::Enumeration(EncodedEnum::new(
            self.intern(enumeration.name.as_str()),
            variants,
        )))
    }

    fn migrate_reference(
        &mut self,
        reference: &TypeReference,
    ) -> Result<EncodedReference, LegacyIngestError> {
        Ok(match reference {
            TypeReference::String => EncodedReference::String,
            TypeReference::Integer => EncodedReference::Integer,
            TypeReference::Boolean => EncodedReference::Boolean,
            TypeReference::Bytes => EncodedReference::Bytes,
            TypeReference::Path => return Err(LegacyIngestError::UnsupportedPath),
            TypeReference::Plain(name) => EncodedReference::Plain(self.intern(name.as_str())),
            TypeReference::SingleTypeApplication {
                projection,
                argument,
            } => EncodedReference::SingleTypeApplication {
                projection: match projection {
                    LegacySingleProjection::Vector => SingleTypeReferenceProjection::Vector,
                    LegacySingleProjection::Optional => SingleTypeReferenceProjection::Optional,
                    LegacySingleProjection::ScopeOf => SingleTypeReferenceProjection::ScopeOf,
                },
                argument: Box::new(self.migrate_reference(argument)?),
            },
            TypeReference::MultiTypeApplication {
                projection,
                arguments,
            } => EncodedReference::MultiTypeApplication {
                projection: match projection {
                    LegacyMultiProjection::Map => MultiTypeReferenceProjection::Map,
                },
                arguments: arguments
                    .iter()
                    .map(|argument| self.migrate_reference(argument))
                    .collect::<Result<Vec<_>, _>>()?,
            },
            TypeReference::ValueApplication { projection, value } => {
                EncodedReference::ValueApplication {
                    projection: match projection {
                        LegacyValueProjection::Bytes => ValueReferenceProjection::Bytes,
                    },
                    value: *value,
                }
            }
            TypeReference::Application { .. } => {
                return Err(LegacyIngestError::UnsupportedApplication);
            }
        })
    }

    fn intern(&mut self, spelling: &str) -> Identifier {
        self.names.intern(Name::new(spelling))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core_schema::{EncodedType, TextualSchema};
    use name_table::NameResolver;

    /// A string-free six-slot document — no `Text`/`String` divergence — so the
    /// legacy engine (`schema-language`) and the native document decode
    /// (`core-schema`) parse the very same source, and their interface surfaces can
    /// be compared directly.
    const SHARED_MIN: &str = "\
{}
[Ping.Beat Poke.Nudge]
[Pong.Tick]
{
  Beat.Integer
  Nudge.Integer
  Tick.Integer
}
{}
{}";

    /// The interface surface of a schema's input root, as `(root_name, [(variant,
    /// payload)])`, resolved through the schema's own names — the comparable form
    /// that abstracts over each front end's private identifier binding.
    fn input_surface(
        schema: &EncodedSchema,
        names: &impl NameResolver,
    ) -> (String, Vec<(String, String)>) {
        let root = schema.input().expect("an input interface root");
        assert_eq!(
            root.role(),
            DeclarationRole::InterfaceInput,
            "the input root carries the InterfaceInput role"
        );
        let EncodedType::Enumeration(enumeration) = root.value() else {
            panic!("an interface root is an enumeration");
        };
        let root_name = names
            .resolve(root.identifier())
            .unwrap()
            .as_str()
            .to_owned();
        let variants = enumeration
            .variants()
            .iter()
            .map(|variant| {
                let name = names
                    .resolve(variant.identifier())
                    .unwrap()
                    .as_str()
                    .to_owned();
                let payload = match variant.payload() {
                    Some(EncodedReference::Plain(id)) => {
                        names.resolve(*id).unwrap().as_str().to_owned()
                    }
                    other => panic!("interface payloads are Plain references, got {other:?}"),
                };
                (name, payload)
            })
            .collect();
        (root_name, variants)
    }

    /// Legacy ingestion recognizes the two protocol lines as interface roots and
    /// leaves ordinary data types plain.
    #[test]
    fn legacy_ingestion_tags_interface_roots_and_leaves_data_plain() {
        let migration = LegacySchemaIngest::migrate_text(SHARED_MIN).expect("legacy ingestion");
        let schema = &migration.schema;
        let names = &migration.names;

        let (input_name, input_variants) = input_surface(schema, names);
        assert_eq!(input_name, "Input");
        assert_eq!(
            input_variants,
            vec![
                ("Ping".to_owned(), "Beat".to_owned()),
                ("Poke".to_owned(), "Nudge".to_owned()),
            ],
        );

        let output = schema.output().expect("an output interface root");
        assert_eq!(output.role(), DeclarationRole::InterfaceOutput);
        assert_eq!(
            names.resolve(output.identifier()).unwrap().as_str(),
            "Output"
        );

        // The data declarations stay plain: every non-interface declaration is a
        // DataType, and none is spuriously marked an interface root.
        let data: Vec<&str> = schema
            .data_declarations()
            .map(|declaration| names.resolve(declaration.identifier()).unwrap().as_str())
            .collect();
        assert_eq!(data, vec!["Beat", "Nudge", "Tick"]);
        assert_eq!(
            schema
                .declarations()
                .iter()
                .filter(|declaration| declaration.role() != DeclarationRole::DataType)
                .count(),
            2,
            "exactly the two protocol lines are interface roots",
        );
    }

    /// The native document decode and legacy ingestion fill the SAME interface
    /// representation: for the same source they agree on the interface surface —
    /// root name, variant names, and payload names. Content-hash equality is a
    /// separate queued slice (identifier binding) and is deliberately not asserted.
    #[test]
    fn native_and_legacy_agree_on_the_interface_surface() {
        let legacy = LegacySchemaIngest::migrate_text(SHARED_MIN).expect("legacy ingestion");

        let mut native_names = name_table::NameTable::new();
        let native = TextualSchema::schema_document()
            .expect("build the document grammar")
            .decode_document(SHARED_MIN, &mut native_names)
            .expect("native document decode");

        assert_eq!(
            input_surface(&legacy.schema, &legacy.names),
            input_surface(&native, &native_names),
            "both front ends fill one interface representation for the same source",
        );
    }
}
