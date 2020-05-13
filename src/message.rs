mod parsed_message;

use self::parsed_message::ParsedMessage;
use self::rental_internal::RentalMessage;
use crate::{Error, ErrorInner, Result, RoomId, Sender};
use chrono::NaiveDateTime;
use futures::TryFutureExt;
use reqwest::Client;
use serde_derive::Deserialize;
use std::borrow::Cow;
use std::str;

/// Owned message type
#[derive(Debug)]
pub struct Message {
    rental: RentalMessage,
}

impl Message {
    pub(crate) fn new(message: String) -> Self {
        Self {
            rental: RentalMessage::new(message, |message| ParsedMessage::parse_message(message)),
        }
    }

    pub fn room_id(&self) -> RoomId<'_> {
        self.rental.suffix().room_id
    }

    pub fn kind(&self) -> &Kind<'_> {
        &self.rental.suffix().kind
    }
}

rental! {
    mod rental_internal {
        #[rental(covariant, debug)]
        pub struct RentalMessage {
            message: String,
            parsed: super::ParsedMessage<'message>,
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
            "c:" => Kind::Chat(Chat::parse(arguments)),
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
    timestamp: &'a str,
    user: &'a str,
    message: &'a str,
}

impl<'a> Chat<'a> {
    fn parse(arguments: &'a str) -> Self {
        let (timestamp, arguments) = split2(arguments);
        let (user, message) = split2(arguments);
        Self {
            timestamp,
            user,
            message,
        }
    }

    pub fn timestamp(&self) -> NaiveDateTime {
        NaiveDateTime::from_timestamp(self.timestamp.parse().unwrap(), 0)
    }

    pub fn user(&self) -> &str {
        self.user
    }

    pub fn message(&self) -> &str {
        let message = self.message;
        if message.ends_with('\n') {
            &message[..message.len() - 1]
        } else {
            message
        }
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
        let client = Client::new();
        let response = client
            .post("http://play.pokemonshowdown.com/action.php")
            .form(&[
                ("act", "getassertion"),
                ("userid", login),
                ("challstr", self.0),
            ])
            .send()
            .and_then(|r| r.text())
            .await;
        let response = Error::from_reqwest(response)?;
        if response == ";" {
            Ok(Some(PasswordRequired {
                challstr: self,
                login,
                sender,
                client,
            }))
        } else {
            sender
                .send_global_command(&format!("trn {},0,{}", login, response))
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
        self.login_with_password_and_client(sender, login, password, &Client::new())
            .await
    }

    async fn login_with_password_and_client(
        self,
        sender: &mut Sender,
        login: &str,
        password: &str,
        client: &Client,
    ) -> Result<()> {
        if password.is_empty() {
            return self.login(sender, login).await.map(|_| ());
        }
        let response = client
            .post("http://play.pokemonshowdown.com/action.php")
            .form(&[
                ("act", "login"),
                ("name", login),
                ("pass", password),
                ("challstr", self.0),
            ])
            .send()
            .and_then(|r| r.bytes())
            .await;
        let response = Error::from_reqwest(response)?;
        let LoginServerResponse { assertion } =
            serde_json::from_slice(&response[1..]).map_err(|e| Error(ErrorInner::Json(e)))?;
        sender
            .send_global_command(&format!("trn {},0,{}", login, assertion))
            .await
    }
}

pub struct PasswordRequired<'a> {
    challstr: Challenge<'a>,
    login: &'a str,
    sender: &'a mut Sender,
    client: Client,
}

impl PasswordRequired<'_> {
    pub async fn login_with_password(&mut self, password: &str) -> Result<()> {
        self.challstr
            .login_with_password_and_client(self.sender, self.login, password, &self.client)
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
