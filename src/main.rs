#[macro_use] extern crate log;

use tokio::sync::mpsc;

mod logger;

#[tokio::main]
async fn main() {
    logger::init(log::LevelFilter::max(), true).expect("Failed to initialize logger!");

    info!("Logger initialized!");

    // We use a bounded channel to avoid the server using unreasonable
    // amounts of RAM if something goes wrong
    // let (tx, rx) = mpsc::channel(250);
}
