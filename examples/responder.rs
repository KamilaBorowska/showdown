use showdown::message::{Kind, UpdateUser};
use showdown::{connect, ReceiveExt, Result, SendMessage};
use std::env;

async fn start(login: String, password: String) -> Result<()> {
    let mut stream = connect("showdown").await?;
    loop {
        let message = stream.receive().await?;
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
}

#[tokio::main]
async fn main() -> Result<()> {
    let mut args = env::args().skip(1);
    let login = args.next().unwrap();
    let password = args.next().unwrap();
    start(login, password).await
}
