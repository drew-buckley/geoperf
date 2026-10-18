use std::env;

fn main() {
    println!("cargo:rerun-if-env-changed=IPERF_LIB_DIR");
    println!("cargo:rerun-if-env-changed=IPERF_STATIC");

    let statik = env::var("IPERF_STATIC").is_ok_and(|v| v != "0");
    let kind = if statik { "static" } else { "dylib" };

    if let Ok(dir) = env::var("IPERF_LIB_DIR") {
        println!("cargo:rustc-link-search=native={dir}");
        println!("cargo:rustc-link-lib={kind}=iperf");
        return;
    }

    for name in ["iperf", "libiperf"] {
        if pkg_config::Config::new().statik(statik).probe(name).is_ok() {
            return;
        }
    }

    println!(
        "cargo:warning=libiperf not found via pkg-config; falling back to -liperf \
         (install libiperf-dev or set IPERF_LIB_DIR)"
    );
    println!("cargo:rustc-link-lib={kind}=iperf");
}
