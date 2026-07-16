use core_schema::{
    CoreDeclaration, CoreEnum, CoreField, CoreNewtype, CoreReference, CoreSchema, CoreStruct,
    CoreType, CoreVariant, MultiTypeReferenceProjection, SingleTypeReferenceProjection,
    ValueReferenceProjection, Visibility,
};
use name_table::{Identifier, Name, NameTable};
use schema_language::{
    Declaration, MultiTypeReferenceProjection as LegacyMultiProjection, SchemaEngine,
    SchemaIdentity, SingleTypeReferenceProjection as LegacySingleProjection, TypeDeclaration,
    TypeReference, ValueReferenceProjection as LegacyValueProjection,
};

#[derive(Debug, thiserror::Error)]
pub enum LegacyIngestError {
    #[error("legacy schema lowering: {0}")]
    Lower(#[from] schema_language::SchemaError),
    #[error("unsupported legacy Path scalar")]
    UnsupportedPath,
    #[error("unsupported user-defined generic application")]
    UnsupportedApplication,
}

pub struct LegacyMigration {
    pub schema: CoreSchema,
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
        let declarations = source
            .namespace()
            .iter()
            .map(|declaration| ingest.migrate_declaration(declaration))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(LegacyMigration {
            schema: CoreSchema::new(declarations),
            names: ingest.names,
        })
    }

    fn migrate_declaration(
        &mut self,
        declaration: &Declaration,
    ) -> Result<CoreDeclaration, LegacyIngestError> {
        let value = match declaration.value() {
            TypeDeclaration::Newtype(newtype) => CoreType::Newtype(CoreNewtype::new(
                self.intern(newtype.name.as_str()),
                self.migrate_reference(&newtype.reference)?,
            )),
            TypeDeclaration::Struct(structure) => {
                let fields = structure
                    .fields
                    .iter()
                    .map(|field| {
                        Ok(CoreField::new(
                            self.intern(field.name.as_str()),
                            self.migrate_reference(&field.reference)?,
                        ))
                    })
                    .collect::<Result<Vec<_>, LegacyIngestError>>()?;
                CoreType::Struct(CoreStruct::new(
                    self.intern(structure.name.as_str()),
                    fields,
                ))
            }
            TypeDeclaration::Enum(enumeration) => {
                let variants = enumeration
                    .variants
                    .iter()
                    .map(|variant| {
                        Ok(CoreVariant::new(
                            self.intern(variant.name.as_str()),
                            variant
                                .payload
                                .as_ref()
                                .map(|payload| self.migrate_reference(payload))
                                .transpose()?,
                        ))
                    })
                    .collect::<Result<Vec<_>, LegacyIngestError>>()?;
                CoreType::Enumeration(CoreEnum::new(
                    self.intern(enumeration.name.as_str()),
                    variants,
                ))
            }
        };
        let visibility = if declaration.is_private() {
            Visibility::Private
        } else {
            Visibility::Public
        };
        Ok(CoreDeclaration::new(visibility, value))
    }

    fn migrate_reference(
        &mut self,
        reference: &TypeReference,
    ) -> Result<CoreReference, LegacyIngestError> {
        Ok(match reference {
            TypeReference::String => CoreReference::String,
            TypeReference::Integer => CoreReference::Integer,
            TypeReference::Boolean => CoreReference::Boolean,
            TypeReference::Bytes => CoreReference::Bytes,
            TypeReference::Path => return Err(LegacyIngestError::UnsupportedPath),
            TypeReference::Plain(name) => CoreReference::Plain(self.intern(name.as_str())),
            TypeReference::SingleTypeApplication {
                projection,
                argument,
            } => CoreReference::SingleTypeApplication {
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
            } => CoreReference::MultiTypeApplication {
                projection: match projection {
                    LegacyMultiProjection::Map => MultiTypeReferenceProjection::Map,
                },
                arguments: arguments
                    .iter()
                    .map(|argument| self.migrate_reference(argument))
                    .collect::<Result<Vec<_>, _>>()?,
            },
            TypeReference::ValueApplication { projection, value } => {
                CoreReference::ValueApplication {
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
