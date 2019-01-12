pub mod message;

use self::message::Message;
use reqwest::Client;
use serde_derive::Deserialize;
use std::error::Error as StdError;
use std::fmt::{self, Display, Formatter};
use std::result::Result as StdResult;
use websocket::stream::sync::NetworkStream;
pub use websocket::url;
use websocket::url::Url;
use websocket::{ClientBuilder, OwnedMessage, WebSocketError};

pub struct Showdown {
    connection: websocket::sync::Client<Box<dyn NetworkStream + Send>>,
}

impl Showdown {
    pub fn connect(name: &str) -> Result<Self> {
        Self::connect_to_url(&Self::fetch_server_url(name)?)
    }

    pub fn connect_to_url(url: &Url) -> Result<Self> {
        let connection = Error::from_ws(ClientBuilder::from_url(url).connect(None))?;
        Ok(Showdown { connection })
    }

    pub fn fetch_server_url(name: &str) -> Result<Url> {
        let Server { host, port } = Client::new()
            .get(&format!(
                "https://pokemonshowdown.com/servers/{}.json",
                name
            ))
            .send()
            .and_then(|mut r| r.json())
            .map_err(|e| Error(ErrorInner::Reqwest(e)))?;

        let protocol = if port == 443 { "wss" } else { "ws" };
        // Concatenation is fine, as it's also done by the official Showdown client
        Url::parse(&format!(
            "{}://{}:{}/showdown/websocket",
            protocol, host, port
        ))
        .map_err(|e| Error(ErrorInner::Url(e)))
    }

    pub fn receive(&mut self) -> Result<Message> {
        let message = Error::from_ws(self.connection.recv_message())?;
        if let OwnedMessage::Text(message) = message {
            Ok(Message { message })
        } else {
            Err(Error(ErrorInner::UnrecognizedMessage(message)))
        }
    }

    pub fn send_global_command(&mut self, command: &str) -> Result<()> {
        Error::from_ws(
            self.connection
                .send_message(&OwnedMessage::Text(format!("|/{}", command))),
        )
    }
}

impl fmt::Debug for Showdown {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        f.debug_struct("Showdown").finish()
    }
}

#[derive(Deserialize)]
struct Server {
    host: String,
    port: u16,
}

#[derive(Debug, PartialEq, Eq)]
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
    UnrecognizedMessage(OwnedMessage),
}

impl Display for Error {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        match &self.0 {
            ErrorInner::WebSocket(e) => e.fmt(f),
            ErrorInner::Reqwest(e) => e.fmt(f),
            ErrorInner::Url(e) => e.fmt(f),
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
            ErrorInner::UnrecognizedMessage(_) => None,
        }
    }
}
