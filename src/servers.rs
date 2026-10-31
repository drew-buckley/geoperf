use ahash::HashMap;
use anyhow::Context;
use serde::{Deserialize, Serialize};

const SERVER_SOURCE: &str = "http://export.iperf3serverlist.net/listed_iperf3_servers.json";

pub fn fetch() -> Result<HashMap<String, Iperf3Server>, anyhow::Error> {
    log::debug!("GET {}", SERVER_SOURCE);
    let mut body = ureq::get(SERVER_SOURCE)
        .call()
        .context("failed to fetch iperf servers")?
        .into_body();

    let json_str = body.read_to_string().context("failed to read out body")?;
    let servers: Vec<Iperf3Server> =
        serde_json::from_str(&json_str).context("failed to parse JSON server list")?;

    Ok(HashMap::from_iter(
        servers.into_iter().map(|s| (s.site.clone(), s)),
    ))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Iperf3Server {
    #[serde(rename = "IP/HOST")]
    pub host: String,
    #[serde(rename = "PORT")]
    pub ports: String,
    #[serde(rename = "OPTIONS")]
    pub options: String,
    #[serde(rename = "GB/S")]
    pub gbps: String,
    #[serde(rename = "CONTINENT")]
    pub continent: String,
    #[serde(rename = "COUNTRY")]
    pub country: String,
    #[serde(rename = "SITE")]
    pub site: String,
    #[serde(rename = "PROVIDER")]
    pub provider: String,
}

impl Iperf3Server {
    pub fn available_ports(&self) -> Result<Iperf3ServerPortOffering, anyhow::Error> {
        let mut ports = self.ports.split("-");
        let start = ports.next().context("failed to find start port token")?;
        let end = ports.next();

        let start: u16 = start.parse().context("failed to parse start port")?;
        let Some(end) = end else {
            return Ok(Iperf3ServerPortOffering::Single(start));
        };

        Ok(Iperf3ServerPortOffering::Range(
            start..end.parse().context("failed to parse end port")?,
        ))
    }
}

#[derive(Debug, Clone)]
pub enum Iperf3ServerPortOffering {
    Single(u16),
    Range(std::ops::Range<u16>),
}

impl Iperf3ServerPortOffering {
    pub fn first(&self) -> u16 {
        match self {
            Self::Single(p) => *p,
            Self::Range(r) => r.start,
        }
    }
}
