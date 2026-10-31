use std::{net::SocketAddr, time::Duration};

use size::Size;

use crate::iperf3::libiperf::{
    IperfTest, TestRole,
    event::{EventReceiver, IperfEvent, error::EventReceiveError},
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
        let event = event_rx.recv();
        log::debug!("Event RX: {:#?}", event);
        match event {
            Ok(IperfEvent::Interval(interval)) => {
                log::info!(
                    "Interval [{:.3}s-{:.3}s]: transfer={:.3}, bitrate={}",
                    interval.sum.start,
                    interval.sum.end,
                    Size::from_bytes(interval.sum.bytes),
                    scaled_bitrate(interval.sum.bits_per_second)
                );
            }
            Ok(IperfEvent::End(end)) => {
                let (tx_transfer, tx_bitrate) = end
                    .sum_sent
                    .map(|s| (s.bytes, s.bits_per_second))
                    .unwrap_or_default();
                let (rx_transfer, rx_bitrate) = end
                    .sum_received
                    .map(|s| (s.bytes, s.bits_per_second))
                    .unwrap_or_default();

                log::info!(
                    "Summary: TX transfer={:.3}, TX bitrate={}, RX transfer={:.3}, RX bitrate={}",
                    Size::from_bytes(tx_transfer),
                    scaled_bitrate(tx_bitrate),
                    Size::from_bytes(rx_transfer),
                    scaled_bitrate(rx_bitrate),
                )
            }
            Ok(_) => (),
            Err(EventReceiveError::EventStreamOver) => break,
            Err(EventReceiveError::ParseError(err)) => {
                log::error!("Failed to parse iperf event: {:?}", err);
            }
        };
    }

    Ok(())
}

fn scaled_bitrate(bitrate: f64) -> String {
    if bitrate >= 1e18 {
        return format!("{:.3} bit/s", bitrate);
    }

    let mut scaled_bitrate = bitrate;
    let mut scale_steps = 0;
    loop {
        let next_scaled = scaled_bitrate / 1e3;
        if next_scaled >= 1.0 {
            scale_steps += 1;
            scaled_bitrate = next_scaled;
        } else {
            break;
        }
    }

    format!(
        "{:.3} {}bits/s",
        scaled_bitrate,
        match scale_steps {
            0 => "",
            1 => "K",
            2 => "M",
            3 => "G",
            4 => "T",
            5 => "P",
            _ => unreachable!(),
        }
    )
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
