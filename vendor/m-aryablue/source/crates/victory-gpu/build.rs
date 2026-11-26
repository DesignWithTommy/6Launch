use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-env-changed=VICTORY_DLSS_SDK");
    // only the dlss feature links anything; default builds stay as they were
    if std::env::var_os("CARGO_FEATURE_DLSS").is_none() {
        return;
    }
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let sdk = std::env::var_os("VICTORY_DLSS_SDK").map(PathBuf::from).unwrap_or_else(|| {
        panic!("feature dlss needs VICTORY_DLSS_SDK = a clone of https://github.com/NVIDIA/DLSS")
    });
    let lib = sdk.join("lib").join("Windows_x86_64").join("x64");
    if !lib.join("nvsdk_ngx_d.lib").is_file() {
        panic!("no nvsdk_ngx_d.lib under {}", lib.display());
    }
    // nvidia's ngx sdk (dlss sdk), dynamic-crt flavour to match rust's /MD
    println!("cargo:rustc-link-search=native={}", lib.display());
    println!("cargo:rustc-link-lib=static=nvsdk_ngx_d");
    for sys in ["advapi32", "version", "shell32", "ole32", "user32", "dxgi", "d3d12", "d3d11", "setupapi", "crypt32", "wintrust"] {
        println!("cargo:rustc-link-lib=dylib={sys}");
    }
    println!("cargo:rustc-env=VICTORY_DLSS_SDK_BUILT={}", sdk.display());
}
