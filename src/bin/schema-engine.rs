use std::{env, path::PathBuf};

use schema_engine::Runtime;
use signal_schema::{Reply, Request, encode_reply, encode_request};
use signal_sema_storage::{DocumentKind, FixtureScope, FrameMessage, SlotIdentifier, Wire};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{UnixListener, UnixStream},
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut arguments = env::args().skip(1);
    match arguments.next().as_deref() {
        Some("daemon") => {
            let socket = PathBuf::from(
                arguments
                    .next()
                    .unwrap_or_else(|| "/tmp/new-language-engine/schema.sock".into()),
            );
            let sema = PathBuf::from(
                arguments
                    .next()
                    .unwrap_or_else(|| "/tmp/new-language-engine/sema.sock".into()),
            );
            if let Some(parent) = socket.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let _ = std::fs::remove_file(&socket);
            let listener = UnixListener::bind(&socket)?;
            let runtime = Runtime::new(sema);
            println!("READY {}", socket.display());
            loop {
                let (stream, _) = listener.accept().await?;
                let runtime = runtime.clone();
                tokio::spawn(async move {
                    let _ = serve(stream, runtime).await;
                });
            }
        }
        Some("ingest") => {
            let socket = PathBuf::from(arguments.next().ok_or("socket")?);
            let file = PathBuf::from(arguments.next().ok_or("schema file")?);
            let text = tokio::fs::read_to_string(file).await?;
            let reply = client(
                &socket,
                &Request::IngestTypeSchema {
                    scope: FixtureScope(1),
                    slot: SlotIdentifier(1),
                    legacy_text: text,
                },
            )
            .await?;
            println!("{reply:?}");
            Ok(())
        }
        Some("subscribe") => {
            let socket = PathBuf::from(arguments.next().ok_or("socket")?);
            subscribe(&socket).await
        }
        _ => Err("usage: schema-engine daemon [socket] [sema-socket] | ingest <socket> <file> | subscribe <socket>".into()),
    }
}

struct FramedSocket {
    stream: UnixStream,
    sequence: u64,
}
impl FramedSocket {
    async fn connect(path: &PathBuf) -> Result<Self, Box<dyn std::error::Error>> {
        let mut socket = Self {
            stream: UnixStream::connect(path).await?,
            sequence: 0,
        };
        socket
            .stream
            .write_all(&Wire::frame_current_handshake_request()?)
            .await?;
        if !Wire::decode_frame(&socket.read_frame().await?)?.is_accepted_handshake() {
            return Err("daemon rejected shared frame protocol".into());
        }
        Ok(socket)
    }
    async fn accept(stream: UnixStream) -> Result<Self, Box<dyn std::error::Error>> {
        let mut socket = Self {
            stream,
            sequence: 0,
        };
        let FrameMessage::HandshakeRequest(peer) = Wire::decode_frame(&socket.read_frame().await?)?
        else {
            return Err("first frame was not a protocol handshake".into());
        };
        socket
            .stream
            .write_all(&Wire::frame_handshake_reply(Wire::handshake_reply(peer))?)
            .await?;
        Ok(socket)
    }
    async fn read_frame(&mut self) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        let length = self.stream.read_u32().await? as usize;
        let mut frame = Vec::with_capacity(length + 4);
        frame.extend_from_slice(&(length as u32).to_be_bytes());
        frame.resize(length + 4, 0);
        self.stream.read_exact(&mut frame[4..]).await?;
        Ok(frame)
    }
    async fn request(&mut self, request: &Request) -> Result<(), Box<dyn std::error::Error>> {
        let payload = encode_request(request).map_err(|error| format!("encode: {error}"))?;
        let frame = Wire::frame_request(payload, self.sequence)?;
        self.sequence += 1;
        self.stream.write_all(&frame).await?;
        Ok(())
    }
    async fn reply(&mut self) -> Result<Reply, Box<dyn std::error::Error>> {
        let FrameMessage::Reply { payload, .. } = Wire::decode_frame(&self.read_frame().await?)?
        else {
            return Err("expected shared reply frame".into());
        };
        Ok(rkyv::from_bytes::<Reply, rkyv::rancor::Error>(&payload)?)
    }
}

async fn serve(stream: UnixStream, runtime: Runtime) -> Result<(), Box<dyn std::error::Error>> {
    let mut socket = FramedSocket::accept(stream).await?;
    let FrameMessage::Request { exchange, payload } =
        Wire::decode_frame(&socket.read_frame().await?)?
    else {
        return Err("expected shared request frame".into());
    };
    let request = rkyv::from_bytes::<Request, rkyv::rancor::Error>(&payload)?;
    let subscription_filter = match &request {
        Request::Subscribe { scope, kind } => Some((*scope, *kind)),
        _ => None,
    };
    let mut events = runtime.subscribe();
    let reply = runtime.request(request).await?;
    socket
        .stream
        .write_all(&Wire::frame_reply(
            exchange,
            encode_reply(&reply).map_err(|error| error.to_string())?,
        )?)
        .await?;
    if let Some((scope, kind)) = subscription_filter {
        while let Ok(event) = events.recv().await {
            if event.document.key.scope == scope
                && kind.is_none_or(|expected| event.document.key.kind == expected)
            {
                socket
                    .stream
                    .write_all(&Wire::frame_reply(
                        exchange,
                        encode_reply(&Reply::Event(event)).map_err(|error| error.to_string())?,
                    )?)
                    .await?;
            }
        }
    }
    Ok(())
}

async fn subscribe(socket: &PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    let mut socket = FramedSocket::connect(socket).await?;
    socket
        .request(&Request::Subscribe {
            scope: FixtureScope(1),
            kind: Some(DocumentKind::TypeSchema),
        })
        .await?;
    loop {
        println!("{:?}", socket.reply().await?);
    }
}

async fn client(socket: &PathBuf, request: &Request) -> Result<Reply, Box<dyn std::error::Error>> {
    let mut socket = FramedSocket::connect(socket).await?;
    socket.request(request).await?;
    socket.reply().await
}
