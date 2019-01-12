use comparator::collections::BinaryHeap;
use comparator::{comparing, Comparator};
use pokemon_showdown_client::message::{Kind, QueryResponse, Room};
use pokemon_showdown_client::{Result, Showdown};

fn main() -> Result<()> {
    let mut connection = Showdown::connect("showdown")?;
    connection.send_global_command("cmd rooms")?;
    loop {
        if let Kind::QueryResponse(QueryResponse::Rooms(rooms)) = connection.receive()?.parse().kind
        {
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
