use futures::{SinkExt, StreamExt};
use showdown::message::{Kind, UpdateUser};
use showdown::{Result, SendMessage, Stream};
use std::env;

async fn start(login: String, password: String) -> Result<()> {
    let mut stream = Stream::connect("showdown").await?;
    while let Some(message) = stream.next().await {
        let message = message?;
        match message.kind() {
            Kind::Challenge(ch) => {
                ch.login_with_password(&mut stream, &login, &password)
                    .await?
            }
            Kind::UpdateUser(UpdateUser { named: true, .. }) => {
                stream
                    .send(SendMessage::global_command("join bot dev"))
                    .await?
            }
            Kind::Chat(text) if text.message() == ".yay" => {
                stream
                    .send(SendMessage::chat_message(
                        message.room(),
                        format_args!("YAY {}!", text.user().to_uppercase()),
                    ))
                    .await?;
            }
            _ => {}
        }
    }
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let mut args = env::args().skip(1);
    let login = args.next().unwrap();
    let password = args.next().unwrap();
    start(login, password).await
}
