use pokemon_showdown_client::message::{Kind, QueryResponse, Room};
use pokemon_showdown_client::{Result, Showdown};
use std::cmp::Ordering;
use std::collections::BinaryHeap;

fn main() -> Result<()> {
    let mut connection = Showdown::connect("play.pokemonshowdown.com")?;
    connection.send_global_command("cmd rooms")?;
    loop {
        if let Kind::QueryResponse(QueryResponse::Rooms(rooms)) = connection.receive()?.parse().kind
        {
            println!("Top 5 most popular rooms");
            let mut rooms: BinaryHeap<_> = rooms
                .iter()
                .map(|r| ComparingWith(|r: &&Room| r.user_count, r))
                .collect();
            for _ in 0..5 {
                if let Some(ComparingWith(_, room)) = rooms.pop() {
                    println!("{} with {} users", room.title, room.user_count);
                } else {
                    break;
                }
            }
            return Ok(());
        }
    }
}

struct ComparingWith<F, T>(F, T);

impl<F, T, U> PartialEq for ComparingWith<F, T>
where
    F: Fn(&T) -> U,
    U: PartialEq,
{
    fn eq(&self, other: &Self) -> bool {
        (self.0)(&self.1) == (other.0)(&other.1)
    }
}

impl<F, T, U> Eq for ComparingWith<F, T>
where
    F: Fn(&T) -> U,
    U: Eq,
{
}

impl<F, T, U> PartialOrd for ComparingWith<F, T>
where
    F: Fn(&T) -> U,
    U: PartialOrd,
{
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        (self.0)(&self.1).partial_cmp(&(other.0)(&other.1))
    }
}

impl<F, T, U> Ord for ComparingWith<F, T>
where
    F: Fn(&T) -> U,
    U: Ord,
{
    fn cmp(&self, other: &Self) -> Ordering {
        (self.0)(&self.1).cmp(&(other.0)(&other.1))
    }
}
