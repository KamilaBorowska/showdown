use showdown::message::{Kind, UpdateUser};
use showdown::{connect, Result};
use std::env;

async fn start(login: String, password: String) -> Result<()> {
    let (mut sender, mut receiver) = connect("showdown").await?;
    loop {
        let message = receiver.receive().await?;
        match message.kind() {
            Kind::Challenge(ch) => {
                ch.login_with_password(&mut sender, &login, &password)
                    .await?
            }
            Kind::UpdateUser(UpdateUser { named: true, .. }) => {
                sender.send_global_command("join bot dev").await?
            }
            Kind::Chat(chat) if chat.message() == ".yay" => {
                let response = format!("YAY {}!", chat.user().to_uppercase());
                sender
                    .send_chat_message(message.room_id(), &response)
                    .await?
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
