use std::{convert::Infallible, path::PathBuf};

pub mod authority_ingest;
mod legacy_ingest;

pub use authority_ingest::{AuthorityIngestError, ParsedSchema};

use kameo::{
    Actor,
    actor::{ActorRef, Spawn},
    message::{Context, Message},
};
use legacy_ingest::LegacySchemaIngest;
use signal_schema::{Rejection as SchemaRejection, Reply as SchemaReply, Request as SchemaRequest};
use signal_sema_storage::{
    ChangeEvent, DocumentKey, DocumentKind, DocumentPayload, FrameMessage, NameTableBytes,
    Reply as SemaReply, Request as SemaRequest, Snapshot, SubscriptionIdentifier, Wire,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::UnixStream,
    sync::broadcast,
};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("codec: {0}")]
    Codec(String),
    #[error("actor: {0}")]
    Actor(String),
}
type Result<T> = std::result::Result<T, Error>;

pub struct SemaPlane {
    socket: PathBuf,
    commits: u64,
}
impl SemaPlane {
    async fn read_frame(&self, stream: &mut UnixStream) -> Result<Vec<u8>> {
        let length = stream.read_u32().await? as usize;
        let mut frame = Vec::with_capacity(length + 4);
        frame.extend_from_slice(&(length as u32).to_be_bytes());
        frame.resize(length + 4, 0);
        stream.read_exact(&mut frame[4..]).await?;
        Ok(frame)
    }

    async fn exchange(&self, request: &SemaRequest) -> Result<SemaReply> {
        let mut stream = UnixStream::connect(&self.socket).await?;
        stream
            .write_all(
                &Wire::frame_current_handshake_request()
                    .map_err(|error| Error::Codec(error.to_string()))?,
            )
            .await?;
        let handshake = Wire::decode_frame(&self.read_frame(&mut stream).await?)
            .map_err(|error| Error::Codec(error.to_string()))?;
        if !handshake.is_accepted_handshake() {
            return Err(Error::Codec("Sema rejected frame protocol".into()));
        }
        let payload =
            Wire::encode_request(request).map_err(|error| Error::Codec(error.to_string()))?;
        stream
            .write_all(
                &Wire::frame_request(payload, self.commits)
                    .map_err(|error| Error::Codec(error.to_string()))?,
            )
            .await?;
        let FrameMessage::Reply { payload, .. } =
            Wire::decode_frame(&self.read_frame(&mut stream).await?)
                .map_err(|error| Error::Codec(error.to_string()))?
        else {
            return Err(Error::Codec("Sema returned a non-reply frame".into()));
        };
        rkyv::from_bytes::<SemaReply, rkyv::rancor::Error>(&payload)
            .map_err(|error| Error::Codec(error.to_string()))
    }
}
impl Actor for SemaPlane {
    type Args = Self;
    type Error = Infallible;
    async fn on_start(
        actor: Self::Args,
        _: ActorRef<Self>,
    ) -> std::result::Result<Self, Self::Error> {
        Ok(actor)
    }
}
pub struct Commit(pub SemaRequest);
impl Message<Commit> for SemaPlane {
    type Reply = Result<SemaReply>;
    async fn handle(&mut self, message: Commit, _: &mut Context<Self, Self::Reply>) -> Self::Reply {
        self.commits += 1;
        self.exchange(&message.0).await
    }
}

pub struct NexusPlane {
    sema: ActorRef<SemaPlane>,
    events: broadcast::Sender<ChangeEvent>,
    transforms: u64,
}
impl Actor for NexusPlane {
    type Args = Self;
    type Error = Infallible;
    async fn on_start(
        actor: Self::Args,
        _: ActorRef<Self>,
    ) -> std::result::Result<Self, Self::Error> {
        Ok(actor)
    }
}
pub struct Dispatch(pub SchemaRequest);
impl Message<Dispatch> for NexusPlane {
    type Reply = Result<SchemaReply>;
    async fn handle(
        &mut self,
        message: Dispatch,
        _: &mut Context<Self, Self::Reply>,
    ) -> Self::Reply {
        self.transforms += 1;
        let request = match message.0 {
            // LEAN `offline-self-contained-ingest`: the `IngestTypeSchema` path migrates
            // legacy text and stores the resulting schema DIRECTLY with its own
            // parse-order name table — it never consults the central identity authority,
            // so its identifiers are parse-order interned rather than authority-assigned.
            // This is the offline mode; the authority-bound online path
            // ([`authority_ingest::ParsedSchema`]) realizes the keystone. Revision
            // trigger: wiring the daemon's default ingestion through the authority (a
            // BindIdentities round-trip before Store), once the projection consumers read
            // authority-assigned universes.
            SchemaRequest::IngestTypeSchema {
                scope,
                slot,
                legacy_text,
            } => {
                let migration = tokio::task::spawn_blocking(move || {
                    LegacySchemaIngest::migrate_text(&legacy_text)
                })
                .await
                .map_err(|error| Error::Actor(error.to_string()))?
                .map_err(|error| Error::Codec(error.to_string()))?;
                let names = NameTableBytes(
                    migration
                        .names
                        .to_archive_bytes()
                        .map_err(|error| Error::Codec(error.to_string()))?
                        .to_vec(),
                );
                SemaRequest::Store {
                    key: DocumentKey {
                        scope,
                        kind: DocumentKind::TypeSchema,
                        slot,
                    },
                    payload: DocumentPayload::TypeSchema {
                        schema: migration.schema,
                        names,
                    },
                }
            }
            SchemaRequest::StoreSignalContract { scope, slot, root } => SemaRequest::Store {
                key: DocumentKey {
                    scope,
                    kind: DocumentKind::SignalContract,
                    slot,
                },
                payload: DocumentPayload::SignalContract(root),
            },
            SchemaRequest::StoreNexusRuntime { scope, slot, root } => SemaRequest::Store {
                key: DocumentKey {
                    scope,
                    kind: DocumentKind::NexusRuntime,
                    slot,
                },
                payload: DocumentPayload::NexusRuntime(root),
            },
            SchemaRequest::StoreSemaStorage { scope, slot, root } => SemaRequest::Store {
                key: DocumentKey {
                    scope,
                    kind: DocumentKind::SemaStorage,
                    slot,
                },
                payload: DocumentPayload::SemaStorage(root),
            },
            SchemaRequest::List { scope, kind } => SemaRequest::List { scope, kind },
            SchemaRequest::Fetch { hash } => SemaRequest::HashFetch { hash },
            SchemaRequest::Subscribe { scope, kind } => SemaRequest::Subscribe { scope, kind },
        };
        let reply = self
            .sema
            .ask(Commit(request))
            .send()
            .await
            .map_err(|error| Error::Actor(error.to_string()))?;
        Ok(match reply {
            SemaReply::Stored(summary) => {
                let _ = self.events.send(ChangeEvent {
                    subscription: SubscriptionIdentifier(0),
                    snapshot: Snapshot(summary.version.0),
                    document: summary.clone(),
                });
                SchemaReply::Stored(summary)
            }
            SemaReply::Listed(values) => SchemaReply::Listed(values),
            SemaReply::Document(value) => {
                SchemaReply::Fetched(value.map(|document| signal_sema_storage::SlotSummary {
                    key: document.key,
                    version: document.version,
                    hash: document.hash,
                }))
            }
            SemaReply::Subscribed {
                identifier,
                initial,
            } => SchemaReply::Subscribed {
                identifier,
                initial,
            },
            SemaReply::Rejected(signal_sema_storage::Rejection::InvalidDocument(_)) => {
                SchemaReply::Rejected(SchemaRejection::InvalidRoot)
            }
            _ => SchemaReply::Rejected(SchemaRejection::StorageFailed),
        })
    }
}

pub struct SignalPlane {
    nexus: ActorRef<NexusPlane>,
    admitted: u64,
}
impl Actor for SignalPlane {
    type Args = Self;
    type Error = Infallible;
    async fn on_start(
        actor: Self::Args,
        _: ActorRef<Self>,
    ) -> std::result::Result<Self, Self::Error> {
        Ok(actor)
    }
}
impl Message<Dispatch> for SignalPlane {
    type Reply = Result<SchemaReply>;
    async fn handle(
        &mut self,
        message: Dispatch,
        _: &mut Context<Self, Self::Reply>,
    ) -> Self::Reply {
        self.admitted += 1;
        self.nexus
            .ask(message)
            .send()
            .await
            .map_err(|error| Error::Actor(error.to_string()))
    }
}

#[derive(Clone)]
pub struct Runtime {
    signal: ActorRef<SignalPlane>,
    events: broadcast::Sender<ChangeEvent>,
}
impl Runtime {
    pub fn new(sema_socket: PathBuf) -> Self {
        let sema = SemaPlane::spawn(SemaPlane {
            socket: sema_socket,
            commits: 0,
        });
        let (events, _) = broadcast::channel(64);
        let nexus = NexusPlane::spawn(NexusPlane {
            sema,
            events: events.clone(),
            transforms: 0,
        });
        Self {
            signal: SignalPlane::spawn(SignalPlane { nexus, admitted: 0 }),
            events,
        }
    }
    pub async fn request(&self, request: SchemaRequest) -> Result<SchemaReply> {
        self.signal
            .ask(Dispatch(request))
            .send()
            .await
            .map_err(|error| Error::Actor(error.to_string()))
    }
    pub fn subscribe(&self) -> broadcast::Receiver<ChangeEvent> {
        self.events.subscribe()
    }
}
