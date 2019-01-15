use crate::{Error, ErrorInner, RoomId, Sender};
use chrono::NaiveDateTime;
use futures03::{FutureExt, TryFutureExt};
use reqwest::r#async::Client;
use serde_derive::Deserialize;
use std::borrow::Cow;
use std::str;
use tokio::await;
use tokio::prelude::*;

#[derive(Debug)]
pub struct Message {
    pub(super) text: String,
}

impl Message {
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

#[derive(Debug)]
pub struct ParsedMessage<'a> {
    pub room_id: RoomId<'a>,
    pub kind: Kind<'a>,
}

#[derive(Debug)]
pub enum Kind<'a> {
    Chat(Chat<'a>),
    Challenge(Challenge<'a>),
    Html(&'a str),
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

#[derive(Copy, Clone, Debug)]
pub struct Challenge<'a>(&'a str);

impl<'a> Challenge<'a> {
    pub fn login(
        self,
        sender: &'a mut Sender,
        login: &'a str,
    ) -> impl Future<Item = Option<PasswordRequired<'a>>, Error = Error> + 'a {
        let client = Client::new();
        async move {
            let request = client
                .post("http://play.pokemonshowdown.com/action.php")
                .form(&[
                    ("act", "getassertion"),
                    ("userid", login),
                    ("challstr", self.0),
                ]);
            let response =
                Error::from_reqwest(await!(request.send().and_then(|r| r.into_body().concat2())))?;
            let response = str::from_utf8(&response).map_err(|e| Error(ErrorInner::Utf8(e)))?;
            if response == ";" {
                return Ok(Some(PasswordRequired {
                    challstr: self,
                    login,
                    sender,
                    client,
                }));
            }
            await!(sender.send_global_command(&format!("trn {},0,{}", login, response)))?;
            Ok(None)
        }
            .boxed()
            .compat()
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
        let request = client
            .post("http://play.pokemonshowdown.com/action.php")
            .form(&[
                ("act", "login"),
                ("name", login),
                ("pass", password),
                ("challstr", self.0),
            ]);
        async move {
            let response =
                Error::from_reqwest(await!(request.send().and_then(|r| r.into_body().concat2())))?;
            let LoginServerResponse { assertion } =
                serde_json::from_slice(&response[1..]).map_err(|e| Error(ErrorInner::Json(e)))?;
            await!(sender.send_global_command(&format!("trn {},0,{}", login, assertion)))?;
            Ok(())
        }
            .boxed()
            .compat()
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

impl RoomsList<'_> {
    fn parse(arguments: &str) -> Option<RoomsList<'_>> {
        serde_json::from_str(arguments).ok()
    }

    pub fn iter(&self) -> impl Iterator<Item = &Room> {
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
    fn parse(arguments: &'a str) -> Option<UpdateUser<'a>> {
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
