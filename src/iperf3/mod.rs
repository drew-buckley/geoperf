use std::net::SocketAddr;

use riperf3::{ClientBuilder, RiperfError, TransportProtocol};

mod libiperf;

pub struct Iperf3Command<S: AsRef<str>> {
    server: S,
    duration_s: u32,
    bind: Option<SocketAddr>,
}

impl<S: AsRef<str>> Iperf3Command<S> {
    pub fn new(server: S, duration_s: u32) -> Self {
        Self {
            server,
            duration_s,
            bind: None,
        }
    }

    pub fn bind(&mut self, bind: SocketAddr) {
        self.bind = Some(bind);
    }

    pub async fn exec(self) -> Result<(), error::Iperf3ClientError> {
        let mut builder = ClientBuilder::new(self.server.as_ref())
            .duration(self.duration_s)
            .protocol(TransportProtocol::Tcp);
        if let Some(bind) = self.bind.map(|s| s.to_string()) {
            builder = builder.bind_address(&bind);
        }

        let client = builder
            .build()
            .map_err(RiperfError::Config)
            .map_err(error::Iperf3ClientError::new)?;

        let outcome = client.run().await.map_err(error::Iperf3ClientError::new)?;

        todo!()
    }
}

pub mod error {
    use riperf3::RiperfError;

    #[derive(Debug, thiserror::Error)]
    #[error("iperf3 client failed")]
    pub struct Iperf3ClientError {
        #[source]
        source: RiperfError,
    }

    impl Iperf3ClientError {
        pub fn new(source: RiperfError) -> Self {
            Self { source }
        }
    }
}
