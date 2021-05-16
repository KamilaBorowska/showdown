use crate::{Error, ErrorInner, Result, RoomId, SendMessage, Stream};
#[cfg(feature = "chrono")]
use chrono::{offset::Utc, DateTime, NaiveDateTime};
use futures_util::future::TryFutureExt;
use futures_util::sink::SinkExt;
use reqwest::Client;
use serde::Deserialize;
use std::borrow::Cow;
use std::fmt::Debug;
use std::iter::FusedIterator;
use std::str;

/// Owned message type
#[derive(Debug)]
pub struct Message {
    pub(crate) raw: String,
}

impl Message {
    fn parts(&self) -> (&str, &str) {
        let full_message: &str = &self.raw;
        if let Some(without_prefix) = full_message.strip_prefix('>') {
            let mut parts = without_prefix.splitn(2, '\n');
            (parts.next().unwrap(), parts.next().unwrap_or(""))
        } else {
            (RoomId::LOBBY.0, full_message)
        }
    }

    pub fn room(&self) -> RoomId<'_> {
        RoomId(self.parts().0)
    }

    pub fn kind(&self) -> Kind<'_> {
        let message = self.parts().1;
        if let Some(message) = message.strip_prefix('|') {
            let (command, arg) = split2(message);
            Kind::parse(command, arg).unwrap_or(Kind::Unrecognized(UnrecognizedMessage(message)))
        } else {
            Kind::Unrecognized(UnrecognizedMessage(message))
        }
    }
}

fn split2(arg: &str) -> (&str, &str) {
    let mut parts = arg.splitn(2, '|');
    (parts.next().unwrap(), parts.next().unwrap_or(""))
}

#[derive(Debug)]
#[non_exhaustive]
/// Showdown message kind.
///
/// This structure was designed to be matched on. For performance
/// reasons, it's borrowing string slices from `Message`, trying
/// to use this structure when `Message` is not in scope will
/// cause borrow checker failures.
pub enum Kind<'a> {
    Chat(Chat<'a>),
    Private(Private<'a>),
    Join(&'a str),
    NicknameChange(&'a str, &'a str),
    Leave(&'a str),
    /// Login challenge.
    ///
    /// This can be used to authenticate.
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
            "pm" => Kind::Private(Private::parse(arguments)),
            "J" => Kind::Join(arguments),
            "N" => {
                let (a, b) = split2(arguments);
                Kind::NicknameChange(a, b)
            }
            "L" => Kind::Leave(arguments),
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

    #[cfg(feature = "chrono")]
    /// Provides chat message timestamp, requires chrono feature.
    pub fn timestamp(&self) -> DateTime<Utc> {
        DateTime::from_utc(
            NaiveDateTime::from_timestamp(self.timestamp.parse().unwrap(), 0),
            Utc,
        )
    }

    pub fn user(&self) -> &'a str {
        self.user
    }

    pub fn message(&self) -> &'a str {
        self.message.strip_suffix('\n').unwrap_or(&self.message)
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
    /// ```no_run
    /// use rand::prelude::*;
    /// use futures::{SinkExt, StreamExt};
    /// use showdown::message::{Kind, NoInit, NoInitKind};
    /// use showdown::{Result, RoomId, SendMessage, Stream};
    ///
    /// #[tokio::main]
    /// async fn main() -> Result<()> {
    ///     let mut stream = Stream::connect("showdown").await?;
    ///     stream.send(SendMessage::global_command("join bot dev")).await?;
    ///     let mut received;
    ///     // Get the challenge first
    ///     let challenge = loop {
    ///         received = stream.next().await.unwrap()?;
    ///         if let Kind::Challenge(ch) = received.kind() {
    ///             break ch;
    ///         }
    ///     };
    ///     // It's not possible to join a hidden room without being logged in.
    ///     loop {
    ///         if let Kind::NoInit(NoInit {
    ///             kind: NoInitKind::NameRequired,
    ///             ..
    ///         }) = stream.next().await.unwrap()?.kind()
    ///         {
    ///             break;
    ///         }
    ///     }
    ///     let name = random_username();
    ///     challenge.login(&mut stream, &name).await?;
    ///     stream.send(SendMessage::global_command("join bot dev")).await?;
    ///     while let Some(message) = stream.next().await {
    ///         if let Kind::RoomInit(_) = message?.kind() {
    ///             return Ok(());
    ///         }
    ///     }
    ///     panic!("Server did allow the bot to enter a room");
    /// }
    ///
    /// fn random_username() -> String {
    ///     let chars: Vec<_> = (b'a'..b'z').chain(b'0'..b'9').map(char::from).collect();
    ///     (0..16)
    ///         .map(|_| chars.choose(&mut thread_rng()).unwrap())
    ///         .collect()
    /// }
    /// ```
    pub async fn login(
        self,
        stream: &'a mut Stream,
        login: &'a str,
    ) -> Result<Option<PasswordRequired<'a>>> {
        let response = Client::new()
            .post("https://play.pokemonshowdown.com/action.php")
            .form(&[
                ("act", "getassertion"),
                ("userid", login),
                ("challstr", self.0),
            ])
            .send()
            .and_then(|r| r.text())
            .await
            .map_err(|e| Error(ErrorInner::Reqwest(e)))?;
        if response == ";" {
            Ok(Some(PasswordRequired {
                challstr: self,
                login,
                stream,
            }))
        } else {
            let command = SendMessage::global_command(format_args!("trn {},0,{}", login, response));
            stream.send(command).await.map(|()| None)
        }
    }

    pub async fn login_with_password(
        self,
        sender: &mut Stream,
        login: &str,
        password: &str,
    ) -> Result<()> {
        if password.is_empty() {
            return self.login(sender, login).await.map(|_| ());
        }
        let response = Client::new()
            .post("https://play.pokemonshowdown.com/action.php")
            .form(&[
                ("act", "login"),
                ("name", login),
                ("pass", password),
                ("challstr", self.0),
            ])
            .send()
            .and_then(|r| r.bytes())
            .await
            .map_err(|e| Error(ErrorInner::Reqwest(e)))?;
        let LoginServerResponse { assertion } =
            serde_json::from_slice(&response[1..]).map_err(|e| Error(ErrorInner::Json(e)))?;
        let command = SendMessage::global_command(format_args!("trn {},0,{}", login, assertion));
        sender.send(command).await
    }
}

pub struct PasswordRequired<'a> {
    challstr: Challenge<'a>,
    login: &'a str,
    stream: &'a mut Stream,
}

impl PasswordRequired<'_> {
    pub async fn login_with_password(&mut self, password: &str) -> Result<()> {
        self.challstr
            .login_with_password(self.stream, self.login, password)
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

    pub fn iter(
        &self,
    ) -> impl Iterator<Item = &Room<'a>> + Clone + Debug + DoubleEndedIterator + FusedIterator {
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
        let avatar = parts.next()?.split('\n').next().unwrap();
        Some(Self {
            username,
            named,
            avatar,
        })
    }
}

#[derive(Debug)]
pub struct UnrecognizedMessage<'a>(&'a str);

#[cfg(test)]
mod test {
    use super::{Kind, Message};

    #[test]
    fn parse_join() {
        assert!(matches!(
            Message {
                raw: "|J|+xfix".into()
            }
            .kind(),
            Kind::Join("+xfix")
        ));
    }

    #[test]
    fn parse_nickname_change() {
        assert!(matches!(
            Message {
                raw: "|N|+xfix|@xfix".into()
            }
            .kind(),
            Kind::NicknameChange("+xfix", "@xfix")
        ));
    }

    #[test]
    fn parse_leave() {
        assert!(matches!(
            Message {
                raw: "|L|+xfix".into()
            }
            .kind(),
            Kind::Leave("+xfix")
        ));
    }
}
