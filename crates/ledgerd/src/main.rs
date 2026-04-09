pub mod proto {
    tonic::include_proto!("ledger");
}

mod health;
mod migrations;
mod server;
mod store;

fn main() {
    println!("ledgerd stub");
}
