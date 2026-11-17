/* C 머리 파일(bydev.h)이 C11 컴파일러로 맞고, 네트워크 없이 도는 부분이 C에서 그대로 도는지 — sdk:check c가 -Wall -Wextra -Werror로 짓고 돌린다 */
#include <stdio.h>
#include <string.h>

#include "bydev.h"

static int failures = 0;

#define CHECK(cond)                                                     \
  do {                                                                  \
    if (!(cond)) {                                                      \
      fprintf(stderr, "실패 %s:%d: %s\n", __FILE__, __LINE__, #cond); \
      failures++;                                                       \
    }                                                                   \
  } while (0)

static void check_code(bydev_error* error, const char* code) {
  CHECK(error != NULL);
  if (error != NULL) {
    CHECK(strcmp(bydev_error_code(error), code) == 0);
    bydev_error_free(error);
  }
}

/* 안내서 11장의 서명 예시 */
static const char* SECRET = "whsec_/MU12AiKByBoT7xt+p81N/uKdp/WxQRb";
static const char* BODY =
    "{\"id\":\"evt_01J9MC4D6E8F0G2H4J6K8M0N2P\",\"type\":\"build.review_completed\",\"occurredAt\":\"2026-10-06T14:00:00Z\","
    "\"mode\":\"LIVE\",\"simulated\":false,\"data\":{\"build\":{\"id\":\"build_01J9M4D6F8H0J2K4M6N8P0R2S4\",\"productId\":"
    "\"prod_01J9KZ3F7W2X8YQ4N6M5R1TBCD\",\"version\":\"1.4.2\",\"reviewedAt\":\"2026-10-06T13:58:00Z\",\"result\":\"APPROVED\"}}}";

static void webhook(void) {
  const char* secrets[] = {SECRET};
  bydev_error* error = NULL;
  char* ok = bydev_verify_webhook(secrets, 1, "evt_01J9MC4D6E8F0G2H4J6K8M0N2P", "1791295205",
                                  "v1,vmsiq2HY4gik/WNJjXJzuESzXAjJgr8Vi13SKwwhgEo=", (const uint8_t*)BODY, strlen(BODY),
                                  (int64_t)1791295215 * 1000, &error);
  CHECK(ok != NULL && strstr(ok, "evt_01J9MC4D6E8F0G2H4J6K8M0N2P") != NULL);
  bydev_string_free(ok);
  char* bad = bydev_verify_webhook(secrets, 1, "evt_01J9MC4D6E8F0G2H4J6K8M0N2P", "1791295205",
                                   "v1,vmsiq2HY4gik/WNJjXJzuESzXAjJgr8Vi13SKwwhgEo=", (const uint8_t*)BODY, strlen(BODY) - 1,
                                   (int64_t)1791295215 * 1000, &error);
  CHECK(bad == NULL);
  check_code(error, BYDEV_CODE_WEBHOOK_INVALID);
}

static void client(void) {
  bydev_error* error = NULL;
  CHECK(bydev_client_new(NULL, &error) == NULL);
  check_code(error, BYDEV_CODE_CONFIGURATION);

  bydev_config config = {0};
  config.base_url = "http://127.0.0.1:1";
  config.client_id = "cli_01J9M0Y6X4N3Q8ZD5R2KT7VCPW";
  config.product_id = "prod_01J9KZ3F7W2X8YQ4N6M5R1TBCD";
  config.scopes = "entitlement.read session.write";
  config.device_name = "smoke";
  config.device_platform = BYDEV_DEVICE_PLATFORM_MACOS;
  /* app_target · app_version은 함께 준다 — 하나만 주면 CONFIGURATION(명세 1절) */
  config.app_target = BYDEV_DISTRIBUTION_TARGET_WINDOWS_X64;
  CHECK(bydev_client_new(&config, &error) == NULL);
  check_code(error, BYDEV_CODE_CONFIGURATION);
  config.app_version = "1.4.0";
  bydev_client* c = bydev_client_new(&config, &error);
  CHECK(c != NULL);
  CHECK(bydev_client_has_credentials(c, &error) == 0);
  /* 저장된 로그인이 없으면 요청 없이 NOT_SIGNED_IN(명세 6절) */
  CHECK(bydev_client_entitlement(c, &error) == NULL);
  check_code(error, BYDEV_CODE_NOT_SIGNED_IN);

  const uint8_t abc[] = {'a', 'b', 'c'};
  CHECK(bydev_client_verify_download(c, abc, 3, "BA7816BF8F01CFEA414140DE5DAE2223B00361A396177A9CB410FF61F20015AD", &error));
  CHECK(!bydev_client_verify_download(c, abc, 2, "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad", &error));
  check_code(error, BYDEV_CODE_DOWNLOAD_INTEGRITY);

  bydev_session* s = bydev_client_session(c, &error);
  CHECK(s != NULL);
  CHECK(bydev_session_state_type(s) == BYDEV_STATE_STOPPED);
  char* state = bydev_session_state(s);
  CHECK(state != NULL && strstr(state, "\"Stopped\"") != NULL);
  bydev_string_free(state);
  bydev_session_free(s);
  bydev_client_free(c);
}

int main(void) {
  CHECK(strcmp(bydev_spec_version(), BYDEV_SPEC_VERSION) == 0);
  CHECK(strlen(bydev_version()) > 0);
  webhook();
  client();
  if (failures == 0) printf("C 시험 통과\n");
  return failures == 0 ? 0 : 1;
}
