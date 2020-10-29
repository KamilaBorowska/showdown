//! Pokémon Showdown client.
//!
//! # Stability
//!
//! This crate is not stable, not even close. The APIs of this crate are
//! heavily experimented on, and there isn't going to be depreciation period for
//! removed features. Don't use this crate if you aren't prepared for constant
//! breakage.

pub mod message;

use self::message::{Message, Text};
pub use chrono;
use futures::stream::{SplitSink, SplitStream};
use futures::{SinkExt, StreamExt, TryFutureExt};
use serde_derive::Deserialize;
use std::error::Error as StdError;
use std::fmt::{self, Display, Formatter};
use std::result::Result as StdResult;
use tokio::net::TcpStream;
use tokio_native_tls::TlsStream;
use tokio_tungstenite::stream::Stream;
use tokio_tungstenite::tungstenite::{Error as WsError, Message as OwnedMessage};
use tokio_tungstenite::WebSocketStream;
pub use url;
use url::Url;

type SocketStream = WebSocketStream<Stream<TcpStream, TlsStream<TcpStream>>>;

/// Message receiver.
///
/// # Examples
///
/// ```no_run
/// use futures::prelude::*;
/// use showdown::message::{Kind, UpdateUser};
/// use showdown::{connect, Result, RoomId};
///
/// #[tokio::main]
/// async fn main() -> Result<()> {
///     let (_, mut receiver) = connect("showdown").await?;
///     let message = receiver.receive().await?;
///     match message.kind() {
///         Kind::UpdateUser(UpdateUser {
///             username,
///             named: false,
///             ..
///         }) => {
///             assert!(username.starts_with(" Guest "));
///         }
///         _ => panic!(),
///     }
///     Ok(())
/// }
/// ```
pub struct Receiver {
    stream: SplitStream<SocketStream>,
}

impl fmt::Debug for Receiver {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("Receiver").finish()
    }
}

/// Message sender.
pub struct Sender {
    sink: SplitSink<SocketStream, OwnedMessage>,
}

impl fmt::Debug for Sender {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("Sender").finish()
    }
}

impl Sender {
    fn new(sink: SplitSink<SocketStream, OwnedMessage>) -> Sender {
        Self { sink }
    }

    #[deprecated(
        since = "0.7.5",
        note = "Please use .send(SendMessage::global_command(...)) instead"
    )]
    pub async fn send_global_command(&mut self, command: impl Display) -> Result<()> {
        self.send(SendMessage::global_command(command)).await
    }

    /// Sends a message in a chat room.
    #[deprecated(
        since = "0.7.5",
        note = "Please use .send(SendMessage::chat_message(...)) instead"
    )]
    pub async fn send_chat_message(
        &mut self,
        room_id: RoomId<'_>,
        message: impl Display,
    ) -> Result<()> {
        self.send(SendMessage::chat_message(room_id, message)).await
    }

    #[deprecated(
        since = "0.7.5",
        note = "Please use .send(SendMessage::chat_command(...)) instead"
    )]
    pub async fn send_chat_command(
        &mut self,
        room_id: RoomId<'_>,
        command: impl Display,
    ) -> Result<()> {
        self.send(SendMessage::chat_command(room_id, command)).await
    }

    #[deprecated(
        since = "0.7.5",
        note = "Please use .send(SendMessage::broadcast_command(...)) instead"
    )]
    pub async fn broadcast_command(
        &mut self,
        room_id: RoomId<'_>,
        command: impl Display,
    ) -> Result<()> {
        self.send(SendMessage::broadcast_command(room_id, command))
            .await
    }

    pub async fn send(&mut self, message: SendMessage) -> Result<()> {
        Error::from_ws(self.sink.send(OwnedMessage::Text(message.0)).await)
    }
}

pub struct SendMessage(String);

impl SendMessage {
    /// Creates a global command.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use futures::prelude::*;
    /// use showdown::message::{Kind, QueryResponse};
    /// use showdown::{connect, Result, RoomId, SendMessage};
    ///
    /// #[tokio::main]
    /// async fn main() -> Result<()> {
    ///     let (mut sender, mut receiver) = connect("showdown").await?;
    ///     sender.send(SendMessage::global_command("cmd rooms")).await?;
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
    /// ```
    pub fn global_command(command: impl Display) -> Self {
        SendMessage(format!("|/{}", command))
    }

    /// Creates a command that executes in a chat room.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use futures::prelude::*;
    /// use showdown::message::{Kind, QueryResponse};
    /// use showdown::{connect, Result, RoomId, SendMessage};
    ///
    /// #[tokio::main]
    /// async fn main() -> Result<()> {
    ///     let (mut sender, mut receiver) = connect("showdown").await?;
    ///     sender.send(SendMessage::global_command("join lobby")).await?;
    ///     sender.send(SendMessage::chat_message(RoomId::LOBBY, "roomdesc")).await;
    ///     loop {
    ///         if let Kind::Html(html) = receiver.receive().await?.kind() {
    ///             assert!(html.contains("Relax here amidst the chaos."));
    ///             return Ok(());
    ///         }
    ///     }
    /// }
    /// ```
    pub fn chat_message(room_id: RoomId<'_>, message: impl Display) -> Self {
        Self::prefixed(room_id, ' ', message)
    }

    /// Creates a chat room command message.
    pub fn chat_command(room_id: RoomId<'_>, command: impl Display) -> Self {
        Self::prefixed(room_id, '/', command)
    }

    pub fn broadcast_command(room_id: RoomId<'_>, command: impl Display) -> Self {
        Self::prefixed(room_id, '!', command)
    }

    pub fn reply(text: Text, message: impl Display) -> Self {
        match text {
            Text::Chat(chat) => Self::chat_message(chat.room_id(), message),
            Text::Private(private) => {
                Self::global_command(format_args!("pm {},{}", private.from, message))
            }
        }
    }

    fn prefixed(room_id: RoomId<'_>, prefix: char, message: impl Display) -> Self {
        SendMessage(format!("{}|{}{}", room_id.0, prefix, message))
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
/// ```no_run
/// use futures::prelude::*;
/// use showdown::{connect, Result};
///
/// #[tokio::main]
/// async fn main() {
///     assert!(connect("showdown").await.is_ok());
///     assert!(connect("fakestofservers").await.is_err());
/// }
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
/// ```no_run
/// use futures::prelude::*;
/// use showdown::{connect_to_url, fetch_server_url, Result};
///
/// #[tokio::main]
/// async fn main() -> Result<()> {
///     let url = fetch_server_url("smogtours").await?;
///     assert_eq!(url.as_str(), "ws://sim3.psim.us:8002/showdown/websocket");
///     connect_to_url(&url).await?;
///     Ok(())
/// }
/// ```
pub async fn connect_to_url(url: &Url) -> Result<(Sender, Receiver)> {
    let (sink, stream) = Error::from_ws(tokio_tungstenite::connect_async(url).await)?
        .0
        .split();
    Ok((Sender::new(sink), Receiver { stream }))
}

pub async fn fetch_server_url(name: &str) -> Result<Url> {
    let owned_url;
    let url = if name == "showdown" {
        "wss://sim3.psim.us/showdown/websocket"
    } else {
        let Server { host, port } = reqwest::get(&format!(
            "https://pokemonshowdown.com/servers/{}.json",
            name
        ))
        .and_then(|r| r.json())
        .await
        .map_err(|e| Error(ErrorInner::Reqwest(e)))?;
        let protocol = if port == 443 { "wss" } else { "ws" };
        // Concatenation is fine, as it's also done by the official Showdown client
        owned_url = format!("{}://{}:{}/showdown/websocket", protocol, host, port);
        &owned_url
    };
    Url::parse(url).map_err(|e| Error(ErrorInner::Url(e)))
}

impl Receiver {
    pub async fn receive(&mut self) -> Result<Message> {
        let e = self.stream.next().await;
        let message = Error::from_ws(e.transpose())?;
        if let Some(OwnedMessage::Text(raw)) = message {
            Ok(Message { raw })
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

#[derive(Copy, Clone, Debug)]
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
    Reqwest(reqwest::Error),
    Url(url::ParseError),
    Json(serde_json::Error),
    UnrecognizedMessage(Option<OwnedMessage>),
}

impl Display for Error {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match &self.0 {
            ErrorInner::WebSocket(e) => e.fmt(f),
            ErrorInner::Reqwest(e) => e.fmt(f),
            ErrorInner::Url(e) => e.fmt(f),
            ErrorInner::Json(e) => e.fmt(f),
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
            ErrorInner::Json(e) => Some(e),
            ErrorInner::UnrecognizedMessage(_) => None,
        }
    }
}
