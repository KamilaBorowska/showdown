use comparator::collections::BinaryHeap;
use comparator::{comparing, Comparator};
use futures::{SinkExt, StreamExt};
use showdown::message::{Kind, QueryResponse, Room};
use showdown::{connect, Result, SendMessage};

#[tokio::main]
async fn main() -> Result<()> {
    let mut stream = connect("showdown").await?;
    stream
        .send(SendMessage::global_command("cmd rooms"))
        .await?;
    while let Some(message) = stream.next().await {
        if let Kind::QueryResponse(QueryResponse::Rooms(rooms)) = message?.kind() {
            println!("Top 5 most popular rooms");
            let mut rooms_heap = BinaryHeap::with_comparator(
                comparing(|r: &&Room<'_>| r.user_count)
                    .then_comparing(comparing(|r: &&Room<'_>| &r.title).reversed()),
            );
            rooms_heap.extend(rooms.iter());
            for _ in 0..5 {
                if let Some(room) = rooms_heap.pop() {
                    println!("{} with {} users", room.title, room.user_count);
                } else {
                    break;
                }
            }
            break;
        }
    }
    Ok(())
}
