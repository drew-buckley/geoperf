use std::{
    ffi::{CStr, CString, c_char},
    ptr::NonNull,
    str::Utf8Error,
    sync::mpsc,
    time::Duration,
};

use anyhow::Context;
use libffi::high::ClosureMut2;

use crate::iperf3::libiperf::event::IperfEvent;

pub struct IperfTest {
    test_ptr: NonNull<bind::IperfTest>,
    event_tx: Option<mpsc::Sender<Result<String, Utf8Error>>>,
}

impl IperfTest {
    pub fn try_new() -> Result<Self, error::IperfTestError> {
        // SAFETY: test_ptr is freed on Drop
        let test_ptr = NonNull::new(unsafe { bind::iperf_new_test() })
            .context("iperf_new_test returned NULL")
            .map_err(error::IperfTestError::DataMarshalling)?;

        // SAFETY: test_ptr allocated above and only freed on drop
        unsafe {
            bind::iperf_defaults(test_ptr.as_ptr());
        }

        Ok(Self {
            test_ptr,
            event_tx: None,
        })
    }

    pub fn set_test_role(&self, role: TestRole) {
        // SAFETY: test_ptr is NonNull, allocated in new, and only freed on drop
        unsafe {
            bind::iperf_set_test_role(self.test_ptr.as_ptr(), role.into());
        }
    }

    pub fn set_test_server_hostname(
        &self,
        server_hostname: impl AsRef<str>,
    ) -> Result<(), error::IperfTestError> {
        let server_hostname = CString::new(server_hostname.as_ref())
            .context("failed to marshall CString")
            .map_err(error::IperfTestError::DataMarshalling)?;
        let server_hostname: *const c_char = server_hostname.as_ptr();

        // SAFETY: test_ptr is NonNull, allocated in new, and only freed on drop
        unsafe {
            bind::iperf_set_test_server_hostname(self.test_ptr.as_ptr(), server_hostname);
        }

        Ok(())
    }

    pub fn set_test_server_port(&self, port: u16) {
        // SAFETY: test_ptr is NonNull, allocated in new, and only freed on drop
        unsafe { bind::iperf_set_test_server_port(self.test_ptr.as_ptr(), port.into()) }
    }

    //iperf_set_test_bind_address
    pub fn set_test_bind_address(
        &self,
        bind_address: impl AsRef<str>,
    ) -> Result<(), error::IperfTestError> {
        let bind_address = CString::new(bind_address.as_ref())
            .context("failed to marshall CString")
            .map_err(error::IperfTestError::DataMarshalling)?;
        let bind_address: *const c_char = bind_address.as_ptr();

        // SAFETY: test_ptr is NonNull, allocated in new, and only freed on drop
        unsafe {
            bind::iperf_set_test_server_hostname(self.test_ptr.as_ptr(), bind_address);
        }

        Ok(())
    }

    pub fn set_test_duration(&self, duration: Duration) -> Result<(), error::IperfTestError> {
        // SAFETY: test_ptr is NonNull, allocated in new, and only freed on drop
        unsafe {
            bind::iperf_set_test_duration(
                self.test_ptr.as_ptr(),
                duration
                    .as_secs()
                    .try_into()
                    .context("failed to losslessy convert Duration seconds")
                    .map_err(error::IperfTestError::DataMarshalling)?,
            );
        }

        Ok(())
    }

    pub fn subscribe_to_events(&mut self) -> Option<event::EventReceiver> {
        if self.event_tx.is_none() {
            let (tx, rx) = mpsc::channel();
            self.event_tx = Some(tx);
            Some(event::EventReceiver::new(rx))
        } else {
            None
        }
    }

    pub fn exec(mut self) -> Result<Vec<IperfEvent>, error::IperfTestError> {
        let mut event_rx = None;
        let event_tx = match self.event_tx.take() {
            Some(tx) => tx,
            None => {
                let (tx, rx) = mpsc::channel();
                event_rx = Some(rx);
                tx
            }
        };

        let mut event_callback = move |_t: *mut bind::IperfTest, json: *mut c_char| {
            if !json.is_null() {
                let _ = event_tx.send(
                    unsafe { CStr::from_ptr(json) }
                        .to_str()
                        .map(|s| s.to_owned()),
                );
            }
        };
        let event_callback: ClosureMut2<'_, *mut bind::IperfTest, *mut c_char, ()> =
            ClosureMut2::new(&mut event_callback);

        // `self` should drop before `event_callback`
        self.exec_inner(&event_callback)?;

        let mut events = if let Some(event_rx) = event_rx {
            let res: Result<Vec<String>, Utf8Error> = event_rx.iter().collect();
            // res.map_err(|e| error::IperfTestError::)?.iter().map(|s| serde_json::from_str(s));
            todo!()
        } else {
            Vec::new()
        };

        Ok(events)
    }

    fn exec_inner(
        self,
        event_callback: &ClosureMut2<'_, *mut bind::IperfTest, *mut c_char, ()>,
    ) -> Result<(), error::IperfTestError> {
        // SAFETY: `closure` will be dropped at the end of this function, same as `self`.
        unsafe {
            bind::iperf_set_test_json_output(self.test_ptr.as_ptr(), 1);
            bind::iperf_set_test_json_stream(self.test_ptr.as_ptr(), 1);
            bind::iperf_set_test_json_callback(
                self.test_ptr.as_ptr(),
                Some(std::mem::transmute(*event_callback.code_ptr())),
            );
            if bind::iperf_run_client(self.test_ptr.as_ptr()) < 0 {
                return Err(error::IperfTestError::Execution(
                    error::LibiperfError::last(),
                ));
            }
        };

        Ok(())
    }
}

impl Drop for IperfTest {
    fn drop(&mut self) {
        // SAFETY: test_ptr is NonNull, allocated in new, and only freed on drop
        unsafe {
            bind::iperf_free_test(self.test_ptr.as_ptr());
        }
    }
}

pub enum TestRole {
    Client,
    #[allow(dead_code)]
    Server,
}

impl Into<c_char> for TestRole {
    fn into(self) -> c_char {
        match self {
            Self::Client => bind::ROLE_CLIENT,
            Self::Server => bind::ROLE_SERVER,
        }
    }
}

pub mod event {
    use super::*;

    use serde::Deserialize;
    use std::{str::Utf8Error, sync::mpsc};

    pub struct EventReceiver {
        inner: mpsc::Receiver<Result<String, Utf8Error>>,
    }

    impl EventReceiver {
        pub(super) fn new(inner: mpsc::Receiver<Result<String, Utf8Error>>) -> Self {
            Self { inner }
        }

        pub fn recv(&self) -> Result<IperfEvent, error::EventReceiveError> {
            let event_str = self
                .inner
                .recv()
                .map_err(|_| error::EventReceiveError::EventStreamOver)?
                .map_err(|e| error::EventReceiveError::ParseError(e.into()))?;
            log::trace!("Raw event RX: {}", event_str);
            serde_json::from_str(&event_str)
                .map_err(|e| error::EventReceiveError::ParseError(e.into()))
        }
    }

    #[derive(Debug, Deserialize)]
    #[serde(tag = "event", content = "data", rename_all = "snake_case")]
    pub enum IperfEvent {
        Start(serde_json::Value),
        Interval(Interval),
        End(End),
        Error(String),
        #[serde(other)]
        Other,
    }

    #[derive(Debug, Deserialize)]
    pub struct Interval {
        pub streams: Vec<IntervalStream>,
        pub sum: IntervalSum,
    }

    #[derive(Debug, Deserialize)]
    pub struct IntervalStream {
        pub socket: i32,
        pub start: f64,
        pub end: f64,
        pub bytes: u64,
        pub bits_per_second: f64,
        #[serde(default)]
        pub retransmits: Option<i64>,
        #[serde(default)]
        pub rtt: Option<u32>,
        #[serde(default)]
        pub jitter_ms: Option<f64>,
        #[serde(default)]
        pub lost_percent: Option<f64>,
        pub omitted: bool,
        pub sender: bool,
    }

    #[derive(Debug, Deserialize)]
    pub struct IntervalSum {
        pub start: f64,
        pub end: f64,
        pub bytes: u64,
        pub bits_per_second: f64,
        pub omitted: bool,
    }

    #[derive(Debug, Deserialize)]
    pub struct End {
        pub sum_sent: Option<Sum>,
        pub sum_received: Option<Sum>,
    }

    #[derive(Debug, Deserialize)]
    pub struct Sum {
        pub seconds: f64,
        pub bytes: u64,
        pub bits_per_second: f64,
        #[serde(default)]
        pub retransmits: Option<i64>,
    }

    pub mod error {
        #[derive(Debug, thiserror::Error)]
        pub enum EventReceiveError {
            #[error("failed to parse event")]
            ParseError(#[source] anyhow::Error),

            #[error("event stream over")]
            EventStreamOver,
        }
    }
}

pub mod error {
    use std::ffi::CStr;

    use crate::iperf3::libiperf::bind;

    #[derive(Debug, thiserror::Error)]
    pub enum IperfTestError {
        #[error("failed to marshall data at the libiperf C boundary")]
        DataMarshalling(#[source] anyhow::Error),

        #[error("error occurred during test execution")]
        Execution(#[source] Option<LibiperfError>),
    }

    #[derive(Debug, thiserror::Error)]
    #[error("{message}")]
    pub struct LibiperfError {
        errno: bind::IperfErrno,
        message: String,
    }

    impl LibiperfError {
        pub(super) fn last() -> Option<Self> {
            // SAFETY: just an errno variable
            let Some(errno) = bind::IperfErrno::from_raw(unsafe { bind::i_errno }) else {
                return None;
            };

            // SAFETY: static buffer
            let msg_ptr = unsafe { bind::iperf_strerror(errno.as_raw()) };
            let message = if msg_ptr.is_null() {
                format!("libiperf error {}", errno.as_raw())
            } else {
                // SAFETY: non-null, null-terminated buffer owned by libiperf
                unsafe { CStr::from_ptr(msg_ptr) }
                    .to_string_lossy()
                    .trim_end()
                    .to_owned()
            };

            Some(Self { errno, message })
        }

        pub fn errno(&self) -> bind::IperfErrno {
            self.errno
        }

        pub fn message(&self) -> &str {
            &self.message
        }
    }
}

pub(super) mod bind {
    //! FFI bindings to libiperf (`iperf_api.h`).

    #![allow(dead_code)]

    use std::ffi::{c_char, c_double, c_int, c_schar};
    use std::marker::{PhantomData, PhantomPinned};

    pub type JsonCallback = unsafe extern "C" fn(*mut IperfTest, *mut c_char);

    /// `iperf_test`
    #[repr(C)]
    pub struct IperfTest {
        _data: (),
        _marker: PhantomData<(*mut u8, PhantomPinned)>,
    }

    /// `iperf_stream`
    #[repr(C)]
    pub struct IperfStream {
        _data: (),
        _marker: PhantomData<(*mut u8, PhantomPinned)>,
    }

    /// `FILE`
    #[repr(C)]
    pub struct File {
        _data: (),
        _marker: PhantomData<(*mut u8, PhantomPinned)>,
    }

    pub const ROLE_CLIENT: c_char = b'c' as c_char;
    pub const ROLE_SERVER: c_char = b's' as c_char;

    pub const PTCP: c_int = 1; // SOCK_STREAM
    pub const PUDP: c_int = 2; // SOCK_DGRAM
    pub const PSCTP: c_int = 12;

    pub const DEFAULT_UDP_BLKSIZE: c_int = 1460;
    pub const DEFAULT_TCP_BLKSIZE: c_int = 128 * 1024;
    pub const DEFAULT_SCTP_BLKSIZE: c_int = 64 * 1024;

    pub const TEST_START: c_schar = 1;
    pub const TEST_RUNNING: c_schar = 2;
    pub const TEST_END: c_schar = 4;
    pub const STREAM_BEGIN: c_schar = 5;
    pub const STREAM_RUNNING: c_schar = 6;
    pub const STREAM_END: c_schar = 7;
    pub const ALL_STREAMS_END: c_schar = 8;
    pub const PARAM_EXCHANGE: c_schar = 9;
    pub const CREATE_STREAMS: c_schar = 10;
    pub const SERVER_TERMINATE: c_schar = 11;
    pub const CLIENT_TERMINATE: c_schar = 12;
    pub const EXCHANGE_RESULTS: c_schar = 13;
    pub const DISPLAY_RESULTS: c_schar = 14;
    pub const IPERF_START: c_schar = 15;
    pub const IPERF_DONE: c_schar = 16;
    pub const ACCESS_DENIED: c_schar = -1;
    pub const SERVER_ERROR: c_schar = -2;

    pub const IENONE: c_int = 0;

    unsafe extern "C" {
        pub static mut i_errno: c_int;

        pub fn iperf_new_test() -> *mut IperfTest;
        pub fn iperf_defaults(testp: *mut IperfTest) -> c_int;
        pub fn iperf_free_test(testp: *mut IperfTest);
        pub fn iperf_reset_test(testp: *mut IperfTest);
        pub fn iperf_reset_stats(testp: *mut IperfTest);
        pub fn iperf_parse_arguments(
            testp: *mut IperfTest,
            argc: c_int,
            argv: *mut *mut c_char,
        ) -> c_int;
        pub fn iperf_run_client(testp: *mut IperfTest) -> c_int;
        pub fn iperf_run_server(testp: *mut IperfTest) -> c_int;
        pub fn iperf_got_sigend(testp: *mut IperfTest);
        pub fn iperf_strerror(int_errno: c_int) -> *mut c_char;
        pub fn iperf_get_iperf_version() -> *mut c_char;
        pub fn iperf_get_test_role(ipt: *mut IperfTest) -> c_char;
        pub fn iperf_get_test_reverse(ipt: *mut IperfTest) -> c_int;
        pub fn iperf_get_test_bidirectional(ipt: *mut IperfTest) -> c_int;
        pub fn iperf_get_test_blksize(ipt: *mut IperfTest) -> c_int;
        pub fn iperf_get_test_outfile(ipt: *mut IperfTest) -> *mut File;
        pub fn iperf_get_test_rate(ipt: *mut IperfTest) -> u64;
        pub fn iperf_get_test_pacing_timer(ipt: *mut IperfTest) -> c_int;
        pub fn iperf_get_test_fqrate(ipt: *mut IperfTest) -> u64;
        pub fn iperf_get_test_burst(ipt: *mut IperfTest) -> c_int;
        pub fn iperf_get_test_socket_bufsize(ipt: *mut IperfTest) -> c_int;
        pub fn iperf_get_test_reporter_interval(ipt: *mut IperfTest) -> c_double;
        pub fn iperf_get_test_stats_interval(ipt: *mut IperfTest) -> c_double;
        pub fn iperf_get_test_num_streams(ipt: *mut IperfTest) -> c_int;
        pub fn iperf_get_test_timestamps(ipt: *mut IperfTest) -> c_int;
        pub fn iperf_get_test_timestamp_format(ipt: *mut IperfTest) -> *const c_char;
        pub fn iperf_get_test_repeating_payload(ipt: *mut IperfTest) -> c_int;
        pub fn iperf_get_test_bytes(ipt: *mut IperfTest) -> u64;
        pub fn iperf_get_test_blocks(ipt: *mut IperfTest) -> u64;
        pub fn iperf_get_test_duration(ipt: *mut IperfTest) -> c_int;
        pub fn iperf_get_test_server_hostname(ipt: *mut IperfTest) -> *mut c_char;
        pub fn iperf_get_test_server_port(ipt: *mut IperfTest) -> c_int;
        pub fn iperf_get_test_template(ipt: *mut IperfTest) -> *mut c_char;
        pub fn iperf_get_test_protocol_id(ipt: *mut IperfTest) -> c_int;
        pub fn iperf_get_test_json_output(ipt: *mut IperfTest) -> c_int;
        pub fn iperf_get_test_json_output_string(ipt: *mut IperfTest) -> *mut c_char;
        pub fn iperf_get_test_json_stream(ipt: *mut IperfTest) -> c_int;
        pub fn iperf_get_test_zerocopy(ipt: *mut IperfTest) -> c_int;
        pub fn iperf_get_test_get_server_output(ipt: *mut IperfTest) -> c_int;
        pub fn iperf_get_test_unit_format(ipt: *mut IperfTest) -> c_char;
        pub fn iperf_get_test_bind_address(ipt: *mut IperfTest) -> *mut c_char;
        pub fn iperf_get_test_bind_dev(ipt: *mut IperfTest) -> *mut c_char;
        pub fn iperf_get_test_udp_counters_64bit(ipt: *mut IperfTest) -> c_int;
        pub fn iperf_get_test_one_off(ipt: *mut IperfTest) -> c_int;
        pub fn iperf_get_test_tos(ipt: *mut IperfTest) -> c_int;
        pub fn iperf_get_extra_data(ipt: *mut IperfTest) -> *mut c_char;
        pub fn iperf_get_test_congestion_control(ipt: *mut IperfTest) -> *mut c_char;
        pub fn iperf_get_test_no_delay(ipt: *mut IperfTest) -> c_int;
        pub fn iperf_get_test_connect_timeout(ipt: *mut IperfTest) -> c_int;
        pub fn iperf_get_test_mss(ipt: *mut IperfTest) -> c_int;
        pub fn iperf_get_dont_fragment(ipt: *mut IperfTest) -> c_int;
        pub fn iperf_get_test_idle_timeout(ipt: *mut IperfTest) -> c_int;
        pub fn iperf_set_verbose(ipt: *mut IperfTest, verbose: c_int);
        pub fn iperf_set_test_role(ipt: *mut IperfTest, role: c_char);
        pub fn iperf_set_test_reverse(ipt: *mut IperfTest, reverse: c_int);
        pub fn iperf_set_test_bidirectional(ipt: *mut IperfTest, bidirectional: c_int);
        pub fn iperf_set_test_blksize(ipt: *mut IperfTest, blksize: c_int);
        pub fn iperf_set_test_logfile(ipt: *mut IperfTest, logfile: *const c_char);
        pub fn iperf_set_test_outfile(ipt: *mut IperfTest, outfile: *mut File);
        pub fn iperf_set_test_rate(ipt: *mut IperfTest, rate: u64);
        pub fn iperf_set_test_pacing_timer(ipt: *mut IperfTest, pacing_timer: c_int);
        pub fn iperf_set_test_fqrate(ipt: *mut IperfTest, fqrate: u64);
        pub fn iperf_set_test_burst(ipt: *mut IperfTest, burst: c_int);
        pub fn iperf_set_test_socket_bufsize(ipt: *mut IperfTest, socket_bufsize: c_int);
        pub fn iperf_set_test_reporter_interval(ipt: *mut IperfTest, reporter_interval: c_double);
        pub fn iperf_set_test_stats_interval(ipt: *mut IperfTest, stats_interval: c_double);
        pub fn iperf_set_test_state(ipt: *mut IperfTest, state: c_schar);
        pub fn iperf_set_test_num_streams(ipt: *mut IperfTest, num_streams: c_int);
        pub fn iperf_set_test_timestamps(ipt: *mut IperfTest, timestamps: c_int);
        pub fn iperf_set_test_timestamp_format(ipt: *mut IperfTest, tf: *const c_char);
        pub fn iperf_set_test_repeating_payload(ipt: *mut IperfTest, repeating_payload: c_int);
        pub fn iperf_set_test_bytes(ipt: *mut IperfTest, bytes: u64);
        pub fn iperf_set_test_blocks(ipt: *mut IperfTest, blocks: u64);
        pub fn iperf_set_test_duration(ipt: *mut IperfTest, duration: c_int);
        pub fn iperf_set_test_server_hostname(ipt: *mut IperfTest, server_hostname: *const c_char);
        pub fn iperf_set_test_server_port(ipt: *mut IperfTest, server_port: c_int);
        pub fn iperf_set_test_template(ipt: *mut IperfTest, tmp_template: *const c_char);
        pub fn iperf_set_test_json_output(ipt: *mut IperfTest, json_output: c_int);
        pub fn iperf_set_test_json_stream(ipt: *mut IperfTest, json_stream: c_int);
        pub fn iperf_set_test_zerocopy(ipt: *mut IperfTest, zerocopy: c_int);
        pub fn iperf_set_test_get_server_output(ipt: *mut IperfTest, get_server_output: c_int);
        pub fn iperf_set_test_unit_format(ipt: *mut IperfTest, unit_format: c_char);
        pub fn iperf_set_test_bind_address(ipt: *mut IperfTest, bind_address: *const c_char);
        pub fn iperf_set_test_bind_dev(ipt: *mut IperfTest, bind_dev: *const c_char);
        pub fn iperf_set_test_udp_counters_64bit(ipt: *mut IperfTest, udp_counters_64bit: c_int);
        pub fn iperf_set_test_one_off(ipt: *mut IperfTest, one_off: c_int);
        pub fn iperf_set_test_tos(ipt: *mut IperfTest, tos: c_int);
        pub fn iperf_set_test_extra_data(ipt: *mut IperfTest, dat: *const c_char);
        pub fn iperf_set_test_congestion_control(ipt: *mut IperfTest, cc: *const c_char);
        pub fn iperf_set_test_no_delay(ipt: *mut IperfTest, no_delay: c_int);
        pub fn iperf_set_test_connect_timeout(ipt: *mut IperfTest, ct: c_int);
        pub fn iperf_set_test_mss(ipt: *mut IperfTest, mss: c_int);
        pub fn iperf_set_dont_fragment(ipt: *mut IperfTest, dont_fragment: c_int);
        pub fn iperf_set_test_idle_timeout(ipt: *mut IperfTest, to: c_int);
        pub fn iperf_set_test_json_callback(ipt: *mut IperfTest, callback: Option<JsonCallback>);
    }

    macro_rules! c_int_enum {
        (
            $(#[$attr:meta])*
            $vis:vis enum $name:ident {
                $( $(#[$vattr:meta])* $variant:ident = $value:expr, )*
            }
        ) => {
            $(#[$attr])*
            #[repr(i32)]
            $vis enum $name {
                $( $(#[$vattr])* $variant = $value, )*
            }

            impl $name {
                /// Returns `None` for codes with no matching variant.
                pub fn from_raw(code: c_int) -> Option<Self> {
                    #![allow(non_upper_case_globals)]
                    $( const $variant: c_int = $name::$variant as c_int; )*

                    match code {
                        $( $variant => Some(Self::$variant), )*
                        _ => None,
                    }
                }

                pub fn as_raw(self) -> c_int {
                    self as c_int
                }
            }
        };
    }

    c_int_enum! {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum IperfErrno {
            /// `IESERVCLIENT`
            ServerAndClient = 1,
            /// `IENOROLE`
            NoRole = 2,
            /// `IESERVERONLY`
            ServerOnly = 3,
            /// `IECLIENTONLY`
            ClientOnly = 4,
            /// `IEDURATION`
            Duration = 5,
            /// `IENUMSTREAMS`
            NumStreams = 6,
            /// `IEBLOCKSIZE`
            BlockSize = 7,
            /// `IEBUFSIZE`
            BufSize = 8,
            /// `IEINTERVAL`
            Interval = 9,
            /// `IEMSS`
            Mss = 10,
            /// `IENOSENDFILE`
            NoSendfile = 11,
            /// `IEOMIT`
            Omit = 12,
            /// `IEUNIMP`
            Unimplemented = 13,
            /// `IEFILE`
            File = 14,
            /// `IEBURST`
            Burst = 15,
            /// `IEENDCONDITIONS`
            EndConditions = 16,
            /// `IELOGFILE`
            LogFile = 17,
            /// `IENOSCTP`
            NoSctp = 18,
            /// `IEBIND`
            Bind = 19,
            /// `IEUDPBLOCKSIZE`
            UdpBlockSize = 20,
            /// `IEBADTOS`
            BadTos = 21,
            /// `IESETCLIENTAUTH`
            SetClientAuth = 22,
            /// `IESETSERVERAUTH`
            SetServerAuth = 23,
            /// `IEBADFORMAT`
            BadFormat = 24,
            /// `IEREVERSEBIDIR`
            ReverseBidir = 25,
            /// `IEBADPORT`
            BadPort = 26,
            /// `IETOTALRATE`
            TotalRate = 27,
            /// `IETOTALINTERVAL`
            TotalInterval = 28,
            /// `IESKEWTHRESHOLD`
            SkewThreshold = 29,
            /// `IEIDLETIMEOUT`
            IdleTimeout = 30,
            /// `IERCVTIMEOUT`
            RcvTimeout = 31,
            /// `IERVRSONLYRCVTIMEOUT`
            ReverseOnlyRcvTimeout = 32,
            /// `IESNDTIMEOUT`
            SndTimeout = 33,
            /// `IEUDPFILETRANSFER`
            UdpFileTransfer = 34,
            /// `IESERVERAUTHUSERS`
            ServerAuthUsers = 35,
            /// `IECNTLKA`
            CntlKa = 36,
            /// `IEMAXSERVERTESTDURATIONEXCEEDED`
            MaxServerTestDurationExceeded = 37,
            /// `IEUNITVAL`
            UnitVal = 38,
            /// `IERVRSONLYSKIPRXCOPY`
            SkipRxCopyUnsupported = 39,
            /// `IEBADDSCP`
            BadDscp = 40,
            // Test errors
            /// `IENEWTEST`
            NewTest = 100,
            /// `IEINITTEST`
            InitTest = 101,
            /// `IELISTEN`
            Listen = 102,
            /// `IECONNECT`
            Connect = 103,
            /// `IEACCEPT`
            Accept = 104,
            /// `IESENDCOOKIE`
            SendCookie = 105,
            /// `IERECVCOOKIE`
            RecvCookie = 106,
            /// `IECTRLWRITE`
            CtrlWrite = 107,
            /// `IECTRLREAD`
            CtrlRead = 108,
            /// `IECTRLCLOSE`
            CtrlClose = 109,
            /// `IEMESSAGE`
            Message = 110,
            /// `IESENDMESSAGE`
            SendMessage = 111,
            /// `IERECVMESSAGE`
            RecvMessage = 112,
            /// `IESENDPARAMS`
            SendParams = 113,
            /// `IERECVPARAMS`
            RecvParams = 114,
            /// `IEPACKAGERESULTS`
            PackageResults = 115,
            /// `IESENDRESULTS`
            SendResults = 116,
            /// `IERECVRESULTS`
            RecvResults = 117,
            /// `IESELECT`
            Select = 118,
            /// `IECLIENTTERM`
            ClientTerm = 119,
            /// `IESERVERTERM`
            ServerTerm = 120,
            /// `IEACCESSDENIED`
            AccessDenied = 121,
            /// `IESETNODELAY`
            SetNoDelay = 122,
            /// `IESETMSS`
            SetMss = 123,
            /// `IESETBUF`
            SetBuf = 124,
            /// `IESETTOS`
            SetTos = 125,
            /// `IESETCOS`
            SetCos = 126,
            /// `IESETFLOW`
            SetFlow = 127,
            /// `IEREUSEADDR`
            ReuseAddr = 128,
            /// `IENONBLOCKING`
            NonBlocking = 129,
            /// `IESETWINDOWSIZE`
            SetWindowSize = 130,
            /// `IEPROTOCOL`
            Protocol = 131,
            /// `IEAFFINITY`
            Affinity = 132,
            /// `IEDAEMON`
            Daemon = 133,
            /// `IESETCONGESTION`
            SetCongestion = 134,
            /// `IEPIDFILE`
            PidFile = 135,
            /// `IEV6ONLY`
            V6Only = 136,
            /// `IESETSCTPDISABLEFRAG`
            SetSctpDisableFrag = 137,
            /// `IESETSCTPNSTREAM`
            SetSctpNStream = 138,
            /// `IESETSCTPBINDX`
            SetSctpBindx = 139,
            /// `IESETPACING`
            SetPacing = 140,
            /// `IESETBUF2`
            BufSizeMismatch = 141,
            /// `IEAUTHTEST`
            AuthTest = 142,
            /// `IEBINDDEV`
            BindDev = 143,
            /// `IENOMSG`
            NoMsg = 144,
            /// `IESETDONTFRAGMENT`
            SetDontFragment = 145,
            /// `IEBINDDEVNOSUPPORT`
            BindDevNoSupport = 146,
            /// `IEHOSTDEV`
            HostDev = 147,
            /// `IESETUSERTIMEOUT`
            SetUserTimeout = 148,
            /// `IEPTHREADCREATE`
            PthreadCreate = 150,
            /// `IEPTHREADCANCEL`
            PthreadCancel = 151,
            /// `IEPTHREADJOIN`
            PthreadJoin = 152,
            /// `IEPTHREADATTRINIT`
            PthreadAttrInit = 153,
            /// `IEPTHREADATTRDESTROY`
            PthreadAttrDestroy = 154,
            /// `IESETCNTLKA`
            SetCntlKa = 155,
            /// `IESETCNTLKAKEEPIDLE`
            SetCntlKaKeepIdle = 156,
            /// `IESETCNTLKAINTERVAL`
            SetCntlKaInterval = 157,
            /// `IESETCNTLKACOUNT`
            SetCntlKaCount = 158,
            /// `IEPTHREADSIGMASK`
            PthreadSigmask = 159,
            /// `IESERVERTESTDURATIONEXPIRED`
            ServerTestDurationExpired = 160,
            /// `IECREATESTREAM`
            CreateStream = 200,
            /// `IEINITSTREAM`
            InitStream = 201,
            /// `IESTREAMLISTEN`
            StreamListen = 202,
            /// `IESTREAMCONNECT`
            StreamConnect = 203,
            /// `IESTREAMACCEPT`
            StreamAccept = 204,
            /// `IESTREAMWRITE`
            StreamWrite = 205,
            /// `IESTREAMREAD`
            StreamRead = 206,
            /// `IESTREAMCLOSE`
            StreamClose = 207,
            /// `IESTREAMID`
            StreamId = 208,
            /// `IENEWTIMER`
            NewTimer = 300,
            /// `IEUPDATETIMER`
            UpdateTimer = 301,
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn iperf_errno_round_trips() {
            let mut known = 0;
            for code in -1..=1000 {
                if let Some(errno) = IperfErrno::from_raw(code) {
                    assert_eq!(errno.as_raw(), code, "{errno:?}");
                    known += 1;
                }
            }
            assert_eq!(known, 111);
        }

        #[test]
        fn iperf_errno_rejects_none_and_unknown() {
            for code in [
                IENONE,
                -1,
                41,
                99,
                149,
                161,
                209,
                302,
                c_int::MIN,
                c_int::MAX,
            ] {
                assert_eq!(IperfErrno::from_raw(code), None, "code {code}");
            }
        }

        #[test]
        fn iperf_errno_matches_header_values() {
            let cases = [
                (1, IperfErrno::ServerAndClient),
                (40, IperfErrno::BadDscp),
                (100, IperfErrno::NewTest),
                (103, IperfErrno::Connect),
                (121, IperfErrno::AccessDenied),
                (160, IperfErrno::ServerTestDurationExpired),
                (200, IperfErrno::CreateStream),
                (208, IperfErrno::StreamId),
                (300, IperfErrno::NewTimer),
                (301, IperfErrno::UpdateTimer),
            ];
            for (code, errno) in cases {
                assert_eq!(IperfErrno::from_raw(code), Some(errno));
                assert_eq!(errno.as_raw(), code);
            }
        }
    }
}
