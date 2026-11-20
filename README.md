# 데브 SDK: C / C++

데브 라이선스 API의 공식 C SDK와 C++ 래퍼입니다. [Rust SDK](https://github.com/developedby-dev/sdk-rust/blob/main/README.md) 위에 C ABI를 얹은 것이라 동작은 Rust SDK와 똑같습니다.

그리고 같은
[적합성 시험 시나리오](https://github.com/developedby-dev/sdk-spec/tree/main/conformance/scenarios)를 **C ABI만으로** 모두 통과합니다.

| 지원 | 버전 |
|---|---|
| 동작 명세 | 1.1.1 |
| API | v1 |
| C | C11 (`include/bydev.h`) |
| C++ | C++17, 헤더 전용 (`include/bydev.hpp`) |
| 운영체제 | Windows, macOS, Linux. 라이브러리는 정적(`libbydev_c.a`, `bydev_c.lib`)과 동적(`.dylib`, `.so`, `.dll`) 두 가지입니다. |

운영체제별로 미리 빌드한 라이브러리와 헤더 파일은 서비스 오픈을 준비하면서 공개합니다. 지금은 이 저장소에서 `cargo build --release`로 빌드하며, 결과물은
`target/release`에 생깁니다.

## 규칙

- **소유권**: SDK가 반환한 `char*`는 `bydev_string_free`로, `bydev_error*`는 `bydev_error_free`로, 핸들은 각자의 `_free` 함수로 해제해 주세요. 인자로 받은
  `const char*`는 그 호출이 끝날 때까지만 유효합니다.

  문자열은 모두 UTF-8이며 NUL로 끝납니다.
- **에러**: 실패할 수 있는 함수는 마지막 인자 `bydev_error** error`(NULL을 넘겨도 됩니다)에 에러를 담고, NULL이나 false, -1을 돌려줍니다. 분기는
  `bydev_error_code()`의 값을 `BYDEV_CODE_*`와 `strcmp`로 비교해서 해 주세요. `bydev_error_message()`는 화면에 보여 주는 용도로만 쓰고, 오류 화면에는
  `bydev_error_correlation_id()`를 함께 보여 주세요.
- **모델은 JSON**: 이용권, 기기, 업데이트 확인 결과, 클레임, 세션 상태는 JSON 문자열로 돌려줍니다. 앱에서 쓰는 JSON 라이브러리로 읽으세요.

  이렇게 하면
  필드가 늘어나도 ABI가 깨지지 않습니다.
  업데이트 확인 결과에 `entitled`가 있으면(업데이트 제공 기한이 있는 구매자) 최상위 대신 `entitled.release`를 따라 주세요. 최상위 최신 버전은 그
  구매자가 실행할 수 없는 버전일 수 있고, `entitled.release`가 `null`이면 그 대상에 제공 범위 안의 버전이 없습니다.
- **블로킹과 스레드**: 모든 함수는 블로킹이며, 어느 스레드에서 불러도 됩니다. 세션 상태 콜백과 포트 콜백은 SDK의 스레드에서 불리므로, UI 작업은 UI 스레드로
  넘겨 처리해 주세요.

## 빠른 시작 (C)

```c
#include "bydev.h"

/* 세션 상태가 바뀔 때마다 SDK의 스레드에서 불립니다 */
static void on_state(void* user_data, int32_t type, const char* state_json) {
  switch (type) {
    case BYDEV_STATE_ACTIVE:
    case BYDEV_STATE_DEGRADED: enable_features(state_json); break;  /* claims.exp와 sid를 씁니다 */
    case BYDEV_STATE_NEEDS_DECISION: ask_user(state_json); break;   /* 사용자가 동의하면 bydev_session_take_over를 부릅니다 */
    case BYDEV_STATE_LOCKED: lock_app(state_json); break;
  }
}

bydev_config config = {0};
config.base_url = "https://api.developedby.dev";
config.client_id = "cli_01J9M0Y6X4N3Q8ZD5R2KT7VCPW";  /* 판매자 콘솔에서 제품마다 발급받습니다 */
config.product_id = "prod_01J9KZ3F7W2X8YQ4N6M5R1TBCD";
config.scopes = "entitlement.read session.write version.read";
config.app_name = "MyApp";  /* 운영체제 보안 저장소와 지문 출처(앱 폴더의 설치 ID)에 씁니다 */
config.app_target = BYDEV_DISTRIBUTION_TARGET_WINDOWS_X64;
config.app_version = "1.4.0";  /* 콘솔에 업로드한 빌드의 버전과 같게 */

bydev_error* error = NULL;
bydev_client* client = bydev_client_new(&config, &error);

if (bydev_client_has_credentials(client, &error) == 0) {
  bydev_login* login = bydev_client_start_login(client, &error);
  show_code(bydev_login_user_code(login), bydev_login_verification_uri_complete(login));  /* 화면에 코드와 QR 코드를 보여 줍니다 */
  bydev_string_free(bydev_login_wait(login, &error));  /* 사용자가 브라우저에서 승인할 때까지 기다립니다 */
  bydev_login_free(login);
}

bydev_session* session = bydev_client_session(client, &error);
uint64_t subscription = bydev_session_subscribe(session, on_state, NULL);
bydev_session_start(session);
/* 앱을 끌 때 */
bydev_session_stop(session);
bydev_session_unsubscribe(subscription);  /* 이 함수가 돌아온 뒤에는 콜백이 불리지 않습니다 */
bydev_session_free(session);
bydev_client_free(client);
```

## 빠른 시작 (C++)

```cpp
#include "bydev.hpp"

bydev::Config config;
config.base_url = "https://api.developedby.dev";
config.client_id = "cli_01J9M0Y6X4N3Q8ZD5R2KT7VCPW";
config.product_id = "prod_01J9KZ3F7W2X8YQ4N6M5R1TBCD";
config.scopes = {"entitlement.read", "session.write", "version.read"};
config.app_name = "MyApp";
config.app_target = BYDEV_DISTRIBUTION_TARGET_WINDOWS_X64;
config.app_version = "1.4.0";  // 콘솔에 업로드한 빌드의 버전과 같게
bydev::Client client(config);

if (!client.has_credentials()) {
  bydev::Login login = client.start_login();
  show_code(login.user_code(), login.verification_uri_complete());
  login.wait();
}
bydev::Session session = client.session();
bydev::Subscription subscription = session.subscribe([](bydev::StateType type, const std::string& state) { /* … */ });
session.start();
// 실패하면 bydev::Error 예외가 납니다. 예: e.code() == BYDEV_CODE_NOT_SIGNED_IN
```

## 포트

`bydev_config.ports`(C 콜백 표인 `bydev_ports`)로 전송, 저장소, 시계, 스케줄러, 난수, 지문, 로그 구현을 교체할 수 있습니다. 포트 묶음의 콜백을 모두
채우면 그 묶음을 쓰고, 그렇지 않으면 기본 구현을 씁니다.

자세한 내용은 헤더 파일의 주석을 보세요. 콜백이 SDK에 넘기는 문자열은 `bydev_string_new`로 만들어야
합니다. `app_name`도 주지 않고 저장소 포트도 없으면 저장소는 메모리가 되어, 앱을 끄면 로그인이 사라집니다.

| 기본 구현 | Windows | macOS | Linux |
|---|---|---|---|
| 저장소(`app_name`) | 자격 증명 관리자(DPAPI로 보호) | 키체인 | Secret Service |
| 지문 출처(`app_name`) | `MachineGuid` | `IOPlatformUUID` | `/etc/machine-id` (못 읽으면 앱 폴더의 설치 ID) |
| 전송 | rustls(시간 초과 30초, 리다이렉트를 따라가지 않음) | | |

Windows 자격 증명 관리자는 비밀번호로 로그인한 세션에서만 열립니다. Windows 서비스나 SSH 키로 로그인한 세션에서는 `STORAGE_FAILED`가 납니다(원인은
`ERROR_NO_SUCH_LOGON_SESSION`입니다). 데스크톱 앱은 문제없이 동작합니다.

그런 환경에서 돌리는 도구라면 파일에 저장하는 저장소 콜백을 넘겨 주세요.

## 판매자 서버

웹훅 서명은 `bydev_verify_webhook`으로 검증하며, 받은 본문을 그대로 넘겨야 합니다. access_token과 licenseToken은 `bydev_server_verifier_*` 함수로 검증하고,
이때 JWKS는 받아서 캐시해 둡니다.

C++에서는 `bydev::verify_webhook`과 `bydev::ServerVerifier`를 사용해 주세요.

## 출처 확인

이 꾸러미는 공개 저장소 `developedby-dev/sdk-c`의 GitHub Actions가 판 태그에서 짓고 올립니다. 출시물의 라이브러리 묶음(`SHA256SUMS`도 함께 있습니다)에는 GitHub 출처 증명이 있습니다. 받은 파일을 `gh attestation verify <파일> --repo developedby-dev/sdk-c`로 확인할 수 있습니다.

## 개발

| 명령 | 하는 일 |
|---|---|
| `cargo build` | 라이브러리와 `include/bydev.h`를 만듭니다. 헤더는 `build.rs`가 cbindgen으로 `src/lib.rs`에서 생성하므로 직접 수정하지 마세요. |
| `pnpm sdk:check c` | rustfmt, clippy, 헤더 파일 최신 여부, C11과 C++17 시험(`-Wall -Wextra -Werror`로 빌드해 실행), 적합성 시나리오 48개(C ABI만 쓰는 하네스)를 확인합니다. |

`include/bydev_generated.h`에는 에러 코드, 열거 값, 세션 상태 종류, 규칙이 들어 있으며, `pnpm --filter @bydev/sdk-codegen generate`로 생성되는 파일입니다.
라이선스: MIT 또는 Apache-2.0.
