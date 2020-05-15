use crate::{Error, ErrorInner, Result, RoomId, Sender};
use chrono::NaiveDateTime;
use futures::TryFutureExt;
use serde_derive::Deserialize;
use std::borrow::Cow;
use std::fmt::Display;
use std::str;

/// Owned message type
#[derive(Debug)]
pub struct Message {
    pub(crate) raw: String,
}

impl Message {
    pub fn kind(&self) -> Kind<'_> {
        let full_message: &str = &self.raw;
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
        if message.starts_with('|') {
            let (command, arg) = split2(&message[1..]);
            Kind::parse(RoomId(room), command, arg)
                .unwrap_or_else(|| Kind::Unrecognized(UnrecognizedMessage(full_message)))
        } else {
            Kind::Unrecognized(UnrecognizedMessage(full_message))
        }
    }
}

fn split2(arg: &str) -> (&str, &str) {
    match arg.find('|') {
        Some(index) => (&arg[..index], &arg[index + 1..]),
        None => (arg, ""),
    }
}

#[derive(Debug)]
#[non_exhaustive]
pub enum Kind<'a> {
    Text(Text<'a>),
    Challenge(Challenge<'a>),
    Html(&'a str),
    NoInit(NoInit<'a>),
    RoomInit(RoomInit<'a>),
    QueryResponse(QueryResponse<'a>),
    UpdateUser(UpdateUser<'a>),
    Unrecognized(UnrecognizedMessage<'a>),
}

impl Kind<'_> {
    fn parse<'a>(room_id: RoomId<'a>, command: &str, arguments: &'a str) -> Option<Kind<'a>> {
        Some(match command {
            "c:" => Kind::Text(Text::Chat(Chat::parse(room_id, arguments))),
            "pm" => Kind::Text(Text::Private(Private::parse(arguments))),
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
pub enum Text<'a> {
    Chat(Chat<'a>),
    Private(Private<'a>),
}

impl<'a> Text<'a> {
    pub async fn reply(&self, sender: &mut Sender, message: impl Display) -> Result<()> {
        match self {
            Text::Chat(chat) => sender.send_chat_message(chat.room_id(), message).await,
            Text::Private(private) => {
                sender
                    .send_global_command(format_args!("pm {},{}", private.from, message))
                    .await
            }
        }
    }

    pub fn message(&self) -> &'a str {
        match self {
            Text::Chat(chat) => chat.message(),
            Text::Private(private) => private.message,
        }
    }

    pub fn user(&self) -> &'a str {
        match self {
            Text::Chat(chat) => chat.user(),
            Text::Private(private) => private.from,
        }
    }
}

#[derive(Copy, Clone, Debug)]
pub struct Chat<'a> {
    room_id: RoomId<'a>,
    timestamp: &'a str,
    user: &'a str,
    message: &'a str,
}

impl<'a> Chat<'a> {
    fn parse(room_id: RoomId<'a>, arguments: &'a str) -> Self {
        let (timestamp, arguments) = split2(arguments);
        let (user, message) = split2(arguments);
        Self {
            room_id,
            timestamp,
            user,
            message,
        }
    }

    pub fn timestamp(&self) -> NaiveDateTime {
        NaiveDateTime::from_timestamp(self.timestamp.parse().unwrap(), 0)
    }

    pub fn user(&self) -> &'a str {
        self.user
    }

    pub fn message(&self) -> &'a str {
        let message = self.message;
        if message.ends_with('\n') {
            &message[..message.len() - 1]
        } else {
            message
        }
    }

    pub fn room_id(&self) -> RoomId<'a> {
        self.room_id
    }
}

#[derive(Copy, Clone, Debug)]
pub struct Private<'a> {
    pub from: &'a str,
    pub to: &'a str,
    pub message: &'a str,
}

impl<'a> Private<'a> {
    fn parse(arguments: &'a str) -> Self {
        let (from, arguments) = split2(arguments);
        let (to, message) = split2(arguments);
        Self { from, to, message }
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
    /// use futures::prelude::*;
    /// use rand::prelude::*;
    /// use showdown::message::{Kind, NoInit, NoInitKind};
    /// use showdown::{connect, Result, RoomId};
    /// use tokio::runtime::Runtime;
    ///
    /// async fn start() -> Result<()> {
    ///     let (mut sender, mut receiver) = connect("showdown").await?;
    ///     sender.send_global_command("join bot dev").await?;
    ///     let mut received;
    ///     // Get the challenge first
    ///     let challenge = loop {
    ///         received = receiver.receive().await?;
    ///         if let Kind::Challenge(ch) = received.kind() {
    ///             break ch;
    ///         }
    ///     };
    ///     // It's not possible to join a hidden room without being logged in.
    ///     loop {
    ///         if let Kind::NoInit(NoInit {
    ///             kind: NoInitKind::NameRequired,
    ///             ..
    ///         }) = receiver.receive().await?.kind()
    ///         {
    ///             break;
    ///         }
    ///     }
    ///     let name = random_username();
    ///     challenge.login(&mut sender, &name).await?;
    ///     sender.send_global_command("join bot dev").await?;
    ///     loop {
    ///         if let Kind::RoomInit(_) = receiver.receive().await?.kind() {
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
    /// Runtime::new().unwrap().block_on(start()).unwrap();
    /// ```
    pub async fn login(
        self,
        sender: &'a mut Sender,
        login: &'a str,
    ) -> Result<Option<PasswordRequired<'a>>> {
        let response = surf::post("http://play.pokemonshowdown.com/action.php")
            .body_form(&[
                ("act", "getassertion"),
                ("userid", login),
                ("challstr", self.0),
            ])
            .expect("serializable authentication form")
            .recv_string()
            .map_err(|e| Error(ErrorInner::Surf(e)))
            .await?;
        if response == ";" {
            Ok(Some(PasswordRequired {
                challstr: self,
                login,
                sender,
            }))
        } else {
            sender
                .send_global_command(format_args!("trn {},0,{}", login, response))
                .await
                .map(|()| None)
        }
    }

    pub async fn login_with_password(
        self,
        sender: &mut Sender,
        login: &str,
        password: &str,
    ) -> Result<()> {
        if password.is_empty() {
            return self.login(sender, login).await.map(|_| ());
        }
        let response = surf::post("http://play.pokemonshowdown.com/action.php")
            .body_form(&[
                ("act", "login"),
                ("name", login),
                ("pass", password),
                ("challstr", self.0),
            ])
            .expect("serializable authentication form")
            .recv_bytes()
            .await
            .map_err(|e| Error(ErrorInner::Surf(e)))?;
        let LoginServerResponse { assertion } =
            serde_json::from_slice(&response[1..]).map_err(|e| Error(ErrorInner::Json(e)))?;
        sender
            .send_global_command(format_args!("trn {},0,{}", login, assertion))
            .await
    }
}

pub struct PasswordRequired<'a> {
    challstr: Challenge<'a>,
    login: &'a str,
    sender: &'a mut Sender,
}

impl PasswordRequired<'_> {
    pub async fn login_with_password(&mut self, password: &str) -> Result<()> {
        self.challstr
            .login_with_password(self.sender, self.login, password)
            .await
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
#[non_exhaustive]
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
