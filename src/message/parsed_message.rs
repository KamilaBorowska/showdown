use crate::message::{split2, Kind, UnrecognizedMessage};
use crate::RoomId;

#[derive(Debug)]
pub struct ParsedMessage<'a> {
    pub room_id: RoomId<'a>,
    pub kind: Kind<'a>,
}

impl<'a> ParsedMessage<'a> {
    pub fn parse_message(full_message: &'a str) -> Self {
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
