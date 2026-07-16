use std::{env, path::PathBuf};

use schema_engine::Runtime;
use signal_schema::{Reply, Request, encode_reply, encode_request};
use signal_sema_storage::{DocumentKind, FixtureScope, SlotIdentifier};
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
            let listener = UnixListener::bind(socket)?;
            let runtime = Runtime::new(sema);
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

async fn serve(mut stream: UnixStream, runtime: Runtime) -> Result<(), Box<dyn std::error::Error>> {
    let request = read_request(&mut stream).await?;
    let subscription_filter = match &request {
        Request::Subscribe { scope, kind } => Some((*scope, *kind)),
        _ => None,
    };
    let mut events = runtime.subscribe();
    write_reply(&mut stream, &runtime.request(request).await?).await?;
    if let Some((scope, kind)) = subscription_filter {
        while let Ok(event) = events.recv().await {
            if event.document.key.scope == scope
                && kind.is_none_or(|expected| event.document.key.kind == expected)
            {
                write_reply(&mut stream, &Reply::Event(event)).await?;
            }
        }
    }
    Ok(())
}

async fn subscribe(socket: &PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    let mut stream = UnixStream::connect(socket).await?;
    write_request(
        &mut stream,
        &Request::Subscribe {
            scope: FixtureScope(1),
            kind: Some(DocumentKind::TypeSchema),
        },
    )
    .await?;
    loop {
        println!("{:?}", read_reply(&mut stream).await?);
    }
}

async fn client(socket: &PathBuf, request: &Request) -> Result<Reply, Box<dyn std::error::Error>> {
    let mut stream = UnixStream::connect(socket).await?;
    write_request(&mut stream, request).await?;
    read_reply(&mut stream).await
}

async fn read_request(stream: &mut UnixStream) -> Result<Request, Box<dyn std::error::Error>> {
    let length = stream.read_u32_le().await? as usize;
    let mut bytes = vec![0; length];
    stream.read_exact(&mut bytes).await?;
    Ok(rkyv::from_bytes::<Request, rkyv::rancor::Error>(&bytes)?)
}
async fn write_request(
    stream: &mut UnixStream,
    request: &Request,
) -> Result<(), Box<dyn std::error::Error>> {
    let bytes = encode_request(request).map_err(|error| format!("encode: {error}"))?;
    stream.write_u32_le(bytes.len() as u32).await?;
    stream.write_all(&bytes).await?;
    Ok(())
}
async fn read_reply(stream: &mut UnixStream) -> Result<Reply, Box<dyn std::error::Error>> {
    let length = stream.read_u32_le().await? as usize;
    let mut bytes = vec![0; length];
    stream.read_exact(&mut bytes).await?;
    Ok(rkyv::from_bytes::<Reply, rkyv::rancor::Error>(&bytes)?)
}
async fn write_reply(
    stream: &mut UnixStream,
    reply: &Reply,
) -> Result<(), Box<dyn std::error::Error>> {
    let bytes = encode_reply(reply).map_err(|error| format!("encode: {error}"))?;
    stream.write_u32_le(bytes.len() as u32).await?;
    stream.write_all(&bytes).await?;
    Ok(())
}
