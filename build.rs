//! `include/bydev.h`를 C ABI(src/lib.rs)에서 만든다 — 내용이 바뀔 때만 쓴다. 머리 파일은 저장소에 두고, `sdk:check c`가 최신인지 본다.

fn main() {
    println!("cargo:rerun-if-changed=src/lib.rs");
    println!("cargo:rerun-if-changed=cbindgen.toml");
    let dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR");
    let config = cbindgen::Config::from_file(format!("{dir}/cbindgen.toml")).expect("cbindgen.toml");
    let bindings =
        cbindgen::Builder::new().with_crate(&dir).with_config(config).generate().expect("C 머리 파일을 만들지 못했다");
    let mut out = Vec::new();
    bindings.write(&mut out);
    let path = format!("{dir}/include/bydev.h");
    if std::fs::read(&path).ok().as_deref() != Some(out.as_slice()) {
        std::fs::write(&path, out).expect("include/bydev.h");
    }
}
