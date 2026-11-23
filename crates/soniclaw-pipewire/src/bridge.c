/* Native ABI shim. DSP/storage policy remains in Rust; no allocation in
 * process(). */
#include <pipewire/filter.h>
#include <pipewire/pipewire.h>
#include <spa/param/latency-utils.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#define LIMIT 512
struct sl_info {
  uint32_t id, kind, node, direction;
  char name[256];
};
typedef void (*sl_callback)(void *, const float *const *, float *const *,
                            uint32_t, uint32_t, uint64_t, int64_t, uint32_t);
struct sl_pw {
  struct pw_thread_loop *loop;
  struct pw_context *context;
  struct pw_core *core;
  struct pw_filter *filter;
  struct pw_registry *registry;
  struct spa_hook filter_hook, registry_hook, core_hook;
  void *input[8], *output[8];
  uint32_t channels;
  sl_callback callback;
  void *user;
  struct sl_info objects[LIMIT];
  struct pw_proxy *links[64];
  bool started;
  char error[256];
  uint32_t overflows;
};
static void global(void *data, uint32_t id, uint32_t permissions,
                   const char *type, uint32_t version,
                   const struct spa_dict *props) {
  (void)permissions;
  (void)version;
  struct sl_pw *s = data;
  uint32_t kind = 0;
  const char *name = NULL;
  if (strcmp(type, PW_TYPE_INTERFACE_Node) == 0) {
    kind = 1;
    name = spa_dict_lookup(props, PW_KEY_NODE_NAME);
  } else if (strcmp(type, PW_TYPE_INTERFACE_Port) == 0) {
    kind = 2;
    name = spa_dict_lookup(props, PW_KEY_PORT_NAME);
  } else if (strcmp(type, PW_TYPE_INTERFACE_Link) == 0) {
    kind = 3;
  }
  if (!kind) {
    return;
  }
  for (size_t i = 0; i < LIMIT; i++) {
    if (!s->objects[i].kind || s->objects[i].id == id) {
      struct sl_info *r = &s->objects[i];
      memset(r, 0, sizeof(*r));
      r->id = id;
      r->kind = kind;
      const char *node = spa_dict_lookup(props, PW_KEY_NODE_ID);
      r->node = node ? (uint32_t)strtoul(node, NULL, 10) : id;
      const char *dir = spa_dict_lookup(props, PW_KEY_PORT_DIRECTION);
      r->direction = dir && strcmp(dir, "out") == 0;
      snprintf(r->name, sizeof(r->name), "%s", name ? name : "");
      return;
    }
  }
  s->overflows++;
}
static void removed(void *data, uint32_t id) {
  struct sl_pw *s = data;
  for (size_t i = 0; i < LIMIT; i++)
    if (s->objects[i].id == id)
      s->objects[i].kind = 0;
}
static const struct pw_registry_events registry_events = {
    PW_VERSION_REGISTRY_EVENTS, .global = global, .global_remove = removed};
static void core_error(void *data, uint32_t id, int seq, int res,
                       const char *message) {
  (void)id;
  (void)seq;
  (void)res;
  struct sl_pw *s = data;
  snprintf(s->error, sizeof(s->error), "%s",
           message ? message : "PipeWire core error");
}
static const struct pw_core_events core_events = {PW_VERSION_CORE_EVENTS,
                                                  .error = core_error};
static void process(void *data, struct spa_io_position *position) {
  struct sl_pw *s = data;
  uint64_t n = position->clock.duration;
  if (n == 0 || n > 65536)
    return;
  const float *input[8] = {0};
  float *output[8] = {0};
  for (uint32_t i = 0; i < s->channels; i++) {
    input[i] = pw_filter_get_dsp_buffer(s->input[i], (uint32_t)n);
    output[i] = pw_filter_get_dsp_buffer(s->output[i], (uint32_t)n);
  }
  uint32_t rate = position->clock.rate.num
                      ? position->clock.rate.denom / position->clock.rate.num
                      : 0;
  s->callback(s->user, input, output, (uint32_t)n, rate, position->clock.nsec,
              position->clock.delay, position->clock.flags);
}
static const struct pw_filter_events filter_events = {PW_VERSION_FILTER_EVENTS,
                                                      .process = process};
void sl_pw_init(void) { pw_init(NULL, NULL); }
void sl_pw_destroy(struct sl_pw *s) {
  if (!s)
    return;
  if (s->started)
    pw_thread_loop_stop(s->loop);
  for (size_t i = 0; i < 64; i++)
    if (s->links[i])
      pw_proxy_destroy(s->links[i]);
  if (s->registry) {
    spa_hook_remove(&s->registry_hook);
    pw_proxy_destroy((struct pw_proxy *)s->registry);
  }
  if (s->filter) {
    spa_hook_remove(&s->filter_hook);
    pw_filter_destroy(s->filter);
  }
  if (s->core) {
    spa_hook_remove(&s->core_hook);
    pw_core_disconnect(s->core);
  }
  if (s->context)
    pw_context_destroy(s->context);
  if (s->loop)
    pw_thread_loop_destroy(s->loop);
  free(s);
}
struct sl_pw *sl_pw_create(const char *name, const char *remote,
                           uint32_t channels, uint32_t rate, uint32_t quantum,
                           uint32_t latency, sl_callback callback, void *user) {
  struct sl_pw *s = calloc(1, sizeof(*s));
  if (!s)
    return NULL;
  s->channels = channels;
  s->callback = callback;
  s->user = user;
  s->loop = pw_thread_loop_new(name, NULL);
  if (!s->loop)
    goto fail;
  s->context = pw_context_new(pw_thread_loop_get_loop(s->loop), NULL, 0);
  if (!s->context)
    goto fail;
  s->core = pw_context_connect(
      s->context, pw_properties_new(PW_KEY_REMOTE_NAME, remote, NULL), 0);
  if (!s->core)
    goto fail;
  pw_core_add_listener(s->core, &s->core_hook, &core_events, s);
  char requested[64], rate_string[32];
  snprintf(requested, sizeof(requested), "%u/%u", quantum, rate);
  snprintf(rate_string, sizeof(rate_string), "1/%u", rate);
  s->filter = pw_filter_new(
      s->core, name,
      pw_properties_new(PW_KEY_NODE_NAME, name, PW_KEY_MEDIA_TYPE, "Audio",
                        PW_KEY_MEDIA_CATEGORY, "Filter", PW_KEY_MEDIA_ROLE,
                        "DSP", PW_KEY_NODE_LATENCY, requested, PW_KEY_NODE_RATE,
                        rate_string, PW_KEY_NODE_ALWAYS_PROCESS, "true",
                        PW_KEY_NODE_WANT_DRIVER, "true", NULL));
  if (!s->filter)
    goto fail;
  pw_filter_add_listener(s->filter, &s->filter_hook, &filter_events, s);
  for (uint32_t i = 0; i < channels; i++) {
    char port[32];
    snprintf(port, sizeof(port), "input_%u", i + 1);
    s->input[i] = pw_filter_add_port(
        s->filter, PW_DIRECTION_INPUT, PW_FILTER_PORT_FLAG_MAP_BUFFERS, 1,
        pw_properties_new(PW_KEY_FORMAT_DSP, "32 bit float mono audio",
                          PW_KEY_PORT_NAME, port, NULL),
        NULL, 0);
    snprintf(port, sizeof(port), "output_%u", i + 1);
    s->output[i] = pw_filter_add_port(
        s->filter, PW_DIRECTION_OUTPUT, PW_FILTER_PORT_FLAG_MAP_BUFFERS, 1,
        pw_properties_new(PW_KEY_FORMAT_DSP, "32 bit float mono audio",
                          PW_KEY_PORT_NAME, port, NULL),
        NULL, 0);
    if (!s->input[i] || !s->output[i])
      goto fail;
  }
  s->registry = pw_core_get_registry(s->core, PW_VERSION_REGISTRY, 0);
  if (!s->registry)
    goto fail;
  pw_registry_add_listener(s->registry, &s->registry_hook, &registry_events, s);
  uint8_t bytes[256];
  struct spa_pod_builder b = SPA_POD_BUILDER_INIT(bytes, sizeof(bytes));
  struct spa_process_latency_info info =
      SPA_PROCESS_LATENCY_INFO_INIT(.rate = latency);
  const struct spa_pod *params[] = {
      spa_process_latency_build(&b, SPA_PARAM_ProcessLatency, &info)};
  if (pw_filter_connect(s->filter, PW_FILTER_FLAG_RT_PROCESS, params, 1) < 0)
    goto fail;
  if (pw_thread_loop_start(s->loop) < 0)
    goto fail;
  s->started = true;
  return s;
fail:
  sl_pw_destroy(s);
  return NULL;
}
uint32_t sl_pw_snapshot(struct sl_pw *s, struct sl_info *out, uint32_t capacity,
                        uint32_t *node, int *state, uint32_t *overflows,
                        char *error) {
  pw_thread_loop_lock(s->loop);
  *node = pw_filter_get_node_id(s->filter);
  const char *message = NULL;
  *state = pw_filter_get_state(s->filter, &message);
  *overflows = s->overflows;
  snprintf(error, 256, "%s", message ? message : s->error);
  uint32_t n = 0;
  for (size_t i = 0; i < LIMIT && n < capacity; i++)
    if (s->objects[i].kind)
      out[n++] = s->objects[i];
  pw_thread_loop_unlock(s->loop);
  return n;
}
int sl_pw_link(struct sl_pw *s, uint32_t output, uint32_t input) {
  pw_thread_loop_lock(s->loop);
  int slot = -1;
  for (int i = 0; i < 64; i++)
    if (!s->links[i]) {
      slot = i;
      break;
    }
  if (slot < 0) {
    pw_thread_loop_unlock(s->loop);
    return -1;
  }
  char o[32], in[32];
  snprintf(o, sizeof(o), "%u", output);
  snprintf(in, sizeof(in), "%u", input);
  struct pw_properties *p = pw_properties_new(PW_KEY_LINK_OUTPUT_PORT, o,
                                              PW_KEY_LINK_INPUT_PORT, in, NULL);
  s->links[slot] =
      pw_core_create_object(s->core, "link-factory", PW_TYPE_INTERFACE_Link,
                            PW_VERSION_LINK, &p->dict, 0);
  pw_properties_free(p);
  if (!s->links[slot])
    slot = -1;
  pw_thread_loop_unlock(s->loop);
  return slot;
}
int sl_pw_unlink(struct sl_pw *s, uint32_t slot) {
  if (slot >= 64)
    return -1;
  pw_thread_loop_lock(s->loop);
  if (!s->links[slot]) {
    pw_thread_loop_unlock(s->loop);
    return -1;
  }
  pw_proxy_destroy(s->links[slot]);
  s->links[slot] = NULL;
  pw_thread_loop_unlock(s->loop);
  return 0;
}
