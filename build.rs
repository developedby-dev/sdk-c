//! `include/bydev.h`를 C ABI(src/lib.rs)에서 만든다 — 내용이 바뀔 때만 쓴다. 머리 파일은 저장소에 두고, `sdk:check c`가 최신인지 본다.

fn main() {
    println!("cargo:rerun-if-changed=src/lib.rs");
    println!("cargo:rerun-if-changed=cbindgen.toml");
    // macOS 동적 라이브러리의 설치 이름 — 정하지 않으면 지은 곳의 절대 경로(CI면 /Users/runner/…)가 박혀, 이 라이브러리로 링크한 앱이 실행할 때
    // 「Library not loaded」로 멈춘다(2026-11-21 0.1.0 출시물에서 찾음). @rpath면 앱이 자기 rpath(예: @executable_path/../Frameworks)에서 찾는다
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        println!("cargo:rustc-cdylib-link-arg=-Wl,-install_name,@rpath/libbydev_c.dylib");
    }
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
