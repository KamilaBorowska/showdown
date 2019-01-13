use crate::RoomId;
use serde_derive::Deserialize;
use std::borrow::Cow;

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
    QueryResponse(QueryResponse<'a>),
    Unrecognized(UnrecognizedMessage<'a>),
}

impl Kind<'_> {
    fn parse<'a>(command: &str, arguments: &'a str) -> Option<Kind<'a>> {
        Some(match command {
            "queryresponse" => Kind::QueryResponse(QueryResponse::parse(arguments)?),
            _ => return None,
        })
    }
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

#[derive(Debug)]
pub struct UnrecognizedMessage<'a>(&'a str);
