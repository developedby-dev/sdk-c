// C++ 래퍼(bydev.hpp)가 C++17 컴파일러로 맞고 예외 · RAII가 도는지 — sdk:check c가 -Wall -Wextra -Werror로 짓고 돌린다
#include <cstdio>
#include <cstring>
#include <string>

#include "bydev.hpp"

static int failures = 0;

#define CHECK(cond)                                                       \
  do {                                                                    \
    if (!(cond)) {                                                        \
      std::fprintf(stderr, "실패 %s:%d: %s\n", __FILE__, __LINE__, #cond); \
      failures++;                                                         \
    }                                                                     \
  } while (0)

template <class F>
static std::string code_of(F&& f) {
  try {
    f();
  } catch (const bydev::Error& e) {
    return e.code();
  }
  return "";
}

int main() {
  bydev::Config config;
  config.base_url = "http://127.0.0.1:1";
  config.client_id = "cli_01J9M0Y6X4N3Q8ZD5R2KT7VCPW";
  config.product_id = "prod_01J9KZ3F7W2X8YQ4N6M5R1TBCD";
  config.scopes = {"entitlement.read", "session.write"};
  config.device_name = "smoke";
  config.device_platform = BYDEV_DEVICE_PLATFORM_LINUX;
  config.app_target = BYDEV_DISTRIBUTION_TARGET_LINUX_X64;
  config.app_version = "1.4.0";
  bydev::Client client(config);
  CHECK(!client.has_credentials());
  CHECK(code_of([&] { client.entitlement(); }) == BYDEV_CODE_NOT_SIGNED_IN);
  CHECK(code_of([&] { client.verify_download("ab", "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"); }) ==
        BYDEV_CODE_DOWNLOAD_INTEGRITY);
  client.verify_download("abc", "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");

  bydev::Session session = client.session();
  CHECK(session.state_type() == bydev::StateType::Stopped);
  {
    int calls = 0;
    bydev::Subscription subscription = session.subscribe([&](bydev::StateType, const std::string&) { calls++; });
    session.stop();  // Stopped에서 stop은 아무것도 바꾸지 않는다 — 콜백 없음
    CHECK(calls == 0);
  }  // 구독이 놓이면 끊긴다

  CHECK(code_of([] { bydev::Client bad(bydev::Config{}); }) == BYDEV_CODE_CONFIGURATION);
  CHECK(code_of([&] {
          bydev::Config half = config;
          half.app_version.clear();  // app_target만 — 함께 주어야 한다
          bydev::Client bad(half);
        }) == BYDEV_CODE_CONFIGURATION);
  CHECK(code_of([] { bydev::verify_webhook({"whsec_AAAAAAAAAAAAAAAAAAAAAAAA"}, "evt_x", "1", "v1,AAAA", "{}", 1000); }) ==
        BYDEV_CODE_WEBHOOK_INVALID);
  if (failures == 0) std::printf("C++ 시험 통과\n");
  return failures == 0 ? 0 : 1;
}
