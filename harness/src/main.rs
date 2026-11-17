//! C SDK의 적합성 하네스(sdks/spec/harness-protocol.md). **C ABI만으로** SDK를 몬다 — `bydev_*` 함수와 C 콜백 표(`bydev_ports`)만 쓰고 Rust SDK의
//! 타입은 쓰지 않는다. 그래서 시나리오가 C ABI의 소유권 · 문자열 · 포트 변환까지 본다. 머리 파일이 C · C++ 컴파일러로 맞는지는 `tests/smoke.c` ·
//! `tests/smoke.cpp`가 본다.
//!
//! 정착 판정은 Rust 하네스와 같다 — SDK 일을 하는 스레드 수와, 포트에서 기다리는 스레드가 등록한 신호.

#![allow(unsafe_code)]

use std::collections::HashMap;
use std::ffi::{CStr, CString, c_char, c_void};
use std::io::{BufRead, Write};
use std::sync::mpsc;
use std::sync::{Arc, Condvar, LazyLock, Mutex, MutexGuard};
use std::time::Duration;

use bydev_c::*;
use serde_json::{Value, json};

/// 하네스가 직접 만든 신호(sleep · 전송 대기).
#[derive(Clone, Default)]
struct Own(Arc<(Mutex<bool>, Condvar)>);

impl Own {
    fn fire(&self) {
        *self.0.0.lock().unwrap() = true;
        self.0.1.notify_all();
    }
    fn fired(&self) -> bool {
        *self.0.0.lock().unwrap()
    }
    fn block(&self) {
        let mut fired = self.0.0.lock().unwrap();
        while !*fired {
            fired = self.0.1.wait(fired).unwrap();
        }
    }
}

enum Waiter {
    Own(Own),
    /// SDK가 `scheduler_wait`에 넘긴 신호 — 그 호출 동안(등록돼 있는 동안)만 유효하다
    Sdk(*const bydev_signal),
}

unsafe impl Send for Waiter {}

impl Waiter {
    fn fired(&self) -> bool {
        match self {
            Self::Own(o) => o.fired(),
            Self::Sdk(p) => unsafe { bydev_signal_is_fired(*p) },
        }
    }
}

struct Timer {
    id: u64,
    due: i64,
    task: *mut bydev_task,
    cancelled: bool,
}

unsafe impl Send for Timer {}

struct Exchange {
    done: Own,
    response: Option<String>,
}

#[derive(Default)]
struct State {
    now: i64,
    timers: Vec<Timer>,
    next_timer: u64,
    running: i64,
    waiters: HashMap<u64, Waiter>,
    waiter_id: u64,
    pending: HashMap<String, Arc<Mutex<Exchange>>>,
    randoms: Vec<f64>,
    uuids: u64,
    http_seq: u64,
    record: Option<String>,
    fail_load: bool,
    fail_save: bool,
    fail_clear: bool,
    fingerprint: Option<String>,
}

static STATE: LazyLock<Mutex<State>> = LazyLock::new(|| Mutex::new(State::default()));
static OUT: Mutex<()> = Mutex::new(());

struct Handles {
    client: *mut bydev_client,
    session: *mut bydev_session,
    login: *mut bydev_login,
}

unsafe impl Send for Handles {}

static HANDLES: Mutex<Handles> =
    Mutex::new(Handles { client: std::ptr::null_mut(), session: std::ptr::null_mut(), login: std::ptr::null_mut() });

fn state() -> MutexGuard<'static, State> {
    STATE.lock().unwrap_or_else(|e| e.into_inner())
}

fn emit(message: Value) {
    let _guard = OUT.lock().unwrap_or_else(|e| e.into_inner());
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{message}");
    let _ = out.flush();
}

fn fatal(message: impl Into<String>) -> ! {
    emit(json!({ "type": "fatal", "message": message.into() }));
    std::process::exit(1);
}

fn now_ms() -> i64 {
    state().now
}

fn block(waiter: Waiter, wait: impl FnOnce()) {
    let id = {
        let mut s = state();
        s.running -= 1;
        s.waiter_id += 1;
        let id = s.waiter_id;
        s.waiters.insert(id, waiter);
        id
    };
    wait();
    let mut s = state();
    s.waiters.remove(&id);
    s.running += 1;
}

fn c(s: &str) -> CString {
    CString::new(s).unwrap()
}

/// SDK가 놓을 문자열(`bydev_string_new`).
fn sdk_string(s: &str) -> *mut c_char {
    let owned = c(s);
    unsafe { bydev_string_new(owned.as_ptr()) }
}

/// SDK가 준 문자열을 읽고 놓는다.
fn take(p: *mut c_char) -> Option<String> {
    if p.is_null() {
        return None;
    }
    let text = unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned();
    unsafe { bydev_string_free(p) };
    Some(text)
}

fn borrowed(p: *const c_char) -> String {
    if p.is_null() { String::new() } else { unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned() }
}

// ── C 콜백 ────────────────────────────────────────────────────

unsafe extern "C" fn transport_send(_: *mut c_void, request: *const c_char, error: *mut *mut c_char) -> *mut c_char {
    let request: Value = serde_json::from_str(&borrowed(request)).unwrap_or_else(|_| fatal("요청 JSON"));
    let exchange = Arc::new(Mutex::new(Exchange { done: Own::default(), response: None }));
    let (id, at, done) = {
        let mut s = state();
        s.http_seq += 1;
        let id = format!("h{}", s.http_seq);
        s.pending.insert(id.clone(), exchange.clone());
        let done = exchange.lock().unwrap().done.clone();
        (id, s.now, done)
    };
    emit(json!({ "type": "http.request", "id": id, "now": at, "request": request }));
    let wait = done.clone();
    block(Waiter::Own(done), move || wait.block());
    match exchange.lock().unwrap().response.take() {
        Some(r) => sdk_string(&r),
        None => {
            unsafe { *error = sdk_string("대리 전송 실패") };
            std::ptr::null_mut()
        }
    }
}

unsafe extern "C" fn store_load(_: *mut c_void, error: *mut *mut c_char) -> *mut c_char {
    let s = state();
    if s.fail_load {
        unsafe { *error = sdk_string("저장소 읽기 실패(시험)") };
        return std::ptr::null_mut();
    }
    s.record.as_deref().map_or(std::ptr::null_mut(), sdk_string)
}

unsafe extern "C" fn store_save(_: *mut c_void, record: *const c_char, error: *mut *mut c_char) -> bool {
    let record = borrowed(record);
    let at = {
        let mut s = state();
        if s.fail_save {
            unsafe { *error = sdk_string("저장소 쓰기 실패(시험)") };
            return false;
        }
        s.record = Some(record.clone());
        s.now
    };
    let value: Value = serde_json::from_str(&record).unwrap_or(Value::Null);
    emit(json!({ "type": "event", "now": at, "event": { "kind": "storage", "op": "save", "record": value } }));
    true
}

unsafe extern "C" fn store_clear(_: *mut c_void, error: *mut *mut c_char) -> bool {
    let at = {
        let mut s = state();
        if s.fail_clear {
            unsafe { *error = sdk_string("저장소 지우기 실패(시험)") };
            return false;
        }
        s.record = None;
        s.now
    };
    emit(json!({ "type": "event", "now": at, "event": { "kind": "storage", "op": "clear" } }));
    true
}

unsafe extern "C" fn clock_now(_: *mut c_void) -> i64 {
    now_ms()
}

unsafe extern "C" fn scheduler_after(_: *mut c_void, delay_ms: u64, task: *mut bydev_task) -> u64 {
    let mut s = state();
    s.next_timer += 1;
    let id = s.next_timer;
    let due = s.now + delay_ms as i64;
    s.timers.push(Timer { id, due, task, cancelled: false });
    id
}

unsafe extern "C" fn scheduler_cancel(_: *mut c_void, id: u64) {
    if let Some(t) = state().timers.iter_mut().find(|t| t.id == id) {
        t.cancelled = true;
    }
}

unsafe extern "C" fn scheduler_sleep(ud: *mut c_void, delay_ms: u64) {
    let done = Own::default();
    {
        let mut s = state();
        s.next_timer += 1;
        let id = s.next_timer;
        let due = s.now + delay_ms as i64;
        // 잠을 깨우는 일도 예약 작업 하나 — 작업 대신 신호를 켠다(task가 NULL)
        s.timers.push(Timer { id, due, task: std::ptr::null_mut(), cancelled: false });
        SLEEPERS.lock().unwrap().insert(id, done.clone());
    }
    let _ = ud;
    let wait = done.clone();
    block(Waiter::Own(done), move || wait.block());
}

static SLEEPERS: LazyLock<Mutex<HashMap<u64, Own>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

unsafe extern "C" fn scheduler_wait(_: *mut c_void, signal: *const bydev_signal) {
    let ptr = signal as usize;
    block(Waiter::Sdk(signal), move || unsafe { bydev_signal_block(ptr as *const bydev_signal) });
}

unsafe extern "C" fn random_float(_: *mut c_void) -> f64 {
    let mut s = state();
    if s.randoms.is_empty() {
        drop(s);
        fatal("시나리오의 random 값을 다 썼다 — random에 더 적는다");
    }
    s.randoms.remove(0)
}

unsafe extern "C" fn random_uuid(_: *mut c_void) -> *mut c_char {
    let mut s = state();
    s.uuids += 1;
    sdk_string(&format!("00000000-0000-4000-8000-{:012x}", s.uuids))
}

unsafe extern "C" fn fingerprint_read(_: *mut c_void, _: *mut *mut c_char) -> *mut c_char {
    state().fingerprint.as_deref().map_or(std::ptr::null_mut(), sdk_string)
}

unsafe extern "C" fn log(_: *mut c_void, level: i32, message: *const c_char) {
    if level >= 2 {
        eprintln!("warn: {}", borrowed(message));
    }
}

unsafe extern "C" fn on_state(_: *mut c_void, _: i32, state_json: *const c_char) {
    let value: Value = serde_json::from_str(&borrowed(state_json)).unwrap_or(Value::Null);
    emit(json!({ "type": "event", "now": now_ms(), "event": { "kind": "session.state", "state": value } }));
}

// ── 정착 ──────────────────────────────────────────────────────

fn quiet(s: &State) -> bool {
    s.running == 0 && s.pending.is_empty() && !s.waiters.values().any(Waiter::fired)
}

fn wait_quiet() {
    while !quiet(&state()) {
        std::thread::sleep(Duration::from_micros(50));
    }
}

fn prune(s: &mut State) {
    s.timers.retain(|t| {
        if t.cancelled && !t.task.is_null() {
            unsafe { bydev_task_free(t.task) };
        }
        !t.cancelled
    });
}

/// `limit`까지 온 가장 이른 예약(같은 시각이면 먼저 건 것).
fn take_timer(s: &mut State, limit: i64) -> Option<Timer> {
    prune(s);
    let index = s
        .timers
        .iter()
        .enumerate()
        .filter(|(_, t)| t.due <= limit)
        .min_by_key(|(_, t)| (t.due, t.id))
        .map(|(i, _)| i)?;
    Some(s.timers.remove(index))
}

fn settle() {
    loop {
        wait_quiet();
        let timer = {
            let mut s = state();
            let now = s.now;
            let timer = take_timer(&mut s, now);
            if timer.is_some() {
                s.running += 1;
            }
            timer
        };
        let Some(timer) = timer else { return };
        if timer.task.is_null() {
            if let Some(sleeper) = SLEEPERS.lock().unwrap().remove(&timer.id) {
                sleeper.fire();
            }
            state().running -= 1;
            continue;
        }
        let task = timer.task as usize;
        std::thread::spawn(move || {
            unsafe { bydev_task_run(task as *mut bydev_task) };
            state().running -= 1;
        });
    }
}

fn advance(ms: i64) {
    let target = now_ms() + ms;
    loop {
        settle();
        let mut s = state();
        prune(&mut s);
        let Some(due) = s.timers.iter().filter(|t| t.due <= target).map(|t| t.due).min() else { break };
        s.now = due;
    }
    state().now = target;
    settle();
}

// ── 호출 ──────────────────────────────────────────────────────

fn error_value(error: *mut bydev_error) -> Value {
    let json = take(unsafe { bydev_error_json(error) }).unwrap_or_default();
    unsafe { bydev_error_free(error) };
    serde_json::from_str(&json).unwrap_or(Value::Null)
}

type Outcome = Result<Value, Value>;

/// `char*` JSON을 돌려주는 호출의 결과.
fn json_call(f: impl FnOnce(*mut *mut bydev_error) -> *mut c_char) -> Outcome {
    let mut error = std::ptr::null_mut();
    match take(f(&mut error)) {
        Some(text) => Ok(serde_json::from_str(&text).unwrap_or(Value::Null)),
        None => Err(error_value(error)),
    }
}

fn arg(args: &Value, key: &str) -> CString {
    c(args.get(key).and_then(Value::as_str).unwrap_or_else(|| fatal(format!("인자 {key}가 문자열이 아니다"))))
}

fn invoke(method: &str, args: &Value) -> Outcome {
    let (client, session) = {
        let h = HANDLES.lock().unwrap();
        (h.client as usize, h.session as usize)
    };
    let client = client as *const bydev_client;
    let session = session as *const bydev_session;
    let mut error = std::ptr::null_mut();
    unsafe {
        match method {
            "login.start" => {
                let login = bydev_client_start_login(client, &mut error);
                if login.is_null() {
                    return Err(error_value(error));
                }
                let value = json!({
                    "userCode": borrowed(bydev_login_user_code(login)),
                    "verificationUri": borrowed(bydev_login_verification_uri(login)),
                    "verificationUriComplete": borrowed(bydev_login_verification_uri_complete(login)),
                    "expiresAt": borrowed(bydev_login_expires_at(login)),
                });
                let previous = std::mem::replace(&mut HANDLES.lock().unwrap().login, login);
                bydev_login_free(previous);
                Ok(value)
            }
            "login.wait" => {
                let login = HANDLES.lock().unwrap().login as usize;
                if login == 0 {
                    fatal("login.start 전에 login.wait");
                }
                match take(bydev_login_wait(login as *const bydev_login, &mut error)) {
                    Some(scope) => Ok(json!({ "scope": scope })),
                    None => Err(error_value(error)),
                }
            }
            "login.cancel" => {
                bydev_login_cancel(HANDLES.lock().unwrap().login);
                Ok(Value::Null)
            }
            "hasCredentials" => match bydev_client_has_credentials(client, &mut error) {
                -1 => Err(error_value(error)),
                found => Ok(json!(found == 1)),
            },
            "entitlement" => json_call(|e| bydev_client_entitlement(client, e)),
            "devices" => {
                let cursor = args.get("cursor").and_then(Value::as_str).map(c);
                let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(0) as u32;
                json_call(|e| {
                    bydev_client_devices(client, cursor.as_ref().map_or(std::ptr::null(), |s| s.as_ptr()), limit, e)
                })
            }
            "releaseDevice" => {
                let id = arg(args, "deviceId");
                json_call(|e| bydev_client_release_device(client, id.as_ptr(), e))
            }
            "profile" => json_call(|e| bydev_client_profile(client, e)),
            "checkForUpdate" => {
                let (target, current) = (arg(args, "target"), arg(args, "currentVersion"));
                json_call(|e| bydev_client_check_for_update(client, target.as_ptr(), current.as_ptr(), e))
            }
            "verifyDownload" => {
                use base64::Engine;
                let data = base64::engine::general_purpose::STANDARD
                    .decode(args["dataBase64"].as_str().unwrap_or(""))
                    .unwrap_or_else(|e| fatal(format!("dataBase64: {e}")));
                let sha = arg(args, "sha256");
                if bydev_client_verify_download(client, data.as_ptr(), data.len(), sha.as_ptr(), &mut error) {
                    Ok(Value::Null)
                } else {
                    Err(error_value(error))
                }
            }
            "verifyToken" => {
                let token = arg(args, "token");
                json_call(|e| bydev_client_verify_token(client, token.as_ptr(), e))
            }
            "logout" => match bydev_client_logout(client, &mut error) {
                -1 => Err(error_value(error)),
                revoked => Ok(json!({ "serverRevoked": revoked == 1 })),
            },
            "session.start" => {
                bydev_session_start(session);
                Ok(Value::Null)
            }
            "session.takeOver" => {
                bydev_session_take_over(session);
                Ok(Value::Null)
            }
            "session.retry" => {
                bydev_session_retry(session);
                Ok(Value::Null)
            }
            "session.stop" => {
                bydev_session_stop(session);
                Ok(Value::Null)
            }
            "session.state" => {
                Ok(serde_json::from_str(&take(bydev_session_state(session)).unwrap_or_default()).unwrap_or(Value::Null))
            }
            other => fatal(format!("모르는 호출 {other}")),
        }
    }
}

fn start_call(id: String, method: String, args: Value) {
    state().running += 1;
    std::thread::spawn(move || {
        let result = invoke(&method, &args);
        let at = now_ms();
        match result {
            Ok(value) => emit(json!({ "type": "result", "id": id, "now": at, "ok": true, "value": value })),
            Err(error) => emit(json!({ "type": "result", "id": id, "now": at, "ok": false, "error": error })),
        }
        state().running -= 1;
    });
}

// ── 명령 ──────────────────────────────────────────────────────

/// C 쪽이 오래 쥐는 값 — 프로세스 끝까지 둔다.
static KEEP: Mutex<Vec<CString>> = Mutex::new(Vec::new());

fn keep(s: &str) -> *const c_char {
    let owned = c(s);
    let ptr = owned.as_ptr();
    KEEP.lock().unwrap().push(owned);
    ptr
}

fn init(message: &Value) {
    let config = &message["config"];
    let text = |v: &Value, key: &str| v[key].as_str().unwrap_or_else(|| fatal(format!("init.{key}"))).to_owned();
    // 시각 읽기도 C ABI 밖의 도움 없이 — 시나리오 시작 시각은 UTC `…Z`
    let started = parse_utc(message["now"].as_str().unwrap_or("")).unwrap_or_else(|| fatal("init.now"));
    {
        let mut s = state();
        s.now = started;
        s.randoms =
            message["random"].as_array().map(|a| a.iter().filter_map(Value::as_f64).collect()).unwrap_or_default();
        s.record = (!message["storage"].is_null()).then(|| message["storage"].to_string());
        s.fingerprint = config["device"]["fingerprintSource"].as_str().map(str::to_owned);
    }
    let scopes: Vec<String> = config["scopes"]
        .as_array()
        .map(|a| a.iter().filter_map(|s| s.as_str().map(str::to_owned)).collect())
        .unwrap_or_default();
    let ports = Box::leak(Box::new(bydev_ports {
        user_data: std::ptr::null_mut(),
        transport_send: Some(transport_send),
        store_load: Some(store_load),
        store_save: Some(store_save),
        store_clear: Some(store_clear),
        clock_now_ms: Some(clock_now),
        scheduler_after: Some(scheduler_after),
        scheduler_cancel: Some(scheduler_cancel),
        scheduler_sleep: Some(scheduler_sleep),
        scheduler_wait: Some(scheduler_wait),
        random_float: Some(random_float),
        random_uuid: Some(random_uuid),
        fingerprint_read: Some(fingerprint_read),
        log: Some(log),
    }));
    let cfg = bydev_config {
        base_url: keep(&text(config, "baseUrl")),
        client_id: keep(&text(config, "clientId")),
        product_id: keep(&text(config, "productId")),
        scopes: keep(&scopes.join(" ")),
        device_name: keep(&text(&config["device"], "name")),
        device_platform: keep(&text(&config["device"], "platform")),
        app_name: std::ptr::null(),
        ports,
        // 시나리오가 적을 때만 온다
        app_target: if config["app"].is_null() { std::ptr::null() } else { keep(&text(&config["app"], "target")) },
        app_version: if config["app"].is_null() { std::ptr::null() } else { keep(&text(&config["app"], "version")) },
    };
    let mut error = std::ptr::null_mut();
    unsafe {
        let client = bydev_client_new(&cfg, &mut error);
        if client.is_null() {
            fatal(format!("bydev_client_new: {}", error_value(error)));
        }
        let session = bydev_client_session(client, &mut error);
        if session.is_null() {
            fatal(format!("bydev_client_session: {}", error_value(error)));
        }
        if bydev_session_subscribe(session, Some(on_state), std::ptr::null_mut()) == 0 {
            fatal("bydev_session_subscribe");
        }
        let mut h = HANDLES.lock().unwrap();
        h.client = client;
        h.session = session;
    }
}

/// `YYYY-MM-DDTHH:MM:SS(.fff)Z` → Unix 밀리초.
fn parse_utc(text: &str) -> Option<i64> {
    let (date, time) = text.strip_suffix('Z')?.split_once('T')?;
    let mut d = date.split('-').map(|p| p.parse::<i64>().ok());
    let (y, m, day) = (d.next()??, d.next()??, d.next()??);
    let (clock, frac) = time.split_once('.').unwrap_or((time, "0"));
    let mut t = clock.split(':').map(|p| p.parse::<i64>().ok());
    let (hh, mm, ss) = (t.next()??, t.next()??, t.next()??);
    let ms: i64 = format!("{frac:0<3}")[..3].parse().ok()?;
    let y2 = if m <= 2 { y - 1 } else { y };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + day - 1;
    let days = era * 146_097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719_468;
    Some((days * 86_400 + hh * 3600 + mm * 60 + ss) * 1000 + ms)
}

fn on_http_response(message: &Value) {
    let id = message["id"].as_str().unwrap_or("");
    let mut s = state();
    let Some(exchange) = s.pending.remove(id) else {
        drop(s);
        fatal(format!("모르는 http.response id {id}"));
    };
    let mut x = exchange.lock().unwrap();
    if let Some(r) = message.get("response").filter(|r| !r.is_null()) {
        x.response = Some(r.to_string());
    }
    x.done.fire();
}

fn main() {
    let (version, spec) = (borrowed(bydev_version()), borrowed(bydev_spec_version()));
    emit(json!({ "type": "ready", "sdk": "c", "version": version, "specVersion": spec }));
    let (tx, rx) = mpsc::channel::<Value>();
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines() {
            let Ok(line) = line else { break };
            let message: Value =
                serde_json::from_str(&line).unwrap_or_else(|_| fatal(format!("JSON이 아닌 입력: {line}")));
            if message["type"] == "http.response" {
                on_http_response(&message);
            } else if tx.send(message).is_err() {
                break;
            }
        }
    });
    for message in rx {
        match message["type"].as_str().unwrap_or("") {
            "init" => {
                init(&message);
                settle();
            }
            "call" => {
                for call in message["calls"].as_array().cloned().unwrap_or_default() {
                    let id = call["id"].as_str().unwrap_or("").to_owned();
                    let method = call["method"].as_str().unwrap_or("").to_owned();
                    let args = if call["args"].is_object() { call["args"].clone() } else { json!({}) };
                    start_call(id, method, args);
                }
                settle();
            }
            "advance" => advance(message["ms"].as_i64().unwrap_or(0)),
            "set" => {
                let value = message["value"].as_bool().unwrap_or(false);
                {
                    let mut s = state();
                    match message["key"].as_str().unwrap_or("") {
                        "storage.failLoad" => s.fail_load = value,
                        "storage.failSave" => s.fail_save = value,
                        "storage.failClear" => s.fail_clear = value,
                        _ => {}
                    }
                }
                settle();
            }
            "end" => std::process::exit(0),
            other => fatal(format!("모르는 명령 {other}")),
        }
        emit(json!({ "type": "idle", "now": now_ms() }));
    }
}
