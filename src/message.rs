use crate::{Error, ErrorInner, RoomId, Sender};
use chrono::NaiveDateTime;
use futures::future::Either;
use reqwest::r#async::Client;
use serde_derive::Deserialize;
use std::borrow::Cow;
use std::str;
use tokio::prelude::*;

#[derive(Debug)]
pub struct Message {
    pub(super) text: String,
}

/// Owned message type
///
/// To do something useful, it's usually necessary to call its
/// [`Message::parse`] method.
impl Message {
    /// Parses an owned message
    ///
    /// This function needs to exist due to self-borrow involved.
    pub fn parse(&self) -> ParsedMessage<'_> {
        let full_message: &str = &self.text;
        let (room, message) = if full_message.starts_with('>') {
            let without_prefix = &full_message[1..];
            let index = without_prefix
                .find('\n')
                .unwrap_or_else(|| without_prefix.len());
            (
                &without_prefix[..index],
                without_prefix.get(index + 1..).unwrap_or(""),
            )
        } else {
            ("", full_message)
        };
        let kind = if message.starts_with('|') {
            let (command, arg) = split2(&message[1..]);
            Kind::parse(command, arg)
                .unwrap_or_else(|| Kind::Unrecognized(UnrecognizedMessage(full_message)))
        } else {
            Kind::Unrecognized(UnrecognizedMessage(full_message))
        };
        ParsedMessage {
            room_id: RoomId(room),
            kind,
        }
    }
}

fn split2(arg: &str) -> (&str, &str) {
    match arg.find('|') {
        Some(index) => (&arg[..index], &arg[index + 1..]),
        None => (arg, ""),
    }
}

/// Parsed message.
#[derive(Debug)]
pub struct ParsedMessage<'a> {
    /// Room where a message was said.
    pub room_id: RoomId<'a>,
    /// Message kind.
    pub kind: Kind<'a>,
}

#[derive(Debug)]
pub enum Kind<'a> {
    Chat(Chat<'a>),
    Challenge(Challenge<'a>),
    Html(&'a str),
    NoInit(NoInit<'a>),
    RoomInit(RoomInit<'a>),
    QueryResponse(QueryResponse<'a>),
    UpdateUser(UpdateUser<'a>),
    Unrecognized(UnrecognizedMessage<'a>),
}

impl Kind<'_> {
    fn parse<'a>(command: &str, arguments: &'a str) -> Option<Kind<'a>> {
        Some(match command {
            "c:" => Kind::Chat(Chat::parse(arguments)?),
            "challstr" => Kind::Challenge(Challenge(arguments)),
            "html" => Kind::Html(arguments),
            "init" => Kind::RoomInit(RoomInit::parse(arguments)?),
            "noinit" => Kind::NoInit(NoInit::parse(arguments)?),
            "queryresponse" => Kind::QueryResponse(QueryResponse::parse(arguments)?),
            "updateuser" => Kind::UpdateUser(UpdateUser::parse(arguments)?),
            _ => return None,
        })
    }
}

#[derive(Copy, Clone, Debug)]
pub struct Chat<'a> {
    pub timestamp: NaiveDateTime,
    pub user: &'a str,
    pub message: &'a str,
}

impl<'a> Chat<'a> {
    fn parse(arguments: &'a str) -> Option<Self> {
        let (timestamp, arguments) = split2(arguments);
        let timestamp = NaiveDateTime::from_timestamp(timestamp.parse().ok()?, 0);
        let (user, mut message) = split2(arguments);
        if message.ends_with('\n') {
            message = &message[..message.len() - 1];
        }
        Some(Self {
            timestamp,
            user,
            message,
        })
    }
}

/// Login challenge.
#[derive(Copy, Clone, Debug)]
pub struct Challenge<'a>(&'a str);

impl<'a> Challenge<'a> {
    /// Logs in an user.
    ///
    /// # Examples
    ///
    /// ```
    /// #![feature(async_await, await_macro, futures_api)]
    /// #![recursion_limit = "128"]
    ///
    /// use futures03::prelude::*;
    /// use rand::prelude::*;
    /// use showdown::message::{Kind, NoInit, NoInitKind};
    /// use showdown::{connect, Result, RoomId};
    /// use tokio::await;
    /// use tokio::runtime::Runtime;
    ///
    /// async fn start() -> Result<()> {
    ///     let (mut sender, mut receiver) = await!(connect("showdown"))?;
    ///     await!(sender.send_global_command("join bot dev"))?;
    ///     let mut received;
    ///     // Get the challenge first
    ///     let challenge = loop {
    ///         received = await!(receiver.receive())?;
    ///         if let Kind::Challenge(ch) = received.parse().kind {
    ///             break ch;
    ///         }
    ///     };
    ///     // It's not possible to join a hidden room without being logged in.
    ///     loop {
    ///         if let Kind::NoInit(NoInit {
    ///             kind: NoInitKind::NameRequired,
    ///             ..
    ///         }) = await!(receiver.receive())?.parse().kind
    ///         {
    ///             break;
    ///         }
    ///     }
    ///     let name = random_username();
    ///     await!(challenge.login(&mut sender, &name))?;
    ///     await!(sender.send_global_command("join bot dev"))?;
    ///     loop {
    ///         if let Kind::RoomInit(_) = await!(receiver.receive())?.parse().kind {
    ///             return Ok(());
    ///         }
    ///     }
    /// }
    ///
    /// fn random_username() -> String {
    ///     let chars: Vec<_> = (b'a'..b'z').chain(b'0'..b'9').map(char::from).collect();
    ///     (0..16)
    ///         .map(|_| chars.choose(&mut thread_rng()).unwrap())
    ///         .collect()
    /// }
    ///
    /// Runtime::new()
    ///     .unwrap()
    ///     .block_on_all(start().boxed().compat())
    ///     .unwrap();
    /// ```
    pub fn login(
        self,
        sender: &'a mut Sender,
        login: &'a str,
    ) -> impl Future<Item = Option<PasswordRequired<'a>>, Error = Error> + 'a {
        let client = Client::new();
        let request = client
            .post("http://play.pokemonshowdown.com/action.php")
            .form(&[
                ("act", "getassertion"),
                ("userid", login),
                ("challstr", self.0),
            ]);
        request
            .send()
            .and_then(|r| r.into_body().concat2())
            .then(Error::from_reqwest)
            .and_then(move |response| {
                let response = match str::from_utf8(&response) {
                    Err(e) => return Either::A(future::result(Err(Error(ErrorInner::Utf8(e))))),
                    Ok(response) => response,
                };
                if response == ";" {
                    Either::A(future::result(Ok(Some(PasswordRequired {
                        challstr: self,
                        login,
                        sender,
                        client,
                    }))))
                } else {
                    Either::B(
                        sender
                            .send_global_command(&format!("trn {},0,{}", login, response))
                            .map(|()| None),
                    )
                }
            })
    }

    pub fn login_with_password(
        self,
        sender: &'a mut Sender,
        login: &'a str,
        password: &str,
    ) -> impl Future<Item = (), Error = Error> + 'a {
        self.login_with_password_and_client(sender, login, password, &Client::new())
    }

    fn login_with_password_and_client(
        self,
        sender: &'a mut Sender,
        login: &'a str,
        password: &str,
        client: &Client,
    ) -> impl Future<Item = (), Error = Error> + 'a {
        if password.is_empty() {
            return Either::A(self.login(sender, login).map(|_| ()));
        }
        let future = client
            .post("http://play.pokemonshowdown.com/action.php")
            .form(&[
                ("act", "login"),
                ("name", login),
                ("pass", password),
                ("challstr", self.0),
            ])
            .send()
            .and_then(|r| r.into_body().concat2())
            .then(Error::from_reqwest)
            .and_then(move |response| {
                let LoginServerResponse { assertion } = serde_json::from_slice(&response[1..])
                    .map_err(|e| Error(ErrorInner::Json(e)))?;
                Ok(sender.send_global_command(&format!("trn {},0,{}", login, assertion)))
            })
            .and_then(|send| send);
        Either::B(future)
    }
}

pub struct PasswordRequired<'a> {
    challstr: Challenge<'a>,
    login: &'a str,
    sender: &'a mut Sender,
    client: Client,
}

impl<'a> PasswordRequired<'a> {
    pub fn login_with_password(
        &'a mut self,
        password: &str,
    ) -> impl Future<Item = (), Error = Error> + 'a {
        self.challstr.login_with_password_and_client(
            self.sender,
            self.login,
            password,
            &self.client,
        )
    }
}

#[derive(Deserialize)]
struct LoginServerResponse<'a> {
    #[serde(borrow)]
    assertion: Cow<'a, str>,
}

#[derive(Debug)]
pub struct RoomInit<'a> {
    room_type: RoomType,
    title: &'a str,
    users: &'a str,
}

impl RoomInit<'_> {
    fn parse(arguments: &str) -> Option<RoomInit<'_>> {
        let mut lines = arguments.split('\n');
        let room_type = match lines.next()? {
            "chat" => RoomType::Chat,
            "battle" => RoomType::Battle,
            _ => return None,
        };
        let mut title = None;
        let mut users = None;
        for line in lines {
            if line.is_empty() {
                continue;
            }
            if !line.starts_with('|') {
                return None;
            }
            let (command, arguments) = split2(&line[1..]);
            match command {
                "title" => title = Some(arguments),
                "users" => users = Some(arguments),
                _ => {}
            }
        }
        Some(RoomInit {
            room_type,
            title: title?,
            users: users?,
        })
    }
}

#[derive(Copy, Clone, Debug)]
pub struct NoInit<'a> {
    pub kind: NoInitKind,
    pub reason: &'a str,
}

impl<'a> NoInit<'a> {
    fn parse(arguments: &'a str) -> Option<Self> {
        let (kind, reason) = split2(arguments);
        Some(Self {
            kind: NoInitKind::parse(kind)?,
            reason,
        })
    }
}

#[derive(Copy, Clone, Debug)]
pub enum NoInitKind {
    Nonexistent,
    JoinFailed,
    NameRequired,
}

impl NoInitKind {
    fn parse(argument: &str) -> Option<Self> {
        Some(match argument {
            "nonexistent" => NoInitKind::Nonexistent,
            "joinfailed" => NoInitKind::JoinFailed,
            "namerequired" => NoInitKind::NameRequired,
            _ => return None,
        })
    }
}

#[derive(Copy, Clone, Debug)]
pub enum RoomType {
    Chat,
    Battle,
}

#[derive(Debug)]
pub enum QueryResponse<'a> {
    Rooms(RoomsList<'a>),
}

impl QueryResponse<'_> {
    fn parse(arguments: &str) -> Option<QueryResponse<'_>> {
        let (command, arguments) = split2(arguments);
        match command {
            "rooms" => Some(QueryResponse::Rooms(RoomsList::parse(arguments)?)),
            _ => None,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoomsList<'a> {
    #[serde(borrow)]
    pub official: Vec<Room<'a>>,
    #[serde(borrow)]
    pub pspl: Vec<Room<'a>>,
    #[serde(borrow)]
    pub chat: Vec<Room<'a>>,
    pub user_count: u32,
    pub battle_count: u32,
}

impl<'a> RoomsList<'a> {
    fn parse(arguments: &'a str) -> Option<Self> {
        serde_json::from_str(arguments).ok()
    }

    pub fn iter(&self) -> impl Iterator<Item = &Room<'a>> {
        self.official.iter().chain(&self.pspl).chain(&self.chat)
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Room<'a> {
    #[serde(borrow)]
    pub title: Cow<'a, str>,
    #[serde(borrow)]
    pub desc: Cow<'a, str>,
    pub user_count: u32,
    #[serde(borrow, default)]
    pub sub_rooms: Vec<Cow<'a, str>>,
}

#[derive(Copy, Clone, Debug)]
pub struct UpdateUser<'a> {
    pub username: &'a str,
    pub named: bool,
    pub avatar: &'a str,
}

impl<'a> UpdateUser<'a> {
    fn parse(arguments: &'a str) -> Option<Self> {
        let mut parts = arguments.split('|');
        let username = parts.next()?;
        let named = match parts.next()? {
            "0" => false,
            "1" => true,
            _ => return None,
        };
        let mut avatar = parts.next()?;
        if let Some(index) = avatar.find('\n') {
            avatar = &avatar[..index];
        }
        Some(Self {
            username,
            named,
            avatar,
        })
    }
}

#[derive(Debug)]
pub struct UnrecognizedMessage<'a>(&'a str);
