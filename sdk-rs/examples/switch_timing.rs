// How long the two subscriptions of a session switch take (the whole view and every event), against
// the running daemon: `cargo run --release -p neo-sdk --example switch_timing -- <session> [dir]`.
// With a dir, the view and the events are written there for term's `switch_timing`.
use std::time::Instant;

fn main() {
    let session = std::env::args().nth(1).expect("session id");
    let client = neo_sdk::Client::connect(neo_sdk::Options::new("probe")).unwrap();
    let events = client.events();
    let t = Instant::now();
    let view = client.subscribe_view(&session).unwrap();
    eprintln!("view.subscribe: {:?} ({} items)", t.elapsed(), view.items.len());
    let t = Instant::now();
    let sub = client.subscribe(&session, 1).unwrap();
    let mut records = Vec::new();
    while let Ok(e) = events.try_recv() {
        if let neo_sdk::Event::Session(r) = e {
            records.push(r);
        }
    }
    let n = records.len();
    eprintln!(
        "session.subscribe: {:?} ({n} events, through {})",
        t.elapsed(),
        sub.replayed_through
    );
    if let Some(dir) = std::env::args().nth(2) {
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            format!("{dir}/view.json"),
            serde_json::to_string(&view).unwrap(),
        )
        .unwrap();
        std::fs::write(
            format!("{dir}/events.json"),
            serde_json::to_string(&records).unwrap(),
        )
        .unwrap();
    }
}
