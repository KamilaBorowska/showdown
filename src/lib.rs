#![feature(async_await, await_macro, futures_api, transpose_result)]

pub mod message;

use self::message::Message;
use futures::sync::mpsc;
use reqwest::r#async::Client;
use serde_derive::Deserialize;
use std::error::Error as StdError;
use std::fmt::{self, Display, Formatter};
use std::result::Result as StdResult;
use std::time::{Duration, Instant};
use tokio::await;
use tokio::prelude::stream::{SplitSink, SplitStream};
use tokio::prelude::*;
use tokio::timer::Delay;
use websocket::r#async::TcpStream;
pub use websocket::url;
use websocket::url::Url;
use websocket::{ClientBuilder, OwnedMessage, WebSocketError};

pub struct Receiver {
    stream: SplitStream<websocket::r#async::Client<TcpStream>>,
}

impl fmt::Debug for Receiver {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        f.debug_struct("Receiver").finish()
    }
}

#[derive(Debug)]
pub struct Sender {
    sender: mpsc::Sender<OwnedMessage>,
}

impl Sender {
    fn new(mut sink: SplitSink<websocket::r#async::Client<TcpStream>>) -> Sender {
        let (sender, mut receiver) = mpsc::channel(10);
        tokio::spawn_async(
            async move {
                while let Some(m) = await!(receiver.next()) {
                    await!((&mut sink).send(m.unwrap())).unwrap();
                    await!(Delay::new(Instant::now() + Duration::from_millis(600))).unwrap();
                }
            },
        );
        Self { sender }
    }

    pub fn send_global_command(
        &mut self,
        command: &str,
    ) -> impl Future<Item = (), Error = Error> + '_ {
        let message = format!("|/{}", command);
        (&mut self.sender)
            .send(OwnedMessage::Text(message))
            .map(|_| ())
            .map_err(|e| Error(ErrorInner::Mpsc(e)))
    }
}

pub async fn connect(name: &str) -> Result<(Sender, Receiver)> {
    let url = await!(fetch_server_url(name))?;
    await!(connect_to_url(&url))
}

pub async fn connect_to_url(url: &Url) -> Result<(Sender, Receiver)> {
    let (sink, stream) =
        Error::from_ws(await!(ClientBuilder::from_url(url).async_connect_insecure()))?
            .0
            .split();
    Ok((Sender::new(sink), Receiver { stream }))
}

pub async fn fetch_server_url(name: &str) -> Result<Url> {
    let Server { host, port } = await!(Client::new()
        .get(&format!(
            "https://pokemonshowdown.com/servers/{}.json",
            name
        ))
        .send()
        .and_then(|mut r| r.json())
        .map_err(|e| Error(ErrorInner::Reqwest(e))))?;
    let protocol = if port == 443 { "wss" } else { "ws" };
    // Concatenation is fine, as it's also done by the official Showdown client
    Url::parse(&format!(
        "{}://{}:{}/showdown/websocket",
        protocol, host, port
    ))
    .map_err(|e| Error(ErrorInner::Url(e)))
}

impl Receiver {
    pub async fn receive(&mut self) -> Result<Message> {
        let message = Error::from_ws(await!((&mut self.stream).next()).transpose())?;
        if let Some(OwnedMessage::Text(text)) = message {
            Ok(Message { text })
        } else {
            Err(Error(ErrorInner::UnrecognizedMessage(message)))
        }
    }
}

#[derive(Deserialize)]
struct Server {
    host: String,
    port: u16,
}

#[derive(Debug, PartialEq, Eq, Hash)]
pub struct RoomId<'a>(&'a str);

impl RoomId<'_> {
    pub const LOBBY: RoomId<'static> = RoomId("");
}

pub type Result<T> = StdResult<T, Error>;

#[derive(Debug)]
pub struct Error(ErrorInner);

impl Error {
    fn from_ws<T>(r: StdResult<T, WebSocketError>) -> Result<T> {
        r.map_err(|e| Error(ErrorInner::WebSocket(e)))
    }
}

#[derive(Debug)]
enum ErrorInner {
    WebSocket(WebSocketError),
    Reqwest(reqwest::Error),
    Url(url::ParseError),
    Mpsc(mpsc::SendError<OwnedMessage>),
    UnrecognizedMessage(Option<OwnedMessage>),
}

impl Display for Error {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        match &self.0 {
            ErrorInner::WebSocket(e) => e.fmt(f),
            ErrorInner::Reqwest(e) => e.fmt(f),
            ErrorInner::Url(e) => e.fmt(f),
            ErrorInner::Mpsc(e) => e.fmt(f),
            ErrorInner::UnrecognizedMessage(e) => write!(f, "Unrecognized message: {:?}", e),
        }
    }
}

impl StdError for Error {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match &self.0 {
            ErrorInner::WebSocket(e) => Some(e),
            ErrorInner::Reqwest(e) => Some(e),
            ErrorInner::Url(e) => Some(e),
            ErrorInner::Mpsc(e) => Some(e),
            ErrorInner::UnrecognizedMessage(_) => None,
        }
    }
}
