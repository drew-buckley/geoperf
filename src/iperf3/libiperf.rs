use std::{
    ffi::{CStr, CString, c_char, c_double, c_int, c_schar},
    time::Duration,
};

use anyhow::Context;

pub struct IperfTest {
    test_ptr: *mut bind::IperfTest,
}

impl IperfTest {
    pub fn new() -> Self {
        // SAFETY: test_ptr is freed on Drop
        let test_ptr = unsafe { bind::iperf_new_test() };

        // SAFETY: test_ptr allocated above and only freed on drop
        unsafe {
            bind::iperf_defaults(test_ptr);
        }

        Self { test_ptr }
    }

    pub fn set_test_role(&self, role: TestRole) {
        // SAFETY: test_ptr allocated above and only freed on drop
        unsafe {
            bind::iperf_set_test_role(self.test_ptr, role.as_c_char());
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

        // SAFETY: test_ptr allocated above and only freed on drop
        unsafe {
            bind::iperf_set_test_server_hostname(self.test_ptr, server_hostname);
        }

        Ok(())
    }

    pub fn set_test_server_port(&self, port: u16) {
        // SAFETY: test_ptr allocated above and only freed on drop
        unsafe { bind::iperf_set_test_server_port(self.test_ptr, port.into()) }
    }

    pub fn set_test_duration(&self, duration: Duration) -> Result<(), error::IperfTestError> {
        // SAFETY: test_ptr allocated above and only freed on drop
        unsafe {
            bind::iperf_set_test_duration(
                self.test_ptr,
                duration
                    .as_secs()
                    .try_into()
                    .context("context")
                    .map_err(error::IperfTestError::DataMarshalling)?,
            );
        }

        Ok(())
    }
}

impl Drop for IperfTest {
    fn drop(&mut self) {
        // SAFETY: freeing of test_ptr; will only happen on drop
        unsafe {
            bind::iperf_free_test(self.test_ptr);
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

unsafe extern "C" fn on_json_output(_test: *mut bind::IperfTest, json: *mut c_char) {}

pub mod error {
    #[derive(Debug, thiserror::Error)]
    pub enum IperfTestError {
        #[error("failed to marshall data at the libiperf C boundary")]
        DataMarshalling(#[source] anyhow::Error),
    }
}

pub(super) mod bind {
    //! FFI bindings to libiperf (`iperf_api.h`).

    #![allow(dead_code)]

    use std::ffi::{c_char, c_double, c_int, c_schar};
    use std::marker::{PhantomData, PhantomPinned};

    type JsonCallback = unsafe extern "C" fn(*mut IperfTest, *mut c_char);

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
    }
}
