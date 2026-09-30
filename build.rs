use std::{env, path::{Path, PathBuf}, process::Command};

fn run(mut cmd: Command, name: &str) {
    let status = cmd.status().unwrap_or_else(|e| panic!("cannot run {name}: {e}"));
    assert!(status.success(), "{name} failed: {status}");
}

fn stub(out: &Path) {
    let obj = out.join("stub.o");
    let lib = out.join("libascend_miner_stub.a");
    let mut cc = Command::new(env::var("CC").unwrap_or_else(|_| "cc".into()));
    cc.args(["-std=c11", "-O2", "-fPIC", "-c", "ascend/stub.c", "-o"]).arg(&obj);
    run(cc, "stub compile");
    let mut ar = Command::new(env::var("AR").unwrap_or_else(|_| "ar".into()));
    ar.arg("crus").arg(&lib).arg(&obj);
    run(ar, "stub archive");
    println!("cargo:rustc-link-search=native={}", out.display());
    println!("cargo:rustc-link-lib=static=ascend_miner_stub");
}

fn cann(out: &Path) {
    let build = out.join("cann");
    std::fs::create_dir_all(&build).expect("create CANN build dir");
    let modules = env::var("ASC_MODULES").expect("ASC_MODULES is required; use `make build`");
    let arch = env::var("ASCEND_ARCH").unwrap_or_else(|_| "dav-2201".into());
    let src = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap()).join("ascend");

    let mut cmake = Command::new("cmake");
    cmake.current_dir(&build)
        .arg(format!("-DCMAKE_MODULE_PATH={modules}"))
        .arg(format!("-DCMAKE_ASC_ARCHITECTURES={arch}"))
        .arg("-DCMAKE_ASC_RUN_MODE=npu")
        .arg(src);
    run(cmake, "CANN configure");

    let mut compile = Command::new("cmake");
    compile.current_dir(&build).args(["--build", ".", "--parallel"]);
    run(compile, "CANN build");

    println!("cargo:rustc-link-search=native={}", build.display());
    println!("cargo:rustc-link-lib=dylib=ascend_miner");
    println!("cargo:rustc-link-arg=-Wl,-rpath,{}", build.display());
}

fn main() {
    for p in ["ascend/stub.c", "ascend/miner.asc", "ascend/CMakeLists.txt"] {
        println!("cargo:rerun-if-changed={p}");
    }
    println!("cargo:rerun-if-env-changed=ASC_MODULES");
    println!("cargo:rerun-if-env-changed=ASCEND_ARCH");

    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let use_stub = env::var_os("CARGO_FEATURE_STUB").is_some();
    let use_cann = env::var_os("CARGO_FEATURE_CANN").is_some();
    assert!(use_stub ^ use_cann, "enable exactly one of `stub` or `cann`");
    if use_cann { cann(&out) } else { stub(&out) }
}
