use std::{
    net::{Ipv4Addr, SocketAddr, SocketAddrV4},
    num::NonZeroU16,
    process::ExitCode,
};

use anyhow::Context;
use argh::FromArgs;
use geoperf::{
    iperf3::IperfClient,
    servers::{self, Iperf3Server},
};
use toshi::City;

#[derive(Clone, FromArgs)]
/// iperf3 to public servers based on geography
struct Args {
    /// target city of public server
    #[argh(option, short = 'c')]
    city: String,

    /// country code of target city
    #[argh(option, default = "default_country_code()")]
    country_code: String,

    /// interface to bind on client side
    #[argh(option, short = 'b', default = "default_bind()")]
    bind: SocketAddr,
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

    let target_city =
        City::try_new(args.city, args.country_code).context("country code must be 2 characters")?;

    let servers = servers::fetch().context("failed to fetch server list")?;
    let cities =
        toshi::cities(toshi::DatabaseId::Cities1000).context("failed to get city database")?;
    let proximate_cities = cities
        .calculate_proximities(&target_city)
        .context("city not found in database")?;
    for (d, city, server) in proximate_cities
        .iter()
        .filter_map(|(d, c)| servers.get(*c).map(|s| (d, c, s)))
    {
        let coordinates = cities.get(city).unwrap();
        log::trace!(
            "Matching server: {} ({}; {},{})",
            server.site,
            d,
            coordinates.latitude,
            coordinates.longitude
        );
    }
    let mut servers = proximate_cities.iter().filter_map(|(_, c)| servers.get(*c));
    // .context("failed to find matching city")?;

    while let Some(server) = servers.next()
        && let Err(err) = test(server)
    {
        log::error!("Failed to successfully test {}: {:?}", server.site, err);
    }

    Ok(())
}

fn test(server: &Iperf3Server) -> Result<(), anyhow::Error> {
    log::info!("Testing against: {}, {}", server.site, server.country_code);
    let client = IperfClient::try_new(
        &server.host,
        server
            .available_ports()
            .context("failed to get available ports")?
            .first(),
    )
    .context("failed to initialize client")?;

    client.exec().context("iperf client failed to execute test")
}

fn default_bind() -> SocketAddr {
    SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0).into()
}

fn default_country_code() -> String {
    "US".into()
}
