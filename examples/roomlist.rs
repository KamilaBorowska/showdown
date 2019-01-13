#![feature(async_await, await_macro, futures_api)]
#![recursion_limit = "128"]

use comparator::collections::BinaryHeap;
use comparator::{comparing, Comparator};
use pokemon_showdown_client::message::{Kind, QueryResponse, Room};
use pokemon_showdown_client::{connect, Result};
use tokio::await;

async fn start() -> Result<()> {
    let (mut sender, mut receiver) = await!(connect("showdown"))?;
    await!(sender.send_global_command("cmd rooms"))?;
    loop {
        let received = await!(receiver.receive())?;
        if let Kind::QueryResponse(QueryResponse::Rooms(rooms)) = received.parse().kind {
            println!("Top 5 most popular rooms");
            let mut rooms_heap = BinaryHeap::with_comparator(
                comparing(|r: &&Room| r.user_count)
                    .then_comparing(comparing(|r: &&Room| &r.title).reversed()),
            );
            rooms_heap.extend(rooms.iter());
            for _ in 0..5 {
                if let Some(room) = rooms_heap.pop() {
                    println!("{} with {} users", room.title, room.user_count);
                } else {
                    break;
                }
            }
            return Ok(());
        }
    }
}

fn main() {
    tokio::run_async(async { await!(start()).unwrap() });
}
