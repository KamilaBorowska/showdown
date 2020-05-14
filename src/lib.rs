//! Pokémon Showdown client.
//!
//! # Stability
//!
//! This crate is not stable, not even close. The APIs of this crate are
//! heavily experimented on, and there isn't going to be depreciation period for
//! removed features. Don't use this crate if you aren't prepared for constant
//! breakage.

#[macro_use]
extern crate rental;

pub mod message;

use self::message::Message;
pub use chrono;
use futures::stream::SplitStream;
use futures::{Sink, SinkExt, StreamExt};
use serde_derive::Deserialize;
use std::error::Error as StdError;
use std::fmt::{self, Display, Formatter};
use std::result::Result as StdResult;
use std::time::Duration;
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio::time;
use tokio_tls::TlsStream;
use tokio_tungstenite::stream::Stream;
use tokio_tungstenite::tungstenite::{Error as WsError, Message as OwnedMessage};
use tokio_tungstenite::WebSocketStream;
pub use url;
use url::Url;

/// Message receiver.
///
/// # Examples
///
/// ```
/// use futures::prelude::*;
/// use showdown::message::{Kind, UpdateUser};
/// use showdown::{connect, Result, RoomId};
/// use tokio::runtime::Runtime;
///
/// async fn start() -> Result<()> {
///     let (_, mut receiver) = connect("showdown").await?;
///     let message = receiver.receive().await?;
///     match message.kind() {
///         Kind::UpdateUser(UpdateUser {
///             username,
///             named: false,
///             ..
///         }) if message.room_id() == RoomId("") => {
///             assert!(username.starts_with(" Guest "));
///         }
///         _ => panic!(),
///     }
///     Ok(())
/// }
///
/// Runtime::new().unwrap().block_on(start()).unwrap();
/// ```
pub struct Receiver {
    stream: SplitStream<WebSocketStream<Stream<TcpStream, TlsStream<TcpStream>>>>,
}

impl fmt::Debug for Receiver {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("Receiver").finish()
    }
}

/// Message sender.
#[derive(Debug)]
pub struct Sender {
    sender: mpsc::Sender<StdResult<OwnedMessage, WsError>>,
}

impl Sender {
    fn new(sink: impl Sink<OwnedMessage, Error = WsError> + Send + 'static) -> Sender {
        let (sender, receiver) = mpsc::channel(1);
        tokio::spawn(
            time::throttle(Duration::from_millis(600), receiver)
                .forward(sink.sink_map_err(|e| panic!(e))),
        );
        Self { sender }
    }

    /// Sends a global command.
    ///
    /// # Example
    ///
    /// ```
    /// use futures::prelude::*;
    /// use showdown::message::{Kind, QueryResponse};
    /// use showdown::{connect, Result, RoomId};
    /// use tokio::runtime::Runtime;
    ///
    /// async fn start() -> Result<()> {
    ///     let (mut sender, mut receiver) = connect("showdown").await?;
    ///     sender.send_global_command("cmd rooms").await?;
    ///     loop {
    ///         let received = receiver.receive().await?;
    ///         if let Kind::QueryResponse(QueryResponse::Rooms(rooms)) = received.kind() {
    ///             assert!(rooms
    ///                 .official
    ///                 .iter()
    ///                 .any(|room| room.title == "Tournaments"));
    ///             return Ok(());
    ///         }
    ///     }
    /// }
    ///
    /// Runtime::new().unwrap().block_on(start()).unwrap();
    /// ```
    pub async fn send_global_command(&mut self, command: &str) -> Result<()> {
        self.send(format!("|/{}", command)).await
    }

    /// Sends a message in a chat room.
    ///
    /// # Examples
    ///
    /// ```
    /// use futures::prelude::*;
    /// use showdown::message::{Kind, QueryResponse};
    /// use showdown::{connect, Result, RoomId};
    /// use tokio::runtime::Runtime;
    ///
    /// async fn start() -> Result<()> {
    ///     let (mut sender, mut receiver) = connect("showdown").await?;
    ///     sender.send_global_command("join lobby").await?;
    ///     sender.send_chat_message(RoomId::LOBBY, "/roomdesc").await;
    ///     loop {
    ///         if let Kind::Html(html) = receiver.receive().await?.kind() {
    ///             assert!(html.contains("Relax here amidst the chaos."));
    ///             return Ok(());
    ///         }
    ///     }
    /// }
    ///
    /// Runtime::new().unwrap().block_on(start()).unwrap();
    /// ```
    pub async fn send_chat_message(&mut self, room_id: RoomId<'_>, message: &str) -> Result<()> {
        self.send(format!("{}|{}", room_id.0, message)).await
    }

    async fn send(&mut self, message: String) -> Result<()> {
        self.sender
            .send(Ok(OwnedMessage::Text(message)))
            .await
            .map_err(|e| Error(ErrorInner::Mpsc(e)))
    }
}

/// Connects to a named Showdown server.
///
/// Returns two structures, [`Sender`] can be used to send messages to Showdown,
/// while [`Receiver`] can be used to retrieve messages from Showdown. Due to
/// borrow checker, those structures are separate - it's practically necessary
/// to implement anything interesting.
///
/// # Examples
///
/// ```
/// use futures::prelude::*;
/// use showdown::{connect, Result};
/// use tokio::runtime::Runtime;
///
/// async fn start() {
///     assert!(connect("showdown").await.is_ok());
///     assert!(connect("fakestofservers").await.is_err());
/// }
///
/// Runtime::new().unwrap().block_on(start());
/// ```
pub async fn connect(name: &str) -> Result<(Sender, Receiver)> {
    connect_to_url(&fetch_server_url(name).await?).await
}

/// Connects to an URL.
///
/// This URL is provided by [`fetch_server_url`] function.
///
/// # Examples
///
/// ```rust
/// use futures::prelude::*;
/// use showdown::{connect_to_url, fetch_server_url, Result};
/// use tokio::runtime::Runtime;
///
/// async fn start() -> Result<()> {
///     let url = fetch_server_url("showdown").await?;
///     assert_eq!(url.as_str(), "ws://sim3.psim.us:8000/showdown/websocket");
///     connect_to_url(&url).await?;
///     Ok(())
/// }
///
/// Runtime::new().unwrap().block_on(start()).unwrap();
/// ```
pub async fn connect_to_url(url: &Url) -> Result<(Sender, Receiver)> {
    let (sink, stream) = Error::from_ws(tokio_tungstenite::connect_async(url).await)?
        .0
        .split();
    Ok((Sender::new(sink), Receiver { stream }))
}

pub async fn fetch_server_url(name: &str) -> Result<Url> {
    let Server { host, port } = surf::get(&format!(
        "https://pokemonshowdown.com/servers/{}.json",
        name
    ))
    .recv_json()
    .await
    .map_err(|e| Error(ErrorInner::Surf(e)))?;
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
        let e = self.stream.next().await;
        let message = Error::from_ws(e.transpose())?;
        if let Some(OwnedMessage::Text(text)) = message {
            Ok(Message::new(text))
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

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct RoomId<'a>(pub &'a str);

impl RoomId<'_> {
    pub const LOBBY: RoomId<'static> = RoomId("");
}

pub type Result<T> = StdResult<T, Error>;

/// A specialized `Result` type for Showdown client operations.
#[derive(Debug)]
pub struct Error(ErrorInner);

impl Error {
    fn from_ws<T>(r: StdResult<T, tokio_tungstenite::tungstenite::Error>) -> Result<T> {
        r.map_err(|e| Error(ErrorInner::WebSocket(e)))
    }
}

#[derive(Debug)]
enum ErrorInner {
    WebSocket(WsError),
    Surf(surf::Exception),
    Url(url::ParseError),
    Mpsc(mpsc::error::SendError<StdResult<OwnedMessage, WsError>>),
    Json(serde_json::Error),
    UnrecognizedMessage(Option<OwnedMessage>),
}

impl Display for Error {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match &self.0 {
            ErrorInner::WebSocket(e) => e.fmt(f),
            ErrorInner::Surf(e) => e.fmt(f),
            ErrorInner::Url(e) => e.fmt(f),
            ErrorInner::Mpsc(e) => e.fmt(f),
            ErrorInner::Json(e) => e.fmt(f),
            ErrorInner::UnrecognizedMessage(e) => write!(f, "Unrecognized message: {:?}", e),
        }
    }
}

impl StdError for Error {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match &self.0 {
            ErrorInner::WebSocket(e) => Some(e),
            ErrorInner::Surf(e) => Some(&**e),
            ErrorInner::Url(e) => Some(e),
            ErrorInner::Mpsc(e) => Some(e),
            ErrorInner::Json(e) => Some(e),
            ErrorInner::UnrecognizedMessage(_) => None,
        }
    }
}
