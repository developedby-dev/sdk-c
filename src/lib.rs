//! bydev C SDK — Rust SDK(`sdks/rust`) 위의 C ABI. 머리 파일 `include/bydev.h`는 이 파일에서 cbindgen이 만든다.
//!
//! 안전 계약(모든 `unsafe extern "C"` 함수): 포인터 인자는 NULL이거나 유효해야 한다 — 핸들은 이 라이브러리가 준 것, 문자열은 NUL로 끝나는 UTF-8.
//! 모델(이용권 · 기기 · 업데이트 · 클레임 · 세션 상태)은 JSON 문자열로 돌려준다 — 필드가 더해져도 ABI가 깨지지 않는다(불변식 8).

#![allow(unsafe_code, non_camel_case_types, clippy::missing_safety_doc)]

use std::cell::Cell;
use std::collections::{BTreeMap, HashMap};
use std::ffi::{CStr, CString, c_char, c_void};
use std::sync::{Arc, Condvar, LazyLock, Mutex};
use std::time::Duration;

use bydev::{
    AppInfo, BoxError, Cancel, Client, Clock, Config, CredentialRecord, CredentialStore, DeviceInfo, Error,
    FingerprintSource, HttpRequest, HttpResponse, Logger, LoginAttempt, OsFingerprintSource, Ports, Random, Scheduler,
    ServerVerifier, Session, SessionState, Signal, Subscription, Task, Transport, codes,
};

const USER_AGENT: &str = concat!("bydev-c/", env!("CARGO_PKG_VERSION"));

// ── 핸들 ──────────────────────────────────────────────────────

/// 클라이언트 — `bydev_client_new`로 만들고 `bydev_client_free`로 놓는다.
pub struct bydev_client {
    inner: Client,
}

/// 진행 중인 디바이스 로그인 — `bydev_login_free`로 놓는다(놓아도 취소하지 않는다 — `bydev_login_cancel`).
pub struct bydev_login {
    inner: LoginAttempt,
    user_code: CString,
    verification_uri: CString,
    verification_uri_complete: CString,
    expires_at: CString,
}

/// 세션 핸들 — 같은 클라이언트의 세션은 하나다. `bydev_session_free`는 핸들만 놓는다(멈추려면 `bydev_session_stop`).
pub struct bydev_session {
    inner: Session,
}

/// 에러 — `bydev_error_free`로 놓는다.
pub struct bydev_error {
    inner: Error,
    code: CString,
    message: CString,
    correlation_id: Option<CString>,
}

/// 예약 작업 — 스케줄러 콜백이 받는다. 때가 되면 `bydev_task_run`(한 번, 놓기까지), 취소됐으면 `bydev_task_free`.
pub struct bydev_task {
    task: Task,
}

/// 기다림의 신호 — `scheduler_wait` 콜백이 받는다. 그 호출 동안만 유효하다.
pub struct bydev_signal {
    signal: Signal,
}

/// 판매자 서버의 토큰 검증기 — `bydev_server_verifier_free`로 놓는다.
pub struct bydev_server_verifier {
    inner: ServerVerifier,
}

// ── 문자열 · 에러 도우미 ───────────────────────────────────────

fn c_string(s: &str) -> CString {
    CString::new(s.replace('\0', "\u{FFFD}")).unwrap_or_default()
}

fn out_string(s: &str) -> *mut c_char {
    c_string(s).into_raw()
}

unsafe fn text<'a>(p: *const c_char) -> Option<&'a str> {
    if p.is_null() {
        return None;
    }
    unsafe { CStr::from_ptr(p) }.to_str().ok()
}

/// 콜백이 넘긴 문자열(`bydev_string_new`로 만든 것)을 받아 놓는다.
unsafe fn take(p: *mut c_char) -> Option<String> {
    if p.is_null() {
        return None;
    }
    let owned = unsafe { CString::from_raw(p) };
    Some(owned.to_string_lossy().into_owned())
}

fn set_error(out: *mut *mut bydev_error, e: Error) {
    if out.is_null() {
        return;
    }
    let handle = bydev_error {
        code: c_string(e.code()),
        message: c_string(e.message()),
        correlation_id: e.correlation_id().map(c_string),
        inner: e,
    };
    unsafe { *out = Box::into_raw(Box::new(handle)) };
}

fn config_error(message: &str) -> Error {
    Error::from_code(codes::CONFIGURATION, message)
}

fn json_out<T: serde_like::ToJson>(result: Result<T, Error>, error: *mut *mut bydev_error) -> *mut c_char {
    match result {
        Ok(value) => out_string(&value.to_json()),
        Err(e) => {
            set_error(error, e);
            std::ptr::null_mut()
        }
    }
}

mod serde_like {
    /// 모델 → JSON. SDK 모델은 모두 serde로 직렬화된다.
    pub trait ToJson {
        fn to_json(&self) -> String;
    }

    impl<T: serde::Serialize> ToJson for T {
        fn to_json(&self) -> String {
            serde_json::to_string(self).unwrap_or_else(|_| "null".to_owned())
        }
    }
}

// ── 문자열 · 작업 · 신호 ──────────────────────────────────────

/// 콜백이 SDK에 넘길 문자열을 만든다(SDK가 놓는다). `s`가 NULL이면 NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bydev_string_new(s: *const c_char) -> *mut c_char {
    match unsafe { text(s) } {
        Some(t) => out_string(t),
        None if s.is_null() => std::ptr::null_mut(),
        None => out_string(&unsafe { CStr::from_ptr(s) }.to_string_lossy()),
    }
}

/// SDK가 돌려준 문자열을 놓는다. NULL이면 아무것도 하지 않는다.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bydev_string_free(s: *mut c_char) {
    if !s.is_null() {
        drop(unsafe { CString::from_raw(s) });
    }
}

/// 예약 작업을 돌리고 놓는다.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bydev_task_run(task: *mut bydev_task) {
    if !task.is_null() {
        let task = unsafe { Box::from_raw(task) };
        (task.task)();
    }
}

/// 예약 작업을 돌리지 않고 놓는다(취소된 예약).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bydev_task_free(task: *mut bydev_task) {
    if !task.is_null() {
        drop(unsafe { Box::from_raw(task) });
    }
}

/// 신호가 켜질 때까지 막는다.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bydev_signal_block(signal: *const bydev_signal) {
    if let Some(s) = unsafe { signal.as_ref() } {
        s.signal.block();
    }
}

/// 신호가 켜졌는지다.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bydev_signal_is_fired(signal: *const bydev_signal) -> bool {
    unsafe { signal.as_ref() }.is_some_and(|s| s.signal.is_fired())
}

// ── 에러 ─────────────────────────────────────────────────────

/// 에러 코드(`BYDEV_CODE_*`와 strcmp). 에러를 놓기 전까지 유효하다.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bydev_error_code(error: *const bydev_error) -> *const c_char {
    unsafe { error.as_ref() }.map_or(std::ptr::null(), |e| e.code.as_ptr())
}

/// 보여 주기만 하는 설명(한국어).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bydev_error_message(error: *const bydev_error) -> *const c_char {
    unsafe { error.as_ref() }.map_or(std::ptr::null(), |e| e.message.as_ptr())
}

/// 문의용 상관 ID — 없으면 NULL. 오류 화면에 보인다.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bydev_error_correlation_id(error: *const bydev_error) -> *const c_char {
    unsafe { error.as_ref() }.and_then(|e| e.correlation_id.as_ref()).map_or(std::ptr::null(), |c| c.as_ptr())
}

/// HTTP 상태 — 없으면 0.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bydev_error_http_status(error: *const bydev_error) -> u16 {
    unsafe { error.as_ref() }.and_then(|e| e.inner.http_status()).unwrap_or(0)
}

/// 다시 시도할 때까지 기다릴 초 — 없으면 0.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bydev_error_retry_after_sec(error: *const bydev_error) -> u64 {
    unsafe { error.as_ref() }.and_then(|e| e.inner.retry_after_sec()).unwrap_or(0)
}

/// 일시적 실패인지(명세 3.4) — 참이면 다시 해 볼 만하다.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bydev_error_is_transient(error: *const bydev_error) -> bool {
    unsafe { error.as_ref() }.is_some_and(|e| e.inner.is_transient())
}

/// 에러 전체(`code` · `message` · `httpStatus` · `correlationId` · `retryAfterSec` · `details`) JSON. `bydev_string_free`로 놓는다.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bydev_error_json(error: *const bydev_error) -> *mut c_char {
    let Some(e) = (unsafe { error.as_ref() }) else { return std::ptr::null_mut() };
    let e = &e.inner;
    let value = serde_json::json!({
        "code": e.code(), "message": e.message(), "httpStatus": e.http_status(), "correlationId": e.correlation_id(),
        "retryAfterSec": e.retry_after_sec(), "details": e.details(),
    });
    out_string(&value.to_string())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn bydev_error_free(error: *mut bydev_error) {
    if !error.is_null() {
        drop(unsafe { Box::from_raw(error) });
    }
}

// ── 판 ───────────────────────────────────────────────────────

/// SDK 판(이 라이브러리 `Cargo.toml`의 판 — 예: `1.0.0`).
#[unsafe(no_mangle)]
pub extern "C" fn bydev_version() -> *const c_char {
    concat!(env!("CARGO_PKG_VERSION"), "\0").as_ptr().cast()
}

/// 따르는 동작 명세 판(`BYDEV_SPEC_VERSION`과 같다).
#[unsafe(no_mangle)]
pub extern "C" fn bydev_spec_version() -> *const c_char {
    static SPEC: LazyLock<CString> = LazyLock::new(|| c_string(bydev::SPEC_VERSION));
    SPEC.as_ptr()
}

// ── 포트 ─────────────────────────────────────────────────────

/// 바꿔 끼울 포트(명세 2절) — 모두 선택이다. 묶음(전송 · 저장소 셋 · 시계 · 스케줄러 넷 · 난수 둘 · 지문 · 로그)마다 다 채우면 그 묶음을 쓰고, 아니면
/// 기본 구현이다. 콜백은 SDK의 스레드에서 불린다. 콜백이 돌려주는 문자열은 `bydev_string_new`로 만든다. `error`에 메시지를 넣으면 실패다.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct bydev_ports {
    pub user_data: *mut c_void,
    /// 요청 JSON `{"method","url","headers":{},"body":문자열|null}` → 응답 JSON `{"status","headers":{},"body":문자열}`. 연결하지 못하면 NULL + `*error`.
    pub transport_send: Option<unsafe extern "C" fn(*mut c_void, *const c_char, *mut *mut c_char) -> *mut c_char>,
    /// 기록 JSON(명세 3.6) 또는 NULL(없음).
    pub store_load: Option<unsafe extern "C" fn(*mut c_void, *mut *mut c_char) -> *mut c_char>,
    pub store_save: Option<unsafe extern "C" fn(*mut c_void, *const c_char, *mut *mut c_char) -> bool>,
    pub store_clear: Option<unsafe extern "C" fn(*mut c_void, *mut *mut c_char) -> bool>,
    /// 지금 시각(Unix 밀리초).
    pub clock_now_ms: Option<unsafe extern "C" fn(*mut c_void) -> i64>,
    /// `delay_ms` 뒤에 `bydev_task_run(task)` — 예약 ID를 돌려준다.
    pub scheduler_after: Option<unsafe extern "C" fn(*mut c_void, u64, *mut bydev_task) -> u64>,
    /// 예약 취소 — 그 작업은 돌리지 않고 `bydev_task_free`.
    pub scheduler_cancel: Option<unsafe extern "C" fn(*mut c_void, u64)>,
    pub scheduler_sleep: Option<unsafe extern "C" fn(*mut c_void, u64)>,
    /// 신호가 켜질 때까지 기다린다(`bydev_signal_block`).
    pub scheduler_wait: Option<unsafe extern "C" fn(*mut c_void, *const bydev_signal)>,
    pub random_float: Option<unsafe extern "C" fn(*mut c_void) -> f64>,
    pub random_uuid: Option<unsafe extern "C" fn(*mut c_void) -> *mut c_char>,
    /// 기기 식별자 원본 또는 NULL — 원본은 SDK 밖으로 나가지 않는다(해시만).
    pub fingerprint_read: Option<unsafe extern "C" fn(*mut c_void, *mut *mut c_char) -> *mut c_char>,
    /// 0 debug · 1 info · 2 warn. 토큰 · 지문 원본은 넘어오지 않는다.
    pub log: Option<unsafe extern "C" fn(*mut c_void, i32, *const c_char)>,
}

#[derive(Clone, Copy)]
struct Raw(bydev_ports);

// 콜백 표와 user_data를 여러 스레드에서 쓴다 — 스레드 안전은 앱의 계약이다(머리 파일 머리말)
unsafe impl Send for Raw {}
unsafe impl Sync for Raw {}

fn port_error(error: *mut c_char, fallback: &str) -> BoxError {
    unsafe { take(error) }.unwrap_or_else(|| fallback.to_owned()).into()
}

impl Transport for Raw {
    fn send(&self, request: HttpRequest) -> Result<HttpResponse, BoxError> {
        let body = request.body.map(|b| String::from_utf8_lossy(&b).into_owned());
        let json = serde_json::json!({ "method": request.method, "url": request.url, "headers": request.headers, "body": body });
        let input = c_string(&json.to_string());
        let mut error = std::ptr::null_mut();
        let send = self.0.transport_send.expect("묶음 확인");
        let out = unsafe { take(send(self.0.user_data, input.as_ptr(), &mut error)) };
        let Some(out) = out else { return Err(port_error(error, "전송 콜백이 실패했다")) };
        unsafe { bydev_string_free(error) };
        let value: serde_json::Value = serde_json::from_str(&out)?;
        let headers: BTreeMap<String, String> = value["headers"]
            .as_object()
            .map(|h| h.iter().map(|(k, v)| (k.to_ascii_lowercase(), v.as_str().unwrap_or("").to_owned())).collect())
            .unwrap_or_default();
        Ok(HttpResponse {
            status: value["status"].as_u64().unwrap_or(0) as u16,
            headers,
            body: value["body"].as_str().unwrap_or("").as_bytes().to_vec(),
        })
    }
}

impl CredentialStore for Raw {
    fn load(&self) -> Result<Option<CredentialRecord>, BoxError> {
        let mut error = std::ptr::null_mut();
        let out = unsafe { take((self.0.store_load.expect("묶음 확인"))(self.0.user_data, &mut error)) };
        if !error.is_null() {
            return Err(port_error(error, "저장소 읽기 실패"));
        }
        match out {
            None => Ok(None),
            Some(text) => Ok(serde_json::from_str(&text).ok()),
        }
    }

    fn save(&self, record: &CredentialRecord) -> Result<(), BoxError> {
        let input = c_string(&serde_json::to_string(record)?);
        let mut error = std::ptr::null_mut();
        if unsafe { (self.0.store_save.expect("묶음 확인"))(self.0.user_data, input.as_ptr(), &mut error) } {
            unsafe { bydev_string_free(error) };
            Ok(())
        } else {
            Err(port_error(error, "저장소 쓰기 실패"))
        }
    }

    fn clear(&self) -> Result<(), BoxError> {
        let mut error = std::ptr::null_mut();
        if unsafe { (self.0.store_clear.expect("묶음 확인"))(self.0.user_data, &mut error) } {
            unsafe { bydev_string_free(error) };
            Ok(())
        } else {
            Err(port_error(error, "저장소 지우기 실패"))
        }
    }
}

impl Clock for Raw {
    fn now_ms(&self) -> i64 {
        unsafe { (self.0.clock_now_ms.expect("묶음 확인"))(self.0.user_data) }
    }
}

impl Scheduler for Raw {
    fn after(&self, delay: Duration, task: Task) -> Cancel {
        let handle = Box::into_raw(Box::new(bydev_task { task }));
        let id = unsafe {
            (self.0.scheduler_after.expect("묶음 확인"))(self.0.user_data, delay.as_millis() as u64, handle)
        };
        let raw = *self;
        Cancel::new(move || {
            // 필드만 따로 잡히지 않게(에디션 2024의 나눠 잡기) — Send인 Raw 통째로
            let raw = raw;
            unsafe { (raw.0.scheduler_cancel.expect("묶음 확인"))(raw.0.user_data, id) }
        })
    }

    fn sleep(&self, delay: Duration) {
        unsafe { (self.0.scheduler_sleep.expect("묶음 확인"))(self.0.user_data, delay.as_millis() as u64) }
    }

    fn wait(&self, done: &Signal) {
        let signal = bydev_signal { signal: done.clone() };
        unsafe { (self.0.scheduler_wait.expect("묶음 확인"))(self.0.user_data, &signal) }
    }
}

impl Random for Raw {
    fn float(&self) -> f64 {
        unsafe { (self.0.random_float.expect("묶음 확인"))(self.0.user_data) }
    }

    fn uuid(&self) -> String {
        unsafe { take((self.0.random_uuid.expect("묶음 확인"))(self.0.user_data)) }.unwrap_or_default()
    }
}

impl FingerprintSource for Raw {
    fn read(&self) -> Result<Option<String>, BoxError> {
        let mut error = std::ptr::null_mut();
        let out = unsafe { take((self.0.fingerprint_read.expect("묶음 확인"))(self.0.user_data, &mut error)) };
        if !error.is_null() {
            return Err(port_error(error, "지문 출처를 읽지 못했다"));
        }
        Ok(out)
    }
}

impl Logger for Raw {
    fn debug(&self, message: &str) {
        self.log(0, message);
    }
    fn info(&self, message: &str) {
        self.log(1, message);
    }
    fn warn(&self, message: &str) {
        self.log(2, message);
    }
}

impl Raw {
    fn log(&self, level: i32, message: &str) {
        let text = c_string(message);
        unsafe { (self.0.log.expect("묶음 확인"))(self.0.user_data, level, text.as_ptr()) }
    }

    fn ports(self, app_name: Option<&str>, client_id: &str) -> Ports {
        let p = &self.0;
        let me = Arc::new(self);
        let mut ports = Ports::default();
        if p.transport_send.is_some() {
            ports.transport = Some(me.clone());
        }
        if p.store_load.is_some() && p.store_save.is_some() && p.store_clear.is_some() {
            ports.store = Some(me.clone());
        }
        if p.clock_now_ms.is_some() {
            ports.clock = Some(me.clone());
        }
        if p.scheduler_after.is_some()
            && p.scheduler_cancel.is_some()
            && p.scheduler_sleep.is_some()
            && p.scheduler_wait.is_some()
        {
            ports.scheduler = Some(me.clone());
        }
        if p.random_float.is_some() && p.random_uuid.is_some() {
            ports.random = Some(me.clone());
        }
        if p.fingerprint_read.is_some() {
            ports.fingerprint = Some(me.clone());
        }
        if p.log.is_some() {
            ports.logger = Some(me.clone());
        }
        if let Some(app) = app_name {
            if ports.store.is_none() {
                ports.store = Some(bydev::default_credential_store(app, client_id, ports.logger.as_deref()));
            }
            if ports.fingerprint.is_none() {
                ports.fingerprint =
                    Some(Arc::new(OsFingerprintSource::new(Some(bydev::app_data_dir(app).join("bydev-install-id")))));
            }
        }
        ports
    }
}

// ── 클라이언트 ────────────────────────────────────────────────

/// 설정(명세 1절).
#[repr(C)]
pub struct bydev_config {
    pub base_url: *const c_char,
    pub client_id: *const c_char,
    pub product_id: *const c_char,
    /// 공백으로 나눈 스코프(`"entitlement.read session.write"`).
    pub scopes: *const c_char,
    /// NULL이면 호스트 이름.
    pub device_name: *const c_char,
    /// NULL이면 이 운영체제(`BYDEV_DEVICE_PLATFORM_*`).
    pub device_platform: *const c_char,
    /// NULL이 아니면 기본 저장소(운영체제 보안 저장소)와 지문 출처(앱 폴더의 설치 ID)를 쓴다 — 포트로 준 것이 먼저다.
    pub app_name: *const c_char,
    /// NULL이면 모두 기본 구현(저장소는 메모리 — `app_name`을 주지 않으면 앱을 끄면 로그인이 사라진다).
    pub ports: *const bydev_ports,
    /// 실행 중인 앱의 배포 대상(`BYDEV_DISTRIBUTION_TARGET_*` — 업데이트 확인의 `target`과 같은 값). 세션을 쓰면 `app_version`과 함께
    /// 준다(명세 1절 · ADR-0049) — 세션 획득에 싣는다. 업데이트 제공 기한이 있는 영구 이용권은 이 값이 없으면 획득이
    /// `ENTITLEMENT_VERSION_NOT_COVERED`로 거절된다. 둘 다 NULL이면 싣지 않고, 하나만 주면 `CONFIGURATION`.
    pub app_target: *const c_char,
    /// 콘솔에 업로드한 빌드의 버전 문자열(`"1.4.0"`).
    pub app_version: *const c_char,
}

/// 클라이언트를 만든다. 실패하면 NULL + `*error`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bydev_client_new(
    config: *const bydev_config,
    error: *mut *mut bydev_error,
) -> *mut bydev_client {
    let Some(c) = (unsafe { config.as_ref() }) else {
        set_error(error, config_error("설정이 NULL이다"));
        return std::ptr::null_mut();
    };
    let s = |p| unsafe { text(p) }.unwrap_or("").to_owned();
    let client_id = s(c.client_id);
    let system = bydev::system_device_info();
    let device = DeviceInfo {
        name: unsafe { text(c.device_name) }.map(str::to_owned).unwrap_or(system.name),
        platform: unsafe { text(c.device_platform) }.map(str::to_owned).unwrap_or(system.platform),
    };
    let app = match (unsafe { text(c.app_target) }, unsafe { text(c.app_version) }) {
        (Some(target), Some(version)) => Some(AppInfo { target: target.to_owned(), version: version.to_owned() }),
        (None, None) if c.app_target.is_null() && c.app_version.is_null() => None,
        _ => {
            set_error(error, config_error("app_target과 app_version은 둘 다 UTF-8 문자열로 주거나 둘 다 NULL로 둔다"));
            return std::ptr::null_mut();
        }
    };
    let raw = unsafe { c.ports.as_ref() }.copied().unwrap_or(bydev_ports {
        user_data: std::ptr::null_mut(),
        transport_send: None,
        store_load: None,
        store_save: None,
        store_clear: None,
        clock_now_ms: None,
        scheduler_after: None,
        scheduler_cancel: None,
        scheduler_sleep: None,
        scheduler_wait: None,
        random_float: None,
        random_uuid: None,
        fingerprint_read: None,
        log: None,
    });
    let config = Config {
        base_url: s(c.base_url),
        client_id: client_id.clone(),
        product_id: s(c.product_id),
        scopes: s(c.scopes).split_whitespace().map(str::to_owned).collect(),
        device: Some(device),
        app,
        ports: Raw(raw).ports(unsafe { text(c.app_name) }, &client_id),
    };
    match Client::with_user_agent(config, Some(USER_AGENT)) {
        Ok(inner) => Box::into_raw(Box::new(bydev_client { inner })),
        Err(e) => {
            set_error(error, e);
            std::ptr::null_mut()
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn bydev_client_free(client: *mut bydev_client) {
    if !client.is_null() {
        drop(unsafe { Box::from_raw(client) });
    }
}

unsafe fn client<'a>(client: *const bydev_client, error: *mut *mut bydev_error) -> Option<&'a Client> {
    let found = unsafe { client.as_ref() }.map(|c| &c.inner);
    if found.is_none() {
        set_error(error, config_error("클라이언트가 NULL이다"));
    }
    found
}

/// 디바이스 로그인을 시작한다(명세 5절). 화면에 `bydev_login_user_code` · `bydev_login_verification_uri_complete`(QR)를 보이고 `bydev_login_wait`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bydev_client_start_login(
    c: *const bydev_client,
    error: *mut *mut bydev_error,
) -> *mut bydev_login {
    let Some(c) = (unsafe { client(c, error) }) else { return std::ptr::null_mut() };
    match c.start_login() {
        Ok(attempt) => Box::into_raw(Box::new(bydev_login {
            user_code: c_string(&attempt.user_code),
            verification_uri: c_string(&attempt.verification_uri),
            verification_uri_complete: c_string(&attempt.verification_uri_complete),
            expires_at: c_string(&attempt.expires_at),
            inner: attempt,
        })),
        Err(e) => {
            set_error(error, e);
            std::ptr::null_mut()
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn bydev_login_user_code(login: *const bydev_login) -> *const c_char {
    unsafe { login.as_ref() }.map_or(std::ptr::null(), |l| l.user_code.as_ptr())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn bydev_login_verification_uri(login: *const bydev_login) -> *const c_char {
    unsafe { login.as_ref() }.map_or(std::ptr::null(), |l| l.verification_uri.as_ptr())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn bydev_login_verification_uri_complete(login: *const bydev_login) -> *const c_char {
    unsafe { login.as_ref() }.map_or(std::ptr::null(), |l| l.verification_uri_complete.as_ptr())
}

/// 코드가 만료되는 시각(ISO 8601).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bydev_login_expires_at(login: *const bydev_login) -> *const c_char {
    unsafe { login.as_ref() }.map_or(std::ptr::null(), |l| l.expires_at.as_ptr())
}

/// 로그인이 끝나기를 기다린다. 성공하면 받은 스코프(`bydev_string_free`), 실패하면 NULL + `*error`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bydev_login_wait(login: *const bydev_login, error: *mut *mut bydev_error) -> *mut c_char {
    let Some(l) = (unsafe { login.as_ref() }) else { return std::ptr::null_mut() };
    match l.inner.wait() {
        Ok(result) => out_string(&result.scope),
        Err(e) => {
            set_error(error, e);
            std::ptr::null_mut()
        }
    }
}

/// 로그인을 취소한다 — `bydev_login_wait`는 `LOGIN_CANCELLED`로 끝난다.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bydev_login_cancel(login: *const bydev_login) {
    if let Some(l) = unsafe { login.as_ref() } {
        l.inner.cancel();
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn bydev_login_free(login: *mut bydev_login) {
    if !login.is_null() {
        drop(unsafe { Box::from_raw(login) });
    }
}

/// 저장된 로그인이 있으면 1, 없으면 0, 실패면 -1 + `*error` — 이용 가능 여부가 아니다.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bydev_client_has_credentials(c: *const bydev_client, error: *mut *mut bydev_error) -> i32 {
    let Some(c) = (unsafe { client(c, error) }) else { return -1 };
    match c.has_credentials() {
        Ok(found) => i32::from(found),
        Err(e) => {
            set_error(error, e);
            -1
        }
    }
}

/// 이 제품의 이용권 JSON. `updateCoverage.type`(`BYDEV_UPDATE_COVERAGE_TYPE_*`)이 `UNTIL`이면 `until` 전에 출시된 버전까지 실행할 수 있다.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bydev_client_entitlement(c: *const bydev_client, error: *mut *mut bydev_error) -> *mut c_char {
    let Some(c) = (unsafe { client(c, error) }) else { return std::ptr::null_mut() };
    json_out(c.entitlement(), error)
}

/// 등록된 기기 한 쪽 JSON(`items` · `nextCursor`). `cursor`는 NULL이면 첫 쪽, `limit` 0이면 서버 기본.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bydev_client_devices(
    c: *const bydev_client,
    cursor: *const c_char,
    limit: u32,
    error: *mut *mut bydev_error,
) -> *mut c_char {
    let Some(c) = (unsafe { client(c, error) }) else { return std::ptr::null_mut() };
    json_out(c.devices(unsafe { text(cursor) }, (limit > 0).then_some(limit)), error)
}

/// 기기를 해제한다(월 횟수 제한) — 결과 JSON.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bydev_client_release_device(
    c: *const bydev_client,
    device_id: *const c_char,
    error: *mut *mut bydev_error,
) -> *mut c_char {
    let Some(c) = (unsafe { client(c, error) }) else { return std::ptr::null_mut() };
    json_out(c.release_device(unsafe { text(device_id) }.unwrap_or("")), error)
}

/// 닉네임과 가명 식별자 JSON.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bydev_client_profile(c: *const bydev_client, error: *mut *mut bydev_error) -> *mut c_char {
    let Some(c) = (unsafe { client(c, error) }) else { return std::ptr::null_mut() };
    json_out(c.profile(), error)
}

/// 업데이트 확인 JSON. `entitled`가 있으면(업데이트 제공 기한이 있는 구매자) `latest` · `update` · `install` 대신 `entitled.release`를
/// 따른다 — 범위 안 버전이 없으면 null. `update.status`가 `REQUIRED`면 업데이트 전까지 실행을 막는다. 받은 파일은 설치 전에
/// `bydev_client_verify_download`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bydev_client_check_for_update(
    c: *const bydev_client,
    target: *const c_char,
    current_version: *const c_char,
    error: *mut *mut bydev_error,
) -> *mut c_char {
    let Some(c) = (unsafe { client(c, error) }) else { return std::ptr::null_mut() };
    json_out(
        c.check_for_update(unsafe { text(target) }.unwrap_or(""), unsafe { text(current_version) }.unwrap_or("")),
        error,
    )
}

/// 내려받은 파일의 SHA-256을 확인한다 — 다르면 false + `DOWNLOAD_INTEGRITY`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bydev_client_verify_download(
    c: *const bydev_client,
    data: *const u8,
    len: usize,
    sha256: *const c_char,
    error: *mut *mut bydev_error,
) -> bool {
    let Some(c) = (unsafe { client(c, error) }) else { return false };
    let bytes = if data.is_null() { &[][..] } else { unsafe { std::slice::from_raw_parts(data, len) } };
    match c.verify_download(bytes, unsafe { text(sha256) }.unwrap_or("")) {
        Ok(()) => true,
        Err(e) => {
            set_error(error, e);
            false
        }
    }
}

/// licenseToken을 검증한다(명세 9절) — 클레임 JSON. `build_id`는 서버가 앱의 대상 · 버전으로 빌드를 확정했을 때만 있다.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bydev_client_verify_token(
    c: *const bydev_client,
    token: *const c_char,
    error: *mut *mut bydev_error,
) -> *mut c_char {
    let Some(c) = (unsafe { client(c, error) }) else { return std::ptr::null_mut() };
    json_out(c.verify_token(unsafe { text(token) }.unwrap_or("")), error)
}

/// 세션을 멈추고 폐기 요청을 한 번 보낸 뒤 기록을 지운다. 서버가 폐기했으면 1, 아니면 0, 실패면 -1 + `*error`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bydev_client_logout(c: *const bydev_client, error: *mut *mut bydev_error) -> i32 {
    let Some(c) = (unsafe { client(c, error) }) else { return -1 };
    match c.logout() {
        Ok(revoked) => i32::from(revoked),
        Err(e) => {
            set_error(error, e);
            -1
        }
    }
}

// ── 세션 ─────────────────────────────────────────────────────

/// 이 클라이언트의 세션 핸들.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bydev_client_session(
    c: *const bydev_client,
    error: *mut *mut bydev_error,
) -> *mut bydev_session {
    let Some(c) = (unsafe { client(c, error) }) else { return std::ptr::null_mut() };
    match c.session() {
        Ok(inner) => Box::into_raw(Box::new(bydev_session { inner })),
        Err(e) => {
            set_error(error, e);
            std::ptr::null_mut()
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn bydev_session_free(session: *mut bydev_session) {
    if !session.is_null() {
        drop(unsafe { Box::from_raw(session) });
    }
}

/// `Stopped`에서만 의미가 있다(8.2절).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bydev_session_start(session: *const bydev_session) {
    if let Some(s) = unsafe { session.as_ref() } {
        s.inner.start();
    }
}

/// `NeedsDecision`(다른 기기 · 같은 기기 실행 중) 또는 `Locked(SESSION_SUPERSEDED)`에서 인수한다.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bydev_session_take_over(session: *const bydev_session) {
    if let Some(s) = unsafe { session.as_ref() } {
        s.inner.take_over();
    }
}

/// `NeedsDecision`(기기 수) 또는 스스로 풀리지 않는 `Locked`에서 다시 얻는다.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bydev_session_retry(session: *const bydev_session) {
    if let Some(s) = unsafe { session.as_ref() } {
        s.inner.retry();
    }
}

/// 예약을 모두 취소하고 반납을 한 번 보낸다(8.7절).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bydev_session_stop(session: *const bydev_session) {
    if let Some(s) = unsafe { session.as_ref() } {
        s.inner.stop();
    }
}

fn state_type(state: &SessionState) -> i32 {
    let name = match state {
        SessionState::Starting { .. } => bydev::session_state_type::STARTING,
        SessionState::Active(_) => bydev::session_state_type::ACTIVE,
        SessionState::Degraded(_) => bydev::session_state_type::DEGRADED,
        SessionState::NeedsDecision(_) => bydev::session_state_type::NEEDS_DECISION,
        SessionState::Locked(_) => bydev::session_state_type::LOCKED,
        SessionState::Stopped => bydev::session_state_type::STOPPED,
    };
    bydev::session_state_type::KNOWN.iter().position(|k| *k == name).map_or(-1, |i| i as i32)
}

/// 지금 상태 JSON(`{"type": "Active", …}` — 명세 8.1절).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bydev_session_state(session: *const bydev_session) -> *mut c_char {
    unsafe { session.as_ref() }
        .map_or(std::ptr::null_mut(), |s| out_string(&serde_json::to_string(&s.inner.state()).unwrap_or_default()))
}

/// 지금 상태의 종류(`bydev_state_type` — `BYDEV_STATE_ACTIVE` 등).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bydev_session_state_type(session: *const bydev_session) -> i32 {
    unsafe { session.as_ref() }.map_or(-1, |s| state_type(&s.inner.state()))
}

/// 상태 콜백 — `state_type`과 상태 JSON(그 호출 동안만 유효).
pub type bydev_state_callback = Option<unsafe extern "C" fn(*mut c_void, i32, *const c_char)>;

/// 구독 하나 — 해제가 돌아온 뒤로는 콜백이 돌지 않게 진행 중인 호출을 센다.
struct Listener {
    callback: unsafe extern "C" fn(*mut c_void, i32, *const c_char),
    user_data: *mut c_void,
    id: u64,
    /// (살아 있음, 진행 중인 호출 수)
    gate: Mutex<(bool, u32)>,
    idle: Condvar,
}

// user_data를 SDK 스레드에서 쓴다 — 스레드 안전은 앱의 계약이다(머리 파일 머리말)
unsafe impl Send for Listener {}
unsafe impl Sync for Listener {}

thread_local! {
    /// 이 스레드가 지금 부르고 있는 구독 — 콜백 안에서 해제하면 자기 끝을 기다리지 않는다
    static CALLING: Cell<u64> = const { Cell::new(0) };
}

impl Listener {
    fn call(&self, state: &SessionState) {
        {
            let mut gate = self.gate.lock().unwrap_or_else(|e| e.into_inner());
            if !gate.0 {
                return;
            }
            gate.1 += 1;
        }
        let json = c_string(&serde_json::to_string(state).unwrap_or_default());
        let outer = CALLING.with(|c| c.replace(self.id));
        unsafe { (self.callback)(self.user_data, state_type(state), json.as_ptr()) };
        CALLING.with(|c| c.set(outer));
        let mut gate = self.gate.lock().unwrap_or_else(|e| e.into_inner());
        gate.1 -= 1;
        self.idle.notify_all();
    }

    fn close(&self) {
        let mut gate = self.gate.lock().unwrap_or_else(|e| e.into_inner());
        gate.0 = false;
        if CALLING.with(Cell::get) == self.id {
            return;
        }
        while gate.1 > 0 {
            gate = self.idle.wait(gate).unwrap_or_else(|e| e.into_inner());
        }
    }
}

type Subscriptions = (u64, HashMap<u64, (Subscription, Arc<Listener>)>);

static SUBSCRIPTIONS: LazyLock<Mutex<Subscriptions>> = LazyLock::new(|| Mutex::new((0, HashMap::new())));

/// 상태가 바뀔 때마다 부른다(SDK의 스레드에서). 구독 ID를 돌려준다 — `bydev_session_unsubscribe`로 끊는다. 실패하면 0.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bydev_session_subscribe(
    session: *const bydev_session,
    callback: bydev_state_callback,
    user_data: *mut c_void,
) -> u64 {
    let (Some(s), Some(callback)) = (unsafe { session.as_ref() }, callback) else { return 0 };
    let mut subs = SUBSCRIPTIONS.lock().unwrap_or_else(|e| e.into_inner());
    subs.0 += 1;
    let id = subs.0;
    let listener = Arc::new(Listener { callback, user_data, id, gate: Mutex::new((true, 0)), idle: Condvar::new() });
    let calling = listener.clone();
    let subscription = s.inner.subscribe(move |state| calling.call(state));
    subs.1.insert(id, (subscription, listener));
    id
}

/// 구독을 끊는다. 돌아온 뒤로는 그 콜백이 불리지 않는다(다른 스레드에서 진행 중인 호출은 끝날 때까지 기다린다 — 콜백 안에서 부르면 기다리지
/// 않는다). 그 뒤로 `user_data`를 놓아도 된다.
#[unsafe(no_mangle)]
pub extern "C" fn bydev_session_unsubscribe(subscription: u64) {
    let found = SUBSCRIPTIONS.lock().unwrap_or_else(|e| e.into_inner()).1.remove(&subscription);
    if let Some((s, listener)) = found {
        s.unsubscribe();
        listener.close();
    }
}

// ── 판매자 서버 ───────────────────────────────────────────────

/// 웹훅 서명을 검증한다(안내서 11장 · Standard Webhooks v1) — 결과 JSON `{"id","timestamp"}`. `body`는 받은 그대로의 본문이다. 시크릿 교체
/// 중이면 둘 다(`secrets` · `secret_count`). `now_ms`가 음수면 시스템 시계.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bydev_verify_webhook(
    secrets: *const *const c_char,
    secret_count: usize,
    webhook_id: *const c_char,
    webhook_timestamp: *const c_char,
    webhook_signature: *const c_char,
    body: *const u8,
    body_len: usize,
    now_ms: i64,
    error: *mut *mut bydev_error,
) -> *mut c_char {
    let list: Vec<&str> = if secrets.is_null() {
        Vec::new()
    } else {
        unsafe { std::slice::from_raw_parts(secrets, secret_count) }
            .iter()
            .filter_map(|p| unsafe { text(*p) })
            .collect()
    };
    let mut headers = Vec::new();
    for (name, value) in
        [("webhook-id", webhook_id), ("webhook-timestamp", webhook_timestamp), ("webhook-signature", webhook_signature)]
    {
        if let Some(v) = unsafe { text(value) } {
            headers.push((name, v));
        }
    }
    let bytes = if body.is_null() { &[][..] } else { unsafe { std::slice::from_raw_parts(body, body_len) } };
    struct Fixed(i64);
    impl Clock for Fixed {
        fn now_ms(&self) -> i64 {
            self.0
        }
    }
    let fixed = Fixed(now_ms);
    let clock: Option<&dyn Clock> = if now_ms < 0 { None } else { Some(&fixed) };
    let result = bydev::verify_webhook(&list, headers, bytes, clock)
        .map(|v| serde_json::json!({ "id": v.id, "timestamp": v.timestamp }));
    json_out(result, error)
}

/// 판매자 서버의 토큰 검증기(안내서 09장) — JWKS를 받아 캐시한다.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bydev_server_verifier_new(
    base_url: *const c_char,
    client_id: *const c_char,
) -> *mut bydev_server_verifier {
    let inner = ServerVerifier::with_user_agent(
        unsafe { text(base_url) }.unwrap_or(""),
        unsafe { text(client_id) }.unwrap_or(""),
        Ports::default(),
        Some(USER_AGENT),
    );
    Box::into_raw(Box::new(bydev_server_verifier { inner }))
}

/// access_token(`at+jwt`)을 검증한다 — 클레임 JSON.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bydev_server_verifier_access_token(
    verifier: *const bydev_server_verifier,
    token: *const c_char,
    error: *mut *mut bydev_error,
) -> *mut c_char {
    let Some(v) = (unsafe { verifier.as_ref() }) else { return std::ptr::null_mut() };
    json_out(v.inner.verify_access_token(unsafe { text(token) }.unwrap_or("")), error)
}

/// licenseToken을 검증한다 — 클레임 JSON. `min_generation` — 같은 sub · device_id에서 이미 본 가장 큰 세대번호(0이면 보지 않는다).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bydev_server_verifier_license_token(
    verifier: *const bydev_server_verifier,
    token: *const c_char,
    min_generation: i64,
    error: *mut *mut bydev_error,
) -> *mut c_char {
    let Some(v) = (unsafe { verifier.as_ref() }) else { return std::ptr::null_mut() };
    json_out(v.inner.verify_license_token(unsafe { text(token) }.unwrap_or(""), min_generation), error)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn bydev_server_verifier_free(verifier: *mut bydev_server_verifier) {
    if !verifier.is_null() {
        drop(unsafe { Box::from_raw(verifier) });
    }
}
