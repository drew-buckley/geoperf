use std::net::SocketAddr;

use argh::FromArgs;

#[derive(Clone, FromArgs)]
/// iperf3 to public servers based on geography
struct Args {
    /// target city of public server
    #[argh(option, short = 'c')]
    city: String,

    /// interface to bind on client side
    #[argh(option, short = 'b')]
    bind: SocketAddr,
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let args: Args = argh::from_env();
    println!("Hello, world!");
}
