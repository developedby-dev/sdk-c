/* bydev C SDK — 이 파일은 빌드가 만든다(sdks/c/build.rs · cbindgen). 고치지 않는다.
 *
 * 소유권: SDK가 돌려준 char*는 bydev_string_free로, bydev_error*는 bydev_error_free로, 핸들은 그 _free로 놓는다. 콜백이 SDK에 넘기는
 * 문자열은 bydev_string_new로 만든다(SDK가 놓는다). 인자로 받은 const char*는 그 호출 동안만 유효하다. 문자열은 모두 UTF-8 · NUL 끝.
 * 모든 함수는 블로킹이고 어느 스레드에서 불러도 된다. 콜백은 SDK의 스레드에서 불린다. */

#ifndef BYDEV_H
#define BYDEV_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include "bydev_generated.h"

// 클라이언트 — `bydev_client_new`로 만들고 `bydev_client_free`로 놓는다.
typedef struct bydev_client bydev_client;

// 에러 — `bydev_error_free`로 놓는다.
typedef struct bydev_error bydev_error;

// 진행 중인 디바이스 로그인 — `bydev_login_free`로 놓는다(놓아도 취소하지 않는다 — `bydev_login_cancel`).
typedef struct bydev_login bydev_login;

// 판매자 서버의 토큰 검증기 — `bydev_server_verifier_free`로 놓는다.
typedef struct bydev_server_verifier bydev_server_verifier;

// 세션 핸들 — 같은 클라이언트의 세션은 하나다. `bydev_session_free`는 핸들만 놓는다(멈추려면 `bydev_session_stop`).
typedef struct bydev_session bydev_session;

// 기다림의 신호 — `scheduler_wait` 콜백이 받는다. 그 호출 동안만 유효하다.
typedef struct bydev_signal bydev_signal;

// 예약 작업 — 스케줄러 콜백이 받는다. 때가 되면 `bydev_task_run`(한 번, 놓기까지), 취소됐으면 `bydev_task_free`.
typedef struct bydev_task bydev_task;

// 바꿔 끼울 포트(명세 2절) — 모두 선택이다. 묶음(전송 · 저장소 셋 · 시계 · 스케줄러 넷 · 난수 둘 · 지문 · 로그)마다 다 채우면 그 묶음을 쓰고, 아니면
// 기본 구현이다. 콜백은 SDK의 스레드에서 불린다. 콜백이 돌려주는 문자열은 `bydev_string_new`로 만든다. `error`에 메시지를 넣으면 실패다.
typedef struct {
  void *user_data;
  // 요청 JSON `{"method","url","headers":{},"body":문자열|null}` → 응답 JSON `{"status","headers":{},"body":문자열}`. 연결하지 못하면 NULL + `*error`.
  char *(*transport_send)(void*, const char*, char**);
  // 기록 JSON(명세 3.6) 또는 NULL(없음).
  char *(*store_load)(void*, char**);
  bool (*store_save)(void*, const char*, char**);
  bool (*store_clear)(void*, char**);
  // 지금 시각(Unix 밀리초).
  int64_t (*clock_now_ms)(void*);
  // `delay_ms` 뒤에 `bydev_task_run(task)` — 예약 ID를 돌려준다.
  uint64_t (*scheduler_after)(void*, uint64_t, bydev_task*);
  // 예약 취소 — 그 작업은 돌리지 않고 `bydev_task_free`.
  void (*scheduler_cancel)(void*, uint64_t);
  void (*scheduler_sleep)(void*, uint64_t);
  // 신호가 켜질 때까지 기다린다(`bydev_signal_block`).
  void (*scheduler_wait)(void*, const bydev_signal*);
  double (*random_float)(void*);
  char *(*random_uuid)(void*);
  // 기기 식별자 원본 또는 NULL — 원본은 SDK 밖으로 나가지 않는다(해시만).
  char *(*fingerprint_read)(void*, char**);
  // 0 debug · 1 info · 2 warn. 토큰 · 지문 원본은 넘어오지 않는다.
  void (*log)(void*, int32_t, const char*);
} bydev_ports;

// 설정(명세 1절).
typedef struct {
  const char *base_url;
  const char *client_id;
  const char *product_id;
  // 공백으로 나눈 스코프(`"entitlement.read session.write"`).
  const char *scopes;
  // NULL이면 호스트 이름.
  const char *device_name;
  // NULL이면 이 운영체제(`BYDEV_DEVICE_PLATFORM_*`).
  const char *device_platform;
  // NULL이 아니면 기본 저장소(운영체제 보안 저장소)와 지문 출처(앱 폴더의 설치 ID)를 쓴다 — 포트로 준 것이 먼저다.
  const char *app_name;
  // NULL이면 모두 기본 구현(저장소는 메모리 — `app_name`을 주지 않으면 앱을 끄면 로그인이 사라진다).
  const bydev_ports *ports;
  // 실행 중인 앱의 배포 대상(`BYDEV_DISTRIBUTION_TARGET_*` — 업데이트 확인의 `target`과 같은 값). 세션을 쓰면 `app_version`과 함께
  // 준다(명세 1절 · ADR-0049) — 세션 획득에 싣는다. 업데이트 제공 기한이 있는 영구 이용권은 이 값이 없으면 획득이
  // `ENTITLEMENT_VERSION_NOT_COVERED`로 거절된다. 둘 다 NULL이면 싣지 않고, 하나만 주면 `CONFIGURATION`.
  const char *app_target;
  // 콘솔에 업로드한 빌드의 버전 문자열(`"1.4.0"`).
  const char *app_version;
} bydev_config;

// 상태 콜백 — `state_type`과 상태 JSON(그 호출 동안만 유효).
typedef void (*bydev_state_callback)(void*, int32_t, const char*);

#ifdef __cplusplus
extern "C" {
#endif // __cplusplus

// 콜백이 SDK에 넘길 문자열을 만든다(SDK가 놓는다). `s`가 NULL이면 NULL.
char *bydev_string_new(const char *s);

// SDK가 돌려준 문자열을 놓는다. NULL이면 아무것도 하지 않는다.
void bydev_string_free(char *s);

// 예약 작업을 돌리고 놓는다.
void bydev_task_run(bydev_task *task);

// 예약 작업을 돌리지 않고 놓는다(취소된 예약).
void bydev_task_free(bydev_task *task);

// 신호가 켜질 때까지 막는다.
void bydev_signal_block(const bydev_signal *signal);

// 신호가 켜졌는지다.
bool bydev_signal_is_fired(const bydev_signal *signal);

// 에러 코드(`BYDEV_CODE_*`와 strcmp). 에러를 놓기 전까지 유효하다.
const char *bydev_error_code(const bydev_error *error);

// 보여 주기만 하는 설명(한국어).
const char *bydev_error_message(const bydev_error *error);

// 문의용 상관 ID — 없으면 NULL. 오류 화면에 보인다.
const char *bydev_error_correlation_id(const bydev_error *error);

// HTTP 상태 — 없으면 0.
uint16_t bydev_error_http_status(const bydev_error *error);

// 다시 시도할 때까지 기다릴 초 — 없으면 0.
uint64_t bydev_error_retry_after_sec(const bydev_error *error);

// 일시적 실패인지(명세 3.4) — 참이면 다시 해 볼 만하다.
bool bydev_error_is_transient(const bydev_error *error);

// 에러 전체(`code` · `message` · `httpStatus` · `correlationId` · `retryAfterSec` · `details`) JSON. `bydev_string_free`로 놓는다.
char *bydev_error_json(const bydev_error *error);

void bydev_error_free(bydev_error *error);

// SDK 판(이 라이브러리 `Cargo.toml`의 판 — 예: `1.0.0`).
const char *bydev_version(void);

// 따르는 동작 명세 판(`BYDEV_SPEC_VERSION`과 같다).
const char *bydev_spec_version(void);

// 클라이언트를 만든다. 실패하면 NULL + `*error`.
bydev_client *bydev_client_new(const bydev_config *config, bydev_error **error);

void bydev_client_free(bydev_client *client);

// 디바이스 로그인을 시작한다(명세 5절). 화면에 `bydev_login_user_code` · `bydev_login_verification_uri_complete`(QR)를 보이고 `bydev_login_wait`.
bydev_login *bydev_client_start_login(const bydev_client *c,
                                      bydev_error **error);

const char *bydev_login_user_code(const bydev_login *login);

const char *bydev_login_verification_uri(const bydev_login *login);

const char *bydev_login_verification_uri_complete(const bydev_login *login);

// 코드가 만료되는 시각(ISO 8601).
const char *bydev_login_expires_at(const bydev_login *login);

// 로그인이 끝나기를 기다린다. 성공하면 받은 스코프(`bydev_string_free`), 실패하면 NULL + `*error`.
char *bydev_login_wait(const bydev_login *login,
                       bydev_error **error);

// 로그인을 취소한다 — `bydev_login_wait`는 `LOGIN_CANCELLED`로 끝난다.
void bydev_login_cancel(const bydev_login *login);

void bydev_login_free(bydev_login *login);

// 저장된 로그인이 있으면 1, 없으면 0, 실패면 -1 + `*error` — 이용 가능 여부가 아니다.
int32_t bydev_client_has_credentials(const bydev_client *c,
                                     bydev_error **error);

// 이 제품의 이용권 JSON. `updateCoverage.type`(`BYDEV_UPDATE_COVERAGE_TYPE_*`)이 `UNTIL`이면 `until` 전에 출시된 버전까지 실행할 수 있다.
char *bydev_client_entitlement(const bydev_client *c,
                               bydev_error **error);

// 등록된 기기 한 쪽 JSON(`items` · `nextCursor`). `cursor`는 NULL이면 첫 쪽, `limit` 0이면 서버 기본.
char *bydev_client_devices(const bydev_client *c,
                           const char *cursor,
                           uint32_t limit,
                           bydev_error **error);

// 기기를 해제한다(월 횟수 제한) — 결과 JSON.
char *bydev_client_release_device(const bydev_client *c,
                                  const char *device_id,
                                  bydev_error **error);

// 닉네임과 가명 식별자 JSON.
char *bydev_client_profile(const bydev_client *c, bydev_error **error);

// 업데이트 확인 JSON. `entitled`가 있으면(업데이트 제공 기한이 있는 구매자) `latest` · `update` · `install` 대신 `entitled.release`를
// 따른다 — 범위 안 버전이 없으면 null. `update.status`가 `REQUIRED`면 업데이트 전까지 실행을 막는다. 받은 파일은 설치 전에
// `bydev_client_verify_download`.
char *bydev_client_check_for_update(const bydev_client *c,
                                    const char *target,
                                    const char *current_version,
                                    bydev_error **error);

// 내려받은 파일의 SHA-256을 확인한다 — 다르면 false + `DOWNLOAD_INTEGRITY`.
bool bydev_client_verify_download(const bydev_client *c,
                                  const uint8_t *data,
                                  size_t len,
                                  const char *sha256,
                                  bydev_error **error);

// licenseToken을 검증한다(명세 9절) — 클레임 JSON. `build_id`는 서버가 앱의 대상 · 버전으로 빌드를 확정했을 때만 있다.
char *bydev_client_verify_token(const bydev_client *c,
                                const char *token,
                                bydev_error **error);

// 세션을 멈추고 폐기 요청을 한 번 보낸 뒤 기록을 지운다. 서버가 폐기했으면 1, 아니면 0, 실패면 -1 + `*error`.
int32_t bydev_client_logout(const bydev_client *c,
                            bydev_error **error);

// 이 클라이언트의 세션 핸들.
bydev_session *bydev_client_session(const bydev_client *c, bydev_error **error);

void bydev_session_free(bydev_session *session);

// `Stopped`에서만 의미가 있다(8.2절).
void bydev_session_start(const bydev_session *session);

// `NeedsDecision`(다른 기기 · 같은 기기 실행 중) 또는 `Locked(SESSION_SUPERSEDED)`에서 인수한다.
void bydev_session_take_over(const bydev_session *session);

// `NeedsDecision`(기기 수) 또는 스스로 풀리지 않는 `Locked`에서 다시 얻는다.
void bydev_session_retry(const bydev_session *session);

// 예약을 모두 취소하고 반납을 한 번 보낸다(8.7절).
void bydev_session_stop(const bydev_session *session);

// 지금 상태 JSON(`{"type": "Active", …}` — 명세 8.1절).
char *bydev_session_state(const bydev_session *session);

// 지금 상태의 종류(`bydev_state_type` — `BYDEV_STATE_ACTIVE` 등).
int32_t bydev_session_state_type(const bydev_session *session);

// 상태가 바뀔 때마다 부른다(SDK의 스레드에서). 구독 ID를 돌려준다 — `bydev_session_unsubscribe`로 끊는다. 실패하면 0.
uint64_t bydev_session_subscribe(const bydev_session *session,
                                 bydev_state_callback callback,
                                 void *user_data);

// 구독을 끊는다. 돌아온 뒤로는 그 콜백이 불리지 않는다(다른 스레드에서 진행 중인 호출은 끝날 때까지 기다린다 — 콜백 안에서 부르면 기다리지
// 않는다). 그 뒤로 `user_data`를 놓아도 된다.
void bydev_session_unsubscribe(uint64_t subscription);

// 웹훅 서명을 검증한다(안내서 11장 · Standard Webhooks v1) — 결과 JSON `{"id","timestamp"}`. `body`는 받은 그대로의 본문이다. 시크릿 교체
// 중이면 둘 다(`secrets` · `secret_count`). `now_ms`가 음수면 시스템 시계.
char *bydev_verify_webhook(const char *const *secrets,
                           size_t secret_count,
                           const char *webhook_id,
                           const char *webhook_timestamp,
                           const char *webhook_signature,
                           const uint8_t *body,
                           size_t body_len,
                           int64_t now_ms,
                           bydev_error **error);

// 판매자 서버의 토큰 검증기(안내서 09장) — JWKS를 받아 캐시한다.
bydev_server_verifier *bydev_server_verifier_new(const char *base_url, const char *client_id);

// access_token(`at+jwt`)을 검증한다 — 클레임 JSON.
char *bydev_server_verifier_access_token(const bydev_server_verifier *verifier,
                                         const char *token,
                                         bydev_error **error);

// licenseToken을 검증한다 — 클레임 JSON. `min_generation` — 같은 sub · device_id에서 이미 본 가장 큰 세대번호(0이면 보지 않는다).
char *bydev_server_verifier_license_token(const bydev_server_verifier *verifier,
                                          const char *token,
                                          int64_t min_generation,
                                          bydev_error **error);

void bydev_server_verifier_free(bydev_server_verifier *verifier);

#ifdef __cplusplus
}  // extern "C"
#endif  // __cplusplus

#endif  /* BYDEV_H */
