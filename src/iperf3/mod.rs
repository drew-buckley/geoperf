use std::{net::SocketAddr, time::Duration};

use crate::iperf3::libiperf::{
    IperfTest, TestRole,
    event::{EventReceiver, error::EventReceiveError},
};

mod libiperf;

pub struct IperfClient {
    test: IperfTest,
}

impl IperfClient {
    pub fn try_new(
        server_hostname: impl AsRef<str>,
        port: u16,
    ) -> Result<Self, error::Iperf3ClientError> {
        let test = IperfTest::try_new().map_err(error::Iperf3ClientError::LibiperfTest)?;
        test.set_test_role(TestRole::Client);
        test.set_test_server_hostname(server_hostname)
            .map_err(error::Iperf3ClientError::LibiperfTest)?;
        test.set_test_server_port(port);
        Ok(Self { test })
    }

    pub fn set_bind(&self, bind_address: impl AsRef<str>) -> Result<(), error::Iperf3ClientError> {
        self.test
            .set_test_bind_address(bind_address)
            .map_err(error::Iperf3ClientError::LibiperfTest)
    }

    pub fn set_duration(&self, duration: Duration) -> Result<(), error::Iperf3ClientError> {
        self.test
            .set_test_duration(duration)
            .map_err(error::Iperf3ClientError::LibiperfTest)
    }

    pub fn exec(mut self) -> Result<(), error::Iperf3ClientError> {
        // safe to unwrap; only time we attempt to subscribe and self is consumed
        let event_rx = self.test.subscribe_to_events().unwrap();
        let event_rx_task = std::thread::spawn(|| rx_events(event_rx));
        self.test
            .exec()
            .map_err(error::Iperf3ClientError::LibiperfTest)?;
        event_rx_task.join().expect("event rx task panicked")?;
        Ok(())
    }
}

fn rx_events(event_rx: EventReceiver) -> Result<(), error::Iperf3ClientError> {
    loop {
        let event = match event_rx.recv() {
            Ok(event) => event,
            Err(EventReceiveError::EventStreamOver) => return Ok(()),
            Err(EventReceiveError::ParseError(err)) => {
                log::error!("Failed to parse iperf event: {:?}", err);
                continue;
            }
        };
        log::debug!("{:#?}", event);
    }
}

pub mod error {
    use crate::iperf3::libiperf::{error::IperfTestError, event::error::EventReceiveError};

    #[derive(Debug, thiserror::Error)]
    pub enum Iperf3ClientError {
        #[error("failure occurred in libiperf client test")]
        LibiperfTest(IperfTestError),

        #[error("failure occurred when receiving test events")]
        EventReceive(EventReceiveError),
    }
}
