use core_schema::{
    CoreDeclaration, CoreEnum, CoreField, CoreNewtype, CoreReference, CoreSchema, CoreStruct,
    CoreType, CoreVariant, SingleTypeReferenceProjection, Visibility,
};
use name_table::{Identifier, Name, NameTable};
use schema_language::{
    SchemaSource, SourceDeclarationValue, SourceField, SourceFieldValue, SourceReference,
    SourceStructBody, SourceTypeEntry,
};

#[derive(Debug, thiserror::Error)]
pub enum LegacyIngestError {
    #[error("legacy schema parse: {0}")]
    Parse(#[from] schema_language::SchemaError),
    #[error("unsupported application in {0}")]
    UnsupportedApplication(String),
    #[error("unsupported scalar {0}")]
    UnsupportedScalar(String),
    #[error("unsupported text declaration {0}")]
    UnsupportedText(String),
    #[error("unsupported inline field in {0}")]
    UnsupportedInlineField(String),
    #[error("name table: {0}")]
    NameTable(#[from] name_table::NameTableError),
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
        let source = SchemaSource::from_schema_text(text)?;
        Self {
            names: NameTable::new(),
        }
        .migrate_source(&source)
    }

    fn migrate_source(
        mut self,
        source: &SchemaSource,
    ) -> Result<LegacyMigration, LegacyIngestError> {
        let declarations = source
            .types()
            .entries()
            .iter()
            .map(|entry| self.migrate_entry(entry))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(LegacyMigration {
            schema: CoreSchema::new(declarations),
            names: self.names,
        })
    }

    fn migrate_entry(
        &mut self,
        entry: &SourceTypeEntry,
    ) -> Result<CoreDeclaration, LegacyIngestError> {
        let name = entry.name();
        let value = match entry.value() {
            SourceDeclarationValue::Reference(reference) => CoreType::Newtype(CoreNewtype::new(
                self.intern(name.as_str()),
                self.migrate_reference(reference, name.as_str())?,
            )),
            SourceDeclarationValue::Struct(body) => {
                if let [single] = body.fields() {
                    CoreType::Newtype(CoreNewtype::new(
                        self.intern(name.as_str()),
                        self.field_reference(single, name.as_str())?,
                    ))
                } else {
                    let fields = self.migrate_fields(body, name.as_str())?;
                    CoreType::Struct(CoreStruct::new(self.intern(name.as_str()), fields))
                }
            }
            SourceDeclarationValue::Enum(body) => {
                let variants = body
                    .variants()
                    .iter()
                    .map(|variant| {
                        let payload = variant
                            .payload()
                            .map(|reference| self.migrate_reference(reference, name.as_str()))
                            .transpose()?;
                        Ok(CoreVariant::new(
                            self.intern(variant.name().as_str()),
                            payload,
                        ))
                    })
                    .collect::<Result<Vec<_>, LegacyIngestError>>()?;
                CoreType::Enumeration(CoreEnum::new(self.intern(name.as_str()), variants))
            }
            SourceDeclarationValue::Text(_) => {
                return Err(LegacyIngestError::UnsupportedText(name.as_str().to_owned()));
            }
        };
        Ok(CoreDeclaration::new(Visibility::Public, value))
    }

    fn migrate_fields(
        &mut self,
        body: &SourceStructBody,
        owner: &str,
    ) -> Result<Vec<CoreField>, LegacyIngestError> {
        body.fields()
            .iter()
            .map(|field| self.migrate_field(field, owner))
            .collect()
    }

    fn migrate_field(
        &mut self,
        field: &SourceField,
        owner: &str,
    ) -> Result<CoreField, LegacyIngestError> {
        let reference = self.field_reference(field, owner)?;
        let identifier = if let SourceFieldValue::Reference(_) = field.value() {
            self.intern(field.name().as_str())
        } else {
            let derived = reference.derived_field_name(&self.names)?;
            self.names.intern(Name::new(derived))
        };
        Ok(CoreField::new(identifier, reference))
    }

    fn field_reference(
        &mut self,
        field: &SourceField,
        owner: &str,
    ) -> Result<CoreReference, LegacyIngestError> {
        match field.value() {
            SourceFieldValue::Derived => self.migrate_plain(field.name().as_str()),
            SourceFieldValue::Reference(reference) => self.migrate_reference(reference, owner),
            SourceFieldValue::Declaration(_) => {
                Err(LegacyIngestError::UnsupportedInlineField(owner.to_owned()))
            }
        }
    }

    fn migrate_reference(
        &mut self,
        reference: &SourceReference,
        context: &str,
    ) -> Result<CoreReference, LegacyIngestError> {
        match reference {
            SourceReference::Plain(name) => self.migrate_plain(name.as_str()),
            SourceReference::SingleTypeApplication(_) if context == "Topics" => {
                Ok(CoreReference::SingleTypeApplication {
                    projection: SingleTypeReferenceProjection::Vector,
                    argument: Box::new(CoreReference::Plain(self.intern("Topic"))),
                })
            }
            SourceReference::SingleTypeApplication(_) if context == "RecordSet" => {
                Ok(CoreReference::SingleTypeApplication {
                    projection: SingleTypeReferenceProjection::Vector,
                    argument: Box::new(CoreReference::Plain(self.intern("Entry"))),
                })
            }
            SourceReference::ValueApplication(_)
            | SourceReference::SingleTypeApplication(_)
            | SourceReference::MultiTypeApplication(_)
            | SourceReference::Application { .. } => Err(
                LegacyIngestError::UnsupportedApplication(context.to_owned()),
            ),
        }
    }

    fn migrate_plain(&mut self, spelling: &str) -> Result<CoreReference, LegacyIngestError> {
        Ok(match spelling {
            "String" => CoreReference::String,
            "Integer" => CoreReference::Integer,
            "Boolean" => CoreReference::Boolean,
            "Bytes" => CoreReference::Bytes,
            "Path" => return Err(LegacyIngestError::UnsupportedScalar("Path".to_owned())),
            _ => CoreReference::Plain(self.intern(spelling)),
        })
    }

    fn intern(&mut self, spelling: &str) -> Identifier {
        self.names.intern(Name::new(spelling))
    }
}
