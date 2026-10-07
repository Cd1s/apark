// cc crates/ffi/tests/smoke.c -Icrates/ffi/include -Ltarget/debug -lapark_ffi -lm -lpthread -ldl -o /tmp/smoke
#include <stdio.h>
#include <string.h>
#include "apark.h"

static void on_event(const char *e) { printf("event: %.120s\n", e); }

static int call(const char *req) {
    char *r = apark_call(req);
    printf("%s -> %.300s\n", req, r);
    int ok = strstr(r, "\"ok\":true") != NULL;
    apark_free(r);
    return ok;
}

int main(void) {
    apark_set_event_callback(on_event);
    int ok = call("{\"method\":\"info\"}")
          && call("{\"method\":\"accounts\"}")
          && call("{\"method\":\"list\",\"params\":{\"limit\":2}}")
          && call("{\"method\":\"unread_counts\"}")
          && call("{\"method\":\"cached_body\",\"params\":{\"id\":1}}")
          && !call("{\"method\":\"nope\"}")
          && !call("not json");
    printf(ok ? "SMOKE OK\n" : "SMOKE FAIL\n");
    return ok ? 0 : 1;
}
