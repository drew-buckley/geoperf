use std::{
    ffi::{CStr, CString, c_char, c_double, c_int, c_schar},
    ptr::NonNull,
    str::Utf8Error,
    sync::mpsc,
    time::Duration,
};

use anyhow::Context;
use libffi::high::ClosureMut2;

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
            bind::iperf_set_test_role(self.test_ptr.as_ptr(), role.as_c_char());
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

    pub fn exec(mut self) -> Result<(), error::IperfTestError> {
        let event_tx = self.event_tx.take().unwrap_or_else(|| mpsc::channel().0);
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
        self.exec_inner(&event_callback)
    }

    fn exec_inner(
        self,
        event_callback: &ClosureMut2<'_, *mut bind::IperfTest, *mut c_char, ()>,
    ) -> Result<(), error::IperfTestError> {
        // SAFETY: `closure` will be dropped at the end of this function, same as `self`.
        unsafe {
            bind::iperf_set_test_json_callback(
                self.test_ptr.as_ptr(),
                Some(std::mem::transmute(*event_callback.code_ptr())),
            );
            if bind::iperf_run_client(self.test_ptr.as_ptr()) < 0 {
                i_errno_result().map_err(error::IperfTestError::Execution)?;
            }
        };

        todo!()
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
    Server,
}

impl TestRole {
    pub fn as_c_char(&self) -> c_char {
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
        streams: Vec<IntervalStream>,
        sum: IntervalSum,
    }

    #[derive(Debug, Deserialize)]
    pub struct IntervalStream {
        socket: i32,
        start: f64,
        end: f64,
        bytes: u64,
        bits_per_second: f64,
        #[serde(default)]
        retransmits: Option<i64>,
        #[serde(default)]
        rtt: Option<u32>,
        #[serde(default)]
        jitter_ms: Option<f64>,
        #[serde(default)]
        lost_percent: Option<f64>,
        omitted: bool,
        sender: bool,
    }

    #[derive(Debug, Deserialize)]
    pub struct IntervalSum {
        start: f64,
        end: f64,
        bytes: u64,
        bits_per_second: f64,
        omitted: bool,
    }

    #[derive(Debug, Deserialize)]
    pub struct End {
        sum_sent: Option<Sum>,
        sum_received: Option<Sum>,
    }

    #[derive(Debug, Deserialize)]
    pub struct Sum {
        seconds: f64,
        bytes: u64,
        bits_per_second: f64,
        #[serde(default)]
        retransmits: Option<i64>,
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

pub fn i_errno_result() -> Result<(), bind::error::IperfErrno> {
    // SAFETY: just a errno global; relax
    let err_code = unsafe { bind::i_errno };
    match bind::error::IperfErrno::from_raw(err_code) {
        Some(err) => Err(err),
        None => Ok(()),
    }
}

pub mod error {
    use crate::iperf3::libiperf::bind;

    #[derive(Debug, thiserror::Error)]
    pub enum IperfTestError {
        #[error("failed to marshall data at the libiperf C boundary")]
        DataMarshalling(#[source] anyhow::Error),

        #[error("error occurred during test execution")]
        Execution(#[source] bind::error::IperfErrno),
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

    pub mod error {
        use super::*;

        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, thiserror::Error)]
        #[repr(i32)]
        pub enum IperfErrno {
            // Parameter errors
            /// `IESERVCLIENT`
            #[error("cannot be both server and client")]
            ServerAndClient = 1,
            /// `IENOROLE`
            #[error("must be either a client (-c) or server (-s)")]
            NoRole = 2,
            /// `IESERVERONLY`
            #[error("option is server only")]
            ServerOnly = 3,
            /// `IECLIENTONLY`
            #[error("option is client only")]
            ClientOnly = 4,
            /// `IEDURATION`
            #[error("test duration out of range")]
            Duration = 5,
            /// `IENUMSTREAMS`
            #[error("invalid number of parallel streams")]
            NumStreams = 6,
            /// `IEBLOCKSIZE`
            #[error("block size too large")]
            BlockSize = 7,
            /// `IEBUFSIZE`
            #[error("socket buffer size too large")]
            BufSize = 8,
            /// `IEINTERVAL`
            #[error("invalid report interval")]
            Interval = 9,
            /// `IEMSS`
            #[error("TCP MSS too large")]
            Mss = 10,
            /// `IENOSENDFILE`
            #[error("this OS does not support sendfile")]
            NoSendfile = 11,
            /// `IEOMIT`
            #[error("invalid value for --omit")]
            Omit = 12,
            /// `IEUNIMP`
            #[error("option is not implemented yet")]
            Unimplemented = 13,
            /// `IEFILE`
            #[error("unable to open -F file")]
            File = 14,
            /// `IEBURST`
            #[error("invalid burst count")]
            Burst = 15,
            /// `IEENDCONDITIONS`
            #[error("only one test end condition (-t, -n, -k) may be specified")]
            EndConditions = 16,
            /// `IELOGFILE`
            #[error("unable to open log file")]
            LogFile = 17,
            /// `IENOSCTP`
            #[error("no SCTP support available")]
            NoSctp = 18,
            /// `IEBIND`
            #[error("--bind must be specified to use --cport")]
            Bind = 19,
            /// `IEUDPBLOCKSIZE`
            #[error("invalid UDP block size")]
            UdpBlockSize = 20,
            /// `IEBADTOS`
            #[error("bad TOS value (must be 0-255)")]
            BadTos = 21,
            /// `IESETCLIENTAUTH`
            #[error(
                "client authentication requires a username, password, and path to a valid RSA public key"
            )]
            SetClientAuth = 22,
            /// `IESETSERVERAUTH`
            #[error(
                "server authentication requires a path to a valid RSA private key and a user credential file"
            )]
            SetServerAuth = 23,
            /// `IEBADFORMAT`
            #[error("bad format specifier (valid formats are [kmgtKMGT])")]
            BadFormat = 24,
            /// `IEREVERSEBIDIR`
            #[error("cannot be both reverse and bidirectional")]
            ReverseBidir = 25,
            /// `IEBADPORT`
            #[error("port number must be between 1 and 65535")]
            BadPort = 26,
            /// `IETOTALRATE`
            #[error("total required bandwidth is larger than server limit")]
            TotalRate = 27,
            /// `IETOTALINTERVAL`
            #[error("invalid time interval for calculating average data rate")]
            TotalInterval = 28,
            /// `IESKEWTHRESHOLD`
            #[error("skew threshold must be a positive number")]
            SkewThreshold = 29,
            /// `IEIDLETIMEOUT`
            #[error("idle timeout is not positive or exceeds the allowed limit")]
            IdleTimeout = 30,
            /// `IERCVTIMEOUT`
            #[error("receive timeout is invalid or out of range")]
            RcvTimeout = 31,
            /// `IERVRSONLYRCVTIMEOUT`
            #[error("client receive timeout is only valid in receiving mode")]
            ReverseOnlyRcvTimeout = 32,
            /// `IESNDTIMEOUT`
            #[error("send timeout is invalid or out of range")]
            SndTimeout = 33,
            /// `IEUDPFILETRANSFER`
            #[error("cannot transfer file using UDP")]
            UdpFileTransfer = 34,
            /// `IESERVERAUTHUSERS`
            #[error("cannot access authorized users file")]
            ServerAuthUsers = 35,
            /// `IECNTLKA`
            #[error(
                "control connection keepalive period must be larger than the full retry period (interval * count)"
            )]
            CntlKa = 36,
            /// `IEMAXSERVERTESTDURATIONEXCEEDED`
            #[error("client's requested duration exceeds the server's maximum")]
            MaxServerTestDurationExceeded = 37,
            /// `IEUNITVAL`
            #[error("invalid unit value or suffix")]
            UnitVal = 38,
            /// `IERVRSONLYSKIPRXCOPY`
            #[error("this OS does not support --skip-rx-copy")]
            SkipRxCopyUnsupported = 39,
            /// `IEBADDSCP`
            #[error("bad DSCP value")]
            BadDscp = 40,
            // Test errors
            /// `IENEWTEST`
            #[error("unable to create a new test")]
            NewTest = 100,
            /// `IEINITTEST`
            #[error("test initialization failed")]
            InitTest = 101,
            /// `IELISTEN`
            #[error("unable to start listener for connections")]
            Listen = 102,
            /// `IECONNECT`
            #[error("unable to connect to server")]
            Connect = 103,
            /// `IEACCEPT`
            #[error("unable to accept connection from client")]
            Accept = 104,
            /// `IESENDCOOKIE`
            #[error("unable to send cookie to server")]
            SendCookie = 105,
            /// `IERECVCOOKIE`
            #[error("unable to receive cookie at server")]
            RecvCookie = 106,
            /// `IECTRLWRITE`
            #[error("unable to write to the control socket")]
            CtrlWrite = 107,
            /// `IECTRLREAD`
            #[error("unable to read from the control socket")]
            CtrlRead = 108,
            /// `IECTRLCLOSE`
            #[error("control socket closed unexpectedly")]
            CtrlClose = 109,
            /// `IEMESSAGE`
            #[error(
                "received an unknown control message (ensure the other side is iperf3, not iperf)"
            )]
            Message = 110,
            /// `IESENDMESSAGE`
            #[error("unable to send control message")]
            SendMessage = 111,
            /// `IERECVMESSAGE`
            #[error("unable to receive control message")]
            RecvMessage = 112,
            /// `IESENDPARAMS`
            #[error("unable to send parameters to server")]
            SendParams = 113,
            /// `IERECVPARAMS`
            #[error("unable to receive parameters from client")]
            RecvParams = 114,
            /// `IEPACKAGERESULTS`
            #[error("unable to package results")]
            PackageResults = 115,
            /// `IESENDRESULTS`
            #[error("unable to send results")]
            SendResults = 116,
            /// `IERECVRESULTS`
            #[error("unable to receive results")]
            RecvResults = 117,
            /// `IESELECT`
            #[error("select failed")]
            Select = 118,
            /// `IECLIENTTERM`
            #[error("the client has terminated")]
            ClientTerm = 119,
            /// `IESERVERTERM`
            #[error("the server has terminated")]
            ServerTerm = 120,
            /// `IEACCESSDENIED`
            #[error("the server is busy running a test; try again later")]
            AccessDenied = 121,
            /// `IESETNODELAY`
            #[error("unable to set TCP/SCTP NODELAY")]
            SetNoDelay = 122,
            /// `IESETMSS`
            #[error("unable to set TCP/SCTP MSS")]
            SetMss = 123,
            /// `IESETBUF`
            #[error("unable to set socket buffer size")]
            SetBuf = 124,
            /// `IESETTOS`
            #[error("unable to set IP TOS")]
            SetTos = 125,
            /// `IESETCOS`
            #[error("unable to set IPv6 traffic class")]
            SetCos = 126,
            /// `IESETFLOW`
            #[error("unable to set IPv6 flow label")]
            SetFlow = 127,
            /// `IEREUSEADDR`
            #[error("unable to set reuse address on socket")]
            ReuseAddr = 128,
            /// `IENONBLOCKING`
            #[error("unable to set socket to non-blocking")]
            NonBlocking = 129,
            /// `IESETWINDOWSIZE`
            #[error("unable to set socket window size")]
            SetWindowSize = 130,
            /// `IEPROTOCOL`
            #[error("protocol does not exist")]
            Protocol = 131,
            /// `IEAFFINITY`
            #[error("unable to set CPU affinity")]
            Affinity = 132,
            /// `IEDAEMON`
            #[error("unable to become a daemon")]
            Daemon = 133,
            /// `IESETCONGESTION`
            #[error(
                "unable to set TCP_CONGESTION (congestion control algorithm not supported on this host)"
            )]
            SetCongestion = 134,
            /// `IEPIDFILE`
            #[error("unable to write PID file")]
            PidFile = 135,
            /// `IEV6ONLY`
            #[error("unable to set/unset IPV6_V6ONLY")]
            V6Only = 136,
            /// `IESETSCTPDISABLEFRAG`
            #[error("unable to set SCTP_DISABLE_FRAGMENTS")]
            SetSctpDisableFrag = 137,
            /// `IESETSCTPNSTREAM`
            #[error("unable to set SCTP_INIT number of SCTP streams")]
            SetSctpNStream = 138,
            /// `IESETSCTPBINDX`
            #[error("unable to process sctp_bindx() parameters")]
            SetSctpBindx = 139,
            /// `IESETPACING`
            #[error("unable to set socket pacing rate")]
            SetPacing = 140,
            /// `IESETBUF2`
            #[error("socket buffer size not set correctly (written value != read value)")]
            BufSizeMismatch = 141,
            /// `IEAUTHTEST`
            #[error("test authorization failed")]
            AuthTest = 142,
            /// `IEBINDDEV`
            #[error("unable to bind to device (check permissions)")]
            BindDev = 143,
            /// `IENOMSG`
            #[error("idle timeout waiting to receive data")]
            NoMsg = 144,
            /// `IESETDONTFRAGMENT`
            #[error("unable to set IP Do-Not-Fragment flag")]
            SetDontFragment = 145,
            /// `IEBINDDEVNOSUPPORT`
            #[error("`<ip>%<dev>` is not supported as the system does not support bind to device")]
            BindDevNoSupport = 146,
            /// `IEHOSTDEV`
            #[error(
                "host device name (<ip>%<dev>) is only supported (and required) for IPv6 link-local addresses"
            )]
            HostDev = 147,
            /// `IESETUSERTIMEOUT`
            #[error("unable to set TCP USER_TIMEOUT")]
            SetUserTimeout = 148,
            /// `IEPTHREADCREATE`
            #[error("unable to create thread")]
            PthreadCreate = 150,
            /// `IEPTHREADCANCEL`
            #[error("unable to cancel thread")]
            PthreadCancel = 151,
            /// `IEPTHREADJOIN`
            #[error("unable to join thread")]
            PthreadJoin = 152,
            /// `IEPTHREADATTRINIT`
            #[error("unable to initialize thread attributes")]
            PthreadAttrInit = 153,
            /// `IEPTHREADATTRDESTROY`
            #[error("unable to destroy thread attributes")]
            PthreadAttrDestroy = 154,
            /// `IESETCNTLKA`
            #[error("unable to set socket keepalive (SO_KEEPALIVE) option")]
            SetCntlKa = 155,
            /// `IESETCNTLKAKEEPIDLE`
            #[error("unable to set socket keepalive TCP period (TCP_KEEPIDLE) option")]
            SetCntlKaKeepIdle = 156,
            /// `IESETCNTLKAINTERVAL`
            #[error("unable to set/get socket keepalive TCP retry interval (TCP_KEEPINTVL) option")]
            SetCntlKaInterval = 157,
            /// `IESETCNTLKACOUNT`
            #[error(
                "unable to set/get socket keepalive TCP number of retries (TCP_KEEPCNT) option"
            )]
            SetCntlKaCount = 158,
            /// `IEPTHREADSIGMASK`
            #[error("unable to change mask of blocked signals")]
            PthreadSigmask = 159,
            /// `IESERVERTESTDURATIONEXPIRED`
            #[error("server test duration expired")]
            ServerTestDurationExpired = 160,
            // Stream errors
            /// `IECREATESTREAM`
            #[error("unable to create a new stream")]
            CreateStream = 200,
            /// `IEINITSTREAM`
            #[error("unable to initialize stream")]
            InitStream = 201,
            /// `IESTREAMLISTEN`
            #[error("unable to start stream listener")]
            StreamListen = 202,
            /// `IESTREAMCONNECT`
            #[error("unable to connect stream")]
            StreamConnect = 203,
            /// `IESTREAMACCEPT`
            #[error("unable to accept stream connection")]
            StreamAccept = 204,
            /// `IESTREAMWRITE`
            #[error("unable to write to stream socket")]
            StreamWrite = 205,
            /// `IESTREAMREAD`
            #[error("unable to read from stream socket")]
            StreamRead = 206,
            /// `IESTREAMCLOSE`
            #[error("stream socket closed unexpectedly")]
            StreamClose = 207,
            /// `IESTREAMID`
            #[error("stream has an invalid ID")]
            StreamId = 208,
            // Timer errors
            /// `IENEWTIMER`
            #[error("unable to create new timer")]
            NewTimer = 300,
            /// `IEUPDATETIMER`
            #[error("unable to update timer")]
            UpdateTimer = 301,
        }

        impl IperfErrno {
            /// Returns `None` for `IENONE` and codes not known to these bindings.
            pub fn from_raw(code: c_int) -> Option<Self> {
                Some(match code {
                    1 => Self::ServerAndClient,
                    2 => Self::NoRole,
                    3 => Self::ServerOnly,
                    4 => Self::ClientOnly,
                    5 => Self::Duration,
                    6 => Self::NumStreams,
                    7 => Self::BlockSize,
                    8 => Self::BufSize,
                    9 => Self::Interval,
                    10 => Self::Mss,
                    11 => Self::NoSendfile,
                    12 => Self::Omit,
                    13 => Self::Unimplemented,
                    14 => Self::File,
                    15 => Self::Burst,
                    16 => Self::EndConditions,
                    17 => Self::LogFile,
                    18 => Self::NoSctp,
                    19 => Self::Bind,
                    20 => Self::UdpBlockSize,
                    21 => Self::BadTos,
                    22 => Self::SetClientAuth,
                    23 => Self::SetServerAuth,
                    24 => Self::BadFormat,
                    25 => Self::ReverseBidir,
                    26 => Self::BadPort,
                    27 => Self::TotalRate,
                    28 => Self::TotalInterval,
                    29 => Self::SkewThreshold,
                    30 => Self::IdleTimeout,
                    31 => Self::RcvTimeout,
                    32 => Self::ReverseOnlyRcvTimeout,
                    33 => Self::SndTimeout,
                    34 => Self::UdpFileTransfer,
                    35 => Self::ServerAuthUsers,
                    36 => Self::CntlKa,
                    37 => Self::MaxServerTestDurationExceeded,
                    38 => Self::UnitVal,
                    39 => Self::SkipRxCopyUnsupported,
                    40 => Self::BadDscp,
                    100 => Self::NewTest,
                    101 => Self::InitTest,
                    102 => Self::Listen,
                    103 => Self::Connect,
                    104 => Self::Accept,
                    105 => Self::SendCookie,
                    106 => Self::RecvCookie,
                    107 => Self::CtrlWrite,
                    108 => Self::CtrlRead,
                    109 => Self::CtrlClose,
                    110 => Self::Message,
                    111 => Self::SendMessage,
                    112 => Self::RecvMessage,
                    113 => Self::SendParams,
                    114 => Self::RecvParams,
                    115 => Self::PackageResults,
                    116 => Self::SendResults,
                    117 => Self::RecvResults,
                    118 => Self::Select,
                    119 => Self::ClientTerm,
                    120 => Self::ServerTerm,
                    121 => Self::AccessDenied,
                    122 => Self::SetNoDelay,
                    123 => Self::SetMss,
                    124 => Self::SetBuf,
                    125 => Self::SetTos,
                    126 => Self::SetCos,
                    127 => Self::SetFlow,
                    128 => Self::ReuseAddr,
                    129 => Self::NonBlocking,
                    130 => Self::SetWindowSize,
                    131 => Self::Protocol,
                    132 => Self::Affinity,
                    133 => Self::Daemon,
                    134 => Self::SetCongestion,
                    135 => Self::PidFile,
                    136 => Self::V6Only,
                    137 => Self::SetSctpDisableFrag,
                    138 => Self::SetSctpNStream,
                    139 => Self::SetSctpBindx,
                    140 => Self::SetPacing,
                    141 => Self::BufSizeMismatch,
                    142 => Self::AuthTest,
                    143 => Self::BindDev,
                    144 => Self::NoMsg,
                    145 => Self::SetDontFragment,
                    146 => Self::BindDevNoSupport,
                    147 => Self::HostDev,
                    148 => Self::SetUserTimeout,
                    150 => Self::PthreadCreate,
                    151 => Self::PthreadCancel,
                    152 => Self::PthreadJoin,
                    153 => Self::PthreadAttrInit,
                    154 => Self::PthreadAttrDestroy,
                    155 => Self::SetCntlKa,
                    156 => Self::SetCntlKaKeepIdle,
                    157 => Self::SetCntlKaInterval,
                    158 => Self::SetCntlKaCount,
                    159 => Self::PthreadSigmask,
                    160 => Self::ServerTestDurationExpired,
                    200 => Self::CreateStream,
                    201 => Self::InitStream,
                    202 => Self::StreamListen,
                    203 => Self::StreamConnect,
                    204 => Self::StreamAccept,
                    205 => Self::StreamWrite,
                    206 => Self::StreamRead,
                    207 => Self::StreamClose,
                    208 => Self::StreamId,
                    300 => Self::NewTimer,
                    301 => Self::UpdateTimer,
                    _ => return None,
                })
            }

            pub fn as_raw(self) -> c_int {
                self as c_int
            }
        }
    }
}
