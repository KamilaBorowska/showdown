//! Pokémon Showdown client.
//!
//! # Stability
//!
//! This crate is not stable, not even close. It requires nightly, and uses
//! features that are very likely to change. Additionally, the APIs of this
//! crate are heavily experimented on, and there isn't going to be
//! depreciation period for removed features. Don't use this crate if you
//! aren't prepared for constant breakage.

pub mod message;

use self::message::Message;
pub use chrono;
use futures::sync::mpsc;
use reqwest::r#async::Client;
use serde_derive::Deserialize;
use std::error::Error as StdError;
use std::fmt::{self, Display, Formatter};
use std::result::Result as StdResult;
use std::str::Utf8Error;
use std::time::{Duration, Instant};
use tokio::prelude::stream::{SplitSink, SplitStream};
use tokio::prelude::*;
use tokio::timer::{self, Delay};
use websocket::r#async;
pub use websocket::url;
use websocket::url::Url;
use websocket::{ClientBuilder, OwnedMessage, WebSocketError};

/// Message receiver.
///
/// # Examples
///
/// ```
/// #![feature(async_await, await_macro, futures_api)]
/// #![recursion_limit = "128"]
///
/// use futures03::prelude::*;
/// use showdown::message::{Kind, ParsedMessage, UpdateUser};
/// use showdown::{connect, Result, RoomId};
/// use tokio::await;
/// use tokio::runtime::Runtime;
///
/// async fn start() -> Result<()> {
///     let (_, mut receiver) = await!(connect("showdown"))?;
///     let message = await!(receiver.receive())?;
///     match message.parse() {
///         ParsedMessage {
///             room_id: RoomId(""),
///             kind:
///                 Kind::UpdateUser(UpdateUser {
///                     username,
///                     named: false,
///                     ..
///                 }),
///         } => {
///             assert!(username.starts_with("Guest "));
///         }
///         _ => panic!(),
///     }
///     Ok(())
/// }
///
/// Runtime::new()
///     .unwrap()
///     .block_on_all(start().boxed().compat())
///     .unwrap();
/// ```
pub struct Receiver {
    stream: SplitStream<r#async::Client<Box<dyn r#async::Stream + Send>>>,
}

impl fmt::Debug for Receiver {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("Receiver").finish()
    }
}

/// Message sender.
#[derive(Debug)]
pub struct Sender {
    sender: mpsc::UnboundedSender<OwnedMessage>,
}

impl Sender {
    fn new(sink: SplitSink<r#async::Client<Box<dyn r#async::Stream + Send>>>) -> Sender {
        let (sender, receiver) = mpsc::unbounded();
        tokio::spawn(
            receiver
                .fold(sink, |sink, m| {
                    sink.send(m)
                        .then(Error::from_ws)
                        .and_then(|sink| {
                            Delay::new(Instant::now() + Duration::from_millis(600))
                                .map(|_| sink)
                                .map_err(|e| Error(ErrorInner::Timer(e)))
                        })
                        .then(|r| Ok(r.unwrap()))
                })
                .map(|_| ()),
        );
        Self { sender }
    }

    /// Sends a global command.
    ///
    /// # Example
    ///
    /// ```
    /// #![feature(async_await, await_macro, futures_api)]
    /// #![recursion_limit = "128"]
    ///
    /// use futures03::prelude::*;
    /// use showdown::message::{Kind, ParsedMessage, QueryResponse};
    /// use showdown::{connect, Result, RoomId};
    /// use tokio::await;
    /// use tokio::runtime::Runtime;
    ///
    /// async fn start() -> Result<()> {
    ///     let (mut sender, mut receiver) = await!(connect("showdown"))?;
    ///     await!(sender.send_global_command("cmd rooms"))?;
    ///     loop {
    ///         let received = await!(receiver.receive())?;
    ///         if let Kind::QueryResponse(QueryResponse::Rooms(rooms)) = received.parse().kind {
    ///             assert!(rooms
    ///                 .official
    ///                 .iter()
    ///                 .any(|room| room.title == "Tournaments"));
    ///             return Ok(());
    ///         }
    ///     }
    /// }
    ///
    /// Runtime::new()
    ///     .unwrap()
    ///     .block_on_all(start().boxed().compat())
    ///     .unwrap();
    /// ```
    pub fn send_global_command(
        &mut self,
        command: &str,
    ) -> impl Future<Item = (), Error = Error> + '_ {
        self.send(format!("|/{}", command))
    }

    /// Sends a message in a chat room.
    ///
    /// # Examples
    ///
    /// ```
    /// #![feature(async_await, await_macro, futures_api)]
    /// #![recursion_limit = "128"]
    ///
    /// use futures03::prelude::*;
    /// use showdown::message::{Kind, ParsedMessage, QueryResponse};
    /// use showdown::{connect, Result, RoomId};
    /// use tokio::await;
    /// use tokio::runtime::Runtime;
    ///
    /// async fn start() -> Result<()> {
    ///     let (mut sender, mut receiver) = await!(connect("showdown"))?;
    ///     await!(sender.send_global_command("join lobby"))?;
    ///     await!(sender.send_chat_message(RoomId::LOBBY, "/roomdesc"));
    ///     loop {
    ///         if let Kind::Html(html) = await!(receiver.receive())?.parse().kind {
    ///             assert!(html.contains("Relax here amidst the chaos."));
    ///             return Ok(());
    ///         }
    ///     }
    /// }
    ///
    /// Runtime::new()
    ///     .unwrap()
    ///     .block_on_all(start().boxed().compat())
    ///     .unwrap();
    /// ```
    pub fn send_chat_message(
        &mut self,
        room_id: RoomId<'_>,
        message: &str,
    ) -> impl Future<Item = (), Error = Error> + '_ {
        self.send(format!("{}|{}", room_id.0, message))
    }

    fn send(&mut self, message: String) -> impl Future<Item = (), Error = Error> + '_ {
        (&mut self.sender)
            .send(OwnedMessage::Text(message))
            .map(|_| ())
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
/// #![feature(async_await, await_macro, futures_api)]
/// #![recursion_limit = "128"]
///
/// use futures03::prelude::*;
/// use showdown::{connect, Result};
/// use tokio::await;
/// use tokio::runtime::Runtime;
///
/// async fn start() {
///     assert!(await!(connect("showdown")).is_ok());
///     assert!(await!(connect("fakestofservers")).is_err());
/// }
///
/// Runtime::new()
///     .unwrap()
///     .block_on_all(start().unit_error().boxed().compat())
///     .unwrap();
/// ```
pub fn connect(name: &str) -> impl Future<Item = (Sender, Receiver), Error = Error> {
    fetch_server_url(name).and_then(|url| connect_to_url(&url))
}

/// Connects to an URL.
///
/// This URL is provided by [`fetch_server_url`] function.
///
/// # Examples
///
/// ```rust
/// #![feature(async_await, await_macro, futures_api)]
/// #![recursion_limit = "128"]
///
/// use futures03::prelude::*;
/// use showdown::{connect_to_url, fetch_server_url, Result};
/// use tokio::await;
/// use tokio::runtime::Runtime;
///
/// async fn start() -> Result<()> {
///     let url = await!(fetch_server_url("showdown"))?;
///     assert_eq!(url.as_str(), "ws://sim2.psim.us:8000/showdown/websocket");
///     await!(connect_to_url(&url))?;
///     Ok(())
/// }
///
/// Runtime::new()
///     .unwrap()
///     .block_on_all(start().boxed().compat())
///     .unwrap();
/// ```
pub fn connect_to_url(url: &Url) -> impl Future<Item = (Sender, Receiver), Error = Error> {
    ClientBuilder::from_url(url).async_connect(None).then(|r| {
        let (sink, stream) = Error::from_ws(r)?.0.split();
        Ok((Sender::new(sink), Receiver { stream }))
    })
}

pub fn fetch_server_url(name: &str) -> impl Future<Item = Url, Error = Error> {
    Client::new()
        .get(&format!(
            "https://pokemonshowdown.com/servers/{}.json",
            name
        ))
        .send()
        .and_then(|mut r| r.json())
        .then(|result| {
            let Server { host, port } = Error::from_reqwest(result)?;
            let protocol = if port == 443 { "wss" } else { "ws" };
            // Concatenation is fine, as it's also done by the official Showdown client
            Url::parse(&format!(
                "{}://{}:{}/showdown/websocket",
                protocol, host, port
            ))
            .map_err(|e| Error(ErrorInner::Url(e)))
        })
}

impl Receiver {
    pub fn receive(&mut self) -> impl Future<Item = Message, Error = Error> + '_ {
        (&mut self.stream)
            .into_future()
            .then(|e| Error::from_ws(e.map_err(|e| e.0)))
            .and_then(|(message, _)| {
                if let Some(OwnedMessage::Text(text)) = message {
                    Ok(Message { text })
                } else {
                    Err(Error(ErrorInner::UnrecognizedMessage(message)))
                }
            })
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
    fn from_ws<T>(r: StdResult<T, WebSocketError>) -> Result<T> {
        r.map_err(|e| Error(ErrorInner::WebSocket(e)))
    }

    fn from_reqwest<T>(r: StdResult<T, reqwest::Error>) -> Result<T> {
        r.map_err(|e| Error(ErrorInner::Reqwest(e)))
    }
}

#[derive(Debug)]
enum ErrorInner {
    WebSocket(WebSocketError),
    Reqwest(reqwest::Error),
    Url(url::ParseError),
    Mpsc(mpsc::SendError<OwnedMessage>),
    Utf8(Utf8Error),
    Json(serde_json::Error),
    Timer(timer::Error),
    UnrecognizedMessage(Option<OwnedMessage>),
}

impl Display for Error {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match &self.0 {
            ErrorInner::WebSocket(e) => e.fmt(f),
            ErrorInner::Reqwest(e) => e.fmt(f),
            ErrorInner::Url(e) => e.fmt(f),
            ErrorInner::Mpsc(e) => e.fmt(f),
            ErrorInner::Utf8(e) => e.fmt(f),
            ErrorInner::Json(e) => e.fmt(f),
            ErrorInner::Timer(e) => e.fmt(f),
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
            ErrorInner::Utf8(e) => Some(e),
            ErrorInner::Json(e) => Some(e),
            ErrorInner::Timer(e) => Some(e),
            ErrorInner::UnrecognizedMessage(_) => None,
        }
    }
}
