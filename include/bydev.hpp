// bydev C++ SDK — C ABI(bydev.h) 위의 헤더 전용 RAII 래퍼. C++17 이상.
//
// 실패는 bydev::Error 예외다(code()로 분기 — BYDEV_CODE_* 상수). 모델(이용권 · 기기 · 업데이트 · 클레임 · 세션 상태)은 JSON 문자열이다 —
// 앱이 쓰는 JSON 라이브러리로 읽는다(필드가 더해져도 깨지지 않는다). 모든 호출은 블로킹이다.
#ifndef BYDEV_HPP
#define BYDEV_HPP

#include <cstdint>
#include <functional>
#include <memory>
#include <stdexcept>
#include <string>
#include <string_view>
#include <utility>
#include <vector>

#include "bydev.h"

namespace bydev {

namespace detail {

inline std::string str(const char* s) { return s == nullptr ? std::string() : std::string(s); }

/// SDK가 준 문자열을 받아 놓는다.
inline std::string take(char* s) {
  std::string out = str(s);
  bydev_string_free(s);
  return out;
}

}  // namespace detail

/// SDK 에러 — 명세 3.3. code()로 분기하고 message()는 보여 주기만 한다. 오류 화면에 correlation_id()를 보인다.
class Error : public std::runtime_error {
 public:
  explicit Error(bydev_error* e)
      : std::runtime_error(detail::str(bydev_error_code(e)) + ": " + detail::str(bydev_error_message(e))),
        code_(detail::str(bydev_error_code(e))),
        message_(detail::str(bydev_error_message(e))),
        correlation_id_(detail::str(bydev_error_correlation_id(e))),
        http_status_(bydev_error_http_status(e)),
        retry_after_sec_(bydev_error_retry_after_sec(e)),
        transient_(bydev_error_is_transient(e)),
        json_(detail::take(bydev_error_json(e))) {
    bydev_error_free(e);
  }

  const std::string& code() const noexcept { return code_; }
  const std::string& message() const noexcept { return message_; }
  /// 없으면 빈 문자열.
  const std::string& correlation_id() const noexcept { return correlation_id_; }
  /// 없으면 0.
  std::uint16_t http_status() const noexcept { return http_status_; }
  /// 없으면 0.
  std::uint64_t retry_after_sec() const noexcept { return retry_after_sec_; }
  /// 일시적 실패인지(명세 3.4) — 참이면 다시 해 볼 만하다.
  bool is_transient() const noexcept { return transient_; }
  /// details까지 실은 JSON.
  const std::string& json() const noexcept { return json_; }

 private:
  std::string code_;
  std::string message_;
  std::string correlation_id_;
  std::uint16_t http_status_;
  std::uint64_t retry_after_sec_;
  bool transient_;
  std::string json_;
};

namespace detail {

/// char*(JSON)를 돌려주는 C 호출 — NULL이면 그 에러를 던진다.
template <class F>
std::string call(F&& f) {
  bydev_error* error = nullptr;
  char* out = std::forward<F>(f)(&error);
  if (out == nullptr) throw Error(error);
  return take(out);
}

inline const char* opt(const std::string& s) { return s.empty() ? nullptr : s.c_str(); }

}  // namespace detail

/// 설정(명세 1절). device_name · device_platform이 비면 이 기기, app_name을 주면 운영체제 보안 저장소와 지문 출처(설치 ID)를 쓴다.
/// app_target · app_version은 실행 중인 앱의 배포 대상(BYDEV_DISTRIBUTION_TARGET_*)과 콘솔에 업로드한 버전 — 세션을 쓰면 함께 준다
/// (둘 다 비면 싣지 않고, 하나만 주면 CONFIGURATION).
struct Config {
  std::string base_url;
  std::string client_id;
  std::string product_id;
  std::vector<std::string> scopes;
  std::string device_name;
  std::string device_platform;
  std::string app_name;
  /// 바꿔 끼울 포트(C 콜백 표) — nullptr이면 기본 구현. 클라이언트보다 오래 산다.
  const bydev_ports* ports = nullptr;
  std::string app_target;
  std::string app_version;
};

/// 세션 상태 종류.
enum class StateType : std::int32_t {
  Unknown = BYDEV_STATE_UNKNOWN,
  Starting = BYDEV_STATE_STARTING,
  Active = BYDEV_STATE_ACTIVE,
  Degraded = BYDEV_STATE_DEGRADED,
  NeedsDecision = BYDEV_STATE_NEEDS_DECISION,
  Locked = BYDEV_STATE_LOCKED,
  Stopped = BYDEV_STATE_STOPPED,
};

/// 디바이스 로그인 하나 — 화면에 user_code() · verification_uri_complete()(QR)를 보이고 wait().
class Login {
 public:
  explicit Login(bydev_login* h) : h_(h, &bydev_login_free) {}

  std::string user_code() const { return detail::str(bydev_login_user_code(h_.get())); }
  std::string verification_uri() const { return detail::str(bydev_login_verification_uri(h_.get())); }
  std::string verification_uri_complete() const { return detail::str(bydev_login_verification_uri_complete(h_.get())); }
  std::string expires_at() const { return detail::str(bydev_login_expires_at(h_.get())); }

  /// 끝나기를 기다린다 — 받은 스코프.
  std::string wait() const {
    return detail::call([&](bydev_error** e) { return bydev_login_wait(h_.get(), e); });
  }

  /// 취소한다 — wait()는 LOGIN_CANCELLED로 끝난다.
  void cancel() const { bydev_login_cancel(h_.get()); }

 private:
  std::unique_ptr<bydev_login, void (*)(bydev_login*)> h_;
};

/// 상태 구독 — 놓으면 끊는다(놓은 뒤로 함수가 불리지 않는다).
class Subscription {
 public:
  using Listener = std::function<void(StateType, const std::string&)>;

  Subscription() = default;
  Subscription(const bydev_session* session, Listener listener)
      : listener_(std::make_unique<Listener>(std::move(listener))) {
    id_ = bydev_session_subscribe(session, &Subscription::trampoline, listener_.get());
  }
  Subscription(Subscription&& other) noexcept : id_(std::exchange(other.id_, 0)), listener_(std::move(other.listener_)) {}
  Subscription& operator=(Subscription&& other) noexcept {
    if (this != &other) {
      reset();
      id_ = std::exchange(other.id_, 0);
      listener_ = std::move(other.listener_);
    }
    return *this;
  }
  Subscription(const Subscription&) = delete;
  Subscription& operator=(const Subscription&) = delete;
  ~Subscription() { reset(); }

  void reset() {
    if (id_ != 0) bydev_session_unsubscribe(std::exchange(id_, 0));
    listener_.reset();
  }

 private:
  static void trampoline(void* user_data, std::int32_t type, const char* state_json) {
    (*static_cast<Listener*>(user_data))(static_cast<StateType>(type), detail::str(state_json));
  }

  std::uint64_t id_ = 0;
  std::unique_ptr<Listener> listener_;
};

/// 라이선스 세션(실행 리스) — 명세 8절.
class Session {
 public:
  explicit Session(bydev_session* h) : h_(h, &bydev_session_free) {}

  void start() const { bydev_session_start(h_.get()); }
  void take_over() const { bydev_session_take_over(h_.get()); }
  void retry() const { bydev_session_retry(h_.get()); }
  void stop() const { bydev_session_stop(h_.get()); }
  std::string state_json() const { return detail::take(bydev_session_state(h_.get())); }
  StateType state_type() const { return static_cast<StateType>(bydev_session_state_type(h_.get())); }

  /// 상태가 바뀔 때마다 부른다(SDK의 스레드에서).
  Subscription subscribe(Subscription::Listener listener) const { return Subscription(h_.get(), std::move(listener)); }

 private:
  std::unique_ptr<bydev_session, void (*)(bydev_session*)> h_;
};

/// bydev 라이선스 API 클라이언트.
class Client {
 public:
  explicit Client(const Config& config) : h_(nullptr, &bydev_client_free) {
    std::string scopes;
    for (const auto& s : config.scopes) scopes += (scopes.empty() ? "" : " ") + s;
    bydev_config c{config.base_url.c_str(),          config.client_id.c_str(),   config.product_id.c_str(),
                   scopes.c_str(),                   detail::opt(config.device_name), detail::opt(config.device_platform),
                   detail::opt(config.app_name),     config.ports,                detail::opt(config.app_target),
                   detail::opt(config.app_version)};
    bydev_error* error = nullptr;
    h_.reset(bydev_client_new(&c, &error));
    if (!h_) throw Error(error);
  }

  Login start_login() const {
    bydev_error* error = nullptr;
    bydev_login* login = bydev_client_start_login(h_.get(), &error);
    if (login == nullptr) throw Error(error);
    return Login(login);
  }

  /// 저장된 로그인이 있는지 — 이용 가능 여부가 아니다.
  bool has_credentials() const {
    bydev_error* error = nullptr;
    std::int32_t found = bydev_client_has_credentials(h_.get(), &error);
    if (found < 0) throw Error(error);
    return found == 1;
  }

  std::string entitlement() const {
    return detail::call([&](bydev_error** e) { return bydev_client_entitlement(h_.get(), e); });
  }

  std::string devices(const std::string& cursor = std::string(), std::uint32_t limit = 0) const {
    return detail::call([&](bydev_error** e) { return bydev_client_devices(h_.get(), detail::opt(cursor), limit, e); });
  }

  std::string release_device(const std::string& device_id) const {
    return detail::call([&](bydev_error** e) { return bydev_client_release_device(h_.get(), device_id.c_str(), e); });
  }

  std::string profile() const {
    return detail::call([&](bydev_error** e) { return bydev_client_profile(h_.get(), e); });
  }

  std::string check_for_update(const std::string& target, const std::string& current_version) const {
    return detail::call(
        [&](bydev_error** e) { return bydev_client_check_for_update(h_.get(), target.c_str(), current_version.c_str(), e); });
  }

  /// 받은 파일의 SHA-256 — 다르면 DOWNLOAD_INTEGRITY.
  void verify_download(std::string_view data, const std::string& sha256) const {
    bydev_error* error = nullptr;
    const auto* bytes = reinterpret_cast<const std::uint8_t*>(data.data());
    if (!bydev_client_verify_download(h_.get(), bytes, data.size(), sha256.c_str(), &error)) throw Error(error);
  }

  std::string verify_token(const std::string& token) const {
    return detail::call([&](bydev_error** e) { return bydev_client_verify_token(h_.get(), token.c_str(), e); });
  }

  /// 서버가 폐기했는지.
  bool logout() const {
    bydev_error* error = nullptr;
    std::int32_t revoked = bydev_client_logout(h_.get(), &error);
    if (revoked < 0) throw Error(error);
    return revoked == 1;
  }

  Session session() const {
    bydev_error* error = nullptr;
    bydev_session* session = bydev_client_session(h_.get(), &error);
    if (session == nullptr) throw Error(error);
    return Session(session);
  }

 private:
  std::unique_ptr<bydev_client, void (*)(bydev_client*)> h_;
};

/// 웹훅 서명 검증(안내서 11장) — 결과 JSON {"id","timestamp"}. body는 받은 그대로의 본문. now_ms가 음수면 시스템 시계.
inline std::string verify_webhook(const std::vector<std::string>& secrets, const std::string& webhook_id,
                                  const std::string& webhook_timestamp, const std::string& webhook_signature,
                                  std::string_view body, std::int64_t now_ms = -1) {
  std::vector<const char*> list;
  list.reserve(secrets.size());
  for (const auto& s : secrets) list.push_back(s.c_str());
  return detail::call([&](bydev_error** e) {
    return bydev_verify_webhook(list.data(), list.size(), webhook_id.c_str(), webhook_timestamp.c_str(),
                                webhook_signature.c_str(), reinterpret_cast<const std::uint8_t*>(body.data()), body.size(),
                                now_ms, e);
  });
}

/// 판매자 서버의 토큰 검증기(안내서 09장).
class ServerVerifier {
 public:
  ServerVerifier(const std::string& base_url, const std::string& client_id)
      : h_(bydev_server_verifier_new(base_url.c_str(), client_id.c_str()), &bydev_server_verifier_free) {}

  std::string verify_access_token(const std::string& token) const {
    return detail::call([&](bydev_error** e) { return bydev_server_verifier_access_token(h_.get(), token.c_str(), e); });
  }

  std::string verify_license_token(const std::string& token, std::int64_t min_generation = 0) const {
    return detail::call(
        [&](bydev_error** e) { return bydev_server_verifier_license_token(h_.get(), token.c_str(), min_generation, e); });
  }

 private:
  std::unique_ptr<bydev_server_verifier, void (*)(bydev_server_verifier*)> h_;
};

}  // namespace bydev

#endif  // BYDEV_HPP
