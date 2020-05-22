use futures::{SinkExt, StreamExt};
use showdown::chrono::{SubsecRound, Utc};
use showdown::message::{Kind, Text};
use showdown::{Receiver, RoomId, Sender};
use std::error::Error;
use std::net::Ipv4Addr;
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::WebSocketStream;

async fn mock_connection() -> Result<(WebSocketStream<TcpStream>, Sender, Receiver), Box<dyn Error>>
{
    let mut listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
    let port = listener.local_addr()?.port();
    let uri = format!("ws://127.0.0.1:{}", port).parse()?;
    let (socket, result) = tokio::join!(
        async move { tokio_tungstenite::accept_async(listener.accept().await?.0).await },
        showdown::connect_to_url(&uri),
    );
    let (sender, receiver) = result?;
    Ok((socket?, sender, receiver))
}

#[tokio::test]
async fn parsing_chat_messages() -> Result<(), Box<dyn Error>> {
    let (mut socket, _sender, mut receiver) = mock_connection().await?;
    let time = Utc::now().trunc_subsecs(0);
    socket
        .send(Message::Text(format!(
            "|c:|{}|+xfix|Hello|world",
            time.timestamp()
        )))
        .await?;
    let message = receiver.receive().await?;
    let chat = match message.kind() {
        Kind::Text(Text::Chat(chat)) => chat,
        _ => unreachable!(),
    };
    assert_eq!(chat.room_id(), RoomId::LOBBY);
    assert_eq!(chat.timestamp(), time);
    assert_eq!(chat.user(), "+xfix");
    assert_eq!(chat.message(), "Hello|world");
    Ok(())
}

#[tokio::test]
async fn reply_test() -> Result<(), Box<dyn Error>> {
    let (mut socket, mut sender, mut receiver) = mock_connection().await?;
    socket
        .send(Message::Text("|c:|0|+xfix|Hi there".into()))
        .await?;
    let message = receiver.receive().await?;
    let text = match message.kind() {
        Kind::Text(text) => text,
        _ => unreachable!(),
    };
    text.reply(&mut sender, "Hi there").await?;
    assert_eq!(
        socket.next().await.transpose()?,
        Some(Message::Text("| Hi there".into())),
    );
    Ok(())
}
