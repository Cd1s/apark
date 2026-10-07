// Apark core C API. All strings are UTF-8 JSON.
//
//   char *reply = apark_call("{\"method\":\"list\",\"params\":{\"unread\":true}}");
//   ... {"ok":true,"result":[...]} or {"ok":false,"error":"..."}
//   apark_free(reply);
//
// apark_call blocks until done: call it off the UI thread. It is thread-safe.
// Events ({"type":"synced",...}, {"type":"login_url","url":...}) arrive on a
// background thread through the callback.
#ifndef APARK_H
#define APARK_H

#ifdef __cplusplus
extern "C" {
#endif

typedef void (*apark_event_cb)(const char *event_json);

char *apark_call(const char *request_json);
void apark_free(char *s);
void apark_set_event_callback(apark_event_cb cb);

#ifdef __cplusplus
}
#endif

#endif
