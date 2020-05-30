//! Pokémon Showdown client.
//!
//! # Stability
//!
//! This crate is not stable, not even close. The APIs of this crate are
//! heavily experimented on, and there isn't going to be depreciation period for
//! removed features. Don't use this crate if you aren't prepared for constant
//! breakage.

pub mod message;

use self::message::Message;
pub use chrono;
use futures::stream::{SplitSink, SplitStream};
use futures::{SinkExt, StreamExt};
use serde_derive::Deserialize;
use std::error::Error as StdError;
use std::fmt::{self, Display, Formatter};
use std::result::Result as StdResult;
use tokio::net::TcpStream;
use tokio_tls::TlsStream;
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

    /// Sends a global command.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use futures::prelude::*;
    /// use showdown::message::{Kind, QueryResponse};
    /// use showdown::{connect, Result, RoomId};
    ///
    /// #[tokio::main]
    /// async fn main() -> Result<()> {
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
    /// ```
    pub async fn send_global_command(&mut self, command: impl Display) -> Result<()> {
        self.send(format!("|/{}", command)).await
    }

    /// Sends a message in a chat room.
    pub async fn send_chat_message(
        &mut self,
        room_id: RoomId<'_>,
        message: impl Display,
    ) -> Result<()> {
        self.send_chat_prefixed(room_id, ' ', message).await
    }

    /// Sends a command in a chat room.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use futures::prelude::*;
    /// use showdown::message::{Kind, QueryResponse};
    /// use showdown::{connect, Result, RoomId};
    ///
    /// #[tokio::main]
    /// async fn main() -> Result<()> {
    ///     let (mut sender, mut receiver) = connect("showdown").await?;
    ///     sender.send_global_command("join lobby").await?;
    ///     sender.send_chat_command(RoomId::LOBBY, "roomdesc").await;
    ///     loop {
    ///         if let Kind::Html(html) = receiver.receive().await?.kind() {
    ///             assert!(html.contains("Relax here amidst the chaos."));
    ///             return Ok(());
    ///         }
    ///     }
    /// }
    /// ```
    pub async fn send_chat_command(
        &mut self,
        room_id: RoomId<'_>,
        command: impl Display,
    ) -> Result<()> {
        self.send_chat_prefixed(room_id, '/', command).await
    }

    pub async fn broadcast_command(
        &mut self,
        room_id: RoomId<'_>,
        command: impl Display,
    ) -> Result<()> {
        self.send_chat_prefixed(room_id, '!', command).await
    }

    async fn send_chat_prefixed(
        &mut self,
        room_id: RoomId<'_>,
        prefix: char,
        message: impl Display,
    ) -> Result<()> {
        self.send(format!("{}|{}{}", room_id.0, prefix, message))
            .await
    }

    async fn send(&mut self, message: String) -> Result<()> {
        Error::from_ws(self.sink.send(OwnedMessage::Text(message)).await)
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
        let Server { host, port } = surf::get(&format!(
            "https://pokemonshowdown.com/servers/{}.json",
            name
        ))
        .recv_json()
        .await
        .map_err(|e| Error(ErrorInner::Surf(e)))?;
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
    Surf(surf::Exception),
    Url(url::ParseError),
    Json(serde_json::Error),
    UnrecognizedMessage(Option<OwnedMessage>),
}

impl Display for Error {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match &self.0 {
            ErrorInner::WebSocket(e) => e.fmt(f),
            ErrorInner::Surf(e) => e.fmt(f),
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
            ErrorInner::Surf(e) => Some(&**e),
            ErrorInner::Url(e) => Some(e),
            ErrorInner::Json(e) => Some(e),
            ErrorInner::UnrecognizedMessage(_) => None,
        }
    }
}
