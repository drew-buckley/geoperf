use std::env;

#[cfg(not(feature = "iperf-vendored"))]
const MIN_IPERF_VERSION: &str = "3.17";

fn main() {
    #[cfg(feature = "iperf-vendored")]
    build_vendored();

    #[cfg(not(feature = "iperf-vendored"))]
    link_system();
}

#[cfg(feature = "iperf-vendored")]
fn build_vendored() {
    use std::path::PathBuf;

    let src = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap()).join("vendor/iperf");
    if !src.join("configure").exists() {
        panic!("vendor/iperf is missing; run `git submodule update --init`");
    }
    println!("cargo:rerun-if-changed=vendor/iperf/src");

    let dst = autotools::Config::new(&src)
        .disable_shared()
        .enable_static()
        .without("openssl", None)
        .without("sctp", None)
        .env("LDCONFIG", "true")
        .build();

    println!(
        "cargo:rustc-link-search=native={}",
        dst.join("lib").display()
    );
    println!("cargo:rustc-link-lib=static=iperf");
    println!("cargo:rustc-link-lib=m");
}

#[cfg(not(feature = "iperf-vendored"))]
fn link_system() {
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
        let Ok(lib) = pkg_config::Config::new().cargo_metadata(false).probe(name) else {
            continue;
        };
        if !version_at_least(&lib.version, MIN_IPERF_VERSION) {
            panic!(
                "libiperf {} found via pkg-config, but >= {MIN_IPERF_VERSION} is required",
                lib.version
            );
        }
        pkg_config::Config::new()
            .statik(statik)
            .probe(name)
            .unwrap();
        return;
    }

    println!("cargo:warning=libiperf not found via pkg-config; falling back to -liperf");
    println!("cargo:rustc-link-lib={kind}=iperf");
}

#[cfg(not(feature = "iperf-vendored"))]
fn version_at_least(version: &str, min: &str) -> bool {
    let parse = |v: &str| -> Vec<u32> {
        v.split(|c: char| !c.is_ascii_digit())
            .take_while(|s| !s.is_empty())
            .map(|s| s.parse().unwrap_or(0))
            .collect()
    };
    parse(version) >= parse(min)
}
