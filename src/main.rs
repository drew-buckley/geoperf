use std::{
    net::{Ipv4Addr, SocketAddr, SocketAddrV4},
    num::NonZeroU16,
    process::ExitCode,
};

use anyhow::Context;
use argh::FromArgs;
use geoperf::iperf3::IperfClient;

#[derive(Clone, FromArgs)]
/// iperf3 to public servers based on geography
struct Args {
    /// target city of public server
    #[argh(option, short = 'c')]
    city: String,

    /// interface to bind on client side
    #[argh(option, short = 'b', default = "default_bind()")]
    bind: SocketAddr,

    /// target server
    #[argh(option, short = 't')]
    target: String,

    /// target port
    #[argh(option, short = 'p')]
    port: NonZeroU16,
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            log::error!("geoperf failed: {:?}", err);
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), anyhow::Error> {
    env_logger::init();
    let args: Args = argh::from_env();
    let client = IperfClient::try_new(args.target, args.port.into())
        .context("failed to initialize client")?;

    client
        .exec()
        .context("iperf client failed to execute test")?;

    Ok(())
}

fn default_bind() -> SocketAddr {
    SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0).into()
}
