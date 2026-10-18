/* Presentation probe for the toolkits section: loaded into a test program with LD_PRELOAD, it asks the compositor
 * when each new frame of the program's window was shown, through the presentation-time protocol (wp_presentation),
 * the method frames.c uses, and writes one line per frame in frames.c's format to the file named by ARBOR_PROBE. The
 * program and its toolkit are left as they are: the probe sits between them and libwayland-client, sees the requests
 * they send, and adds one request of its own before each commit that brings a new buffer to the window. Its replies
 * arrive on an event queue of its own, which it dispatches in the program's own thread right after the program read
 * from the connection, so the program never sees them and runs no thread more: a thread of the probe's own, waiting
 * to read too, would hold up each of the program's reads until it woke. It reads nothing from the screen.
 *
 * Toolkits that link libwayland-client (GTK, Qt, Mesa) reach the probe through symbol interposition; those that load
 * it with dlopen and dlsym (winit, under Slint and iced) reach it through the probe's own dlsym. Before libwayland
 * 1.24, which has wl_proxy_get_interface, it relies on one detail of the ABI that the headers hide: a wl_proxy starts
 * with its wl_interface pointer, which it checks against wl_proxy_get_class. libwayland calls some of its own
 * functions through the same symbols; the probe passes those inner calls straight on.
 *
 * ARBOR_PROBE, written as the events arrive (all times CLOCK_MONOTONIC ns, the clock Mutter reports in):
 *   start PID NS                                        the probe was loaded into process PID (before its main)
 *   connect PID NS                                      process PID connected to the compositor
 *   ready PID NS                                        the probe bound wp_presentation, two round trips later
 *   clock ID                                            clock of the presentation timestamps (1 = CLOCK_MONOTONIC)
 *   c NS WIDTH HEIGHT ACTIVATED                         the window was configured; ACTIVATED 1 when it has the focus
 *   p FRAME COMMIT_NS PRESENT_NS REFRESH_NS SEQ FLAGS CPU_NS
 *                                                       a frame that was shown, from wp_presentation_feedback.presented;
 *                                                       CPU_NS is the CPU time of the whole program, all its threads,
 *                                                       when the frame was committed
 *   d FRAME COMMIT_NS                                   a frame that was never shown (discarded)
 *   error TEXT                                          why the probe could not watch this program
 */
#define _GNU_SOURCE
#include <dlfcn.h>
#include <errno.h>
#include <fcntl.h>
#include <pthread.h>
#include <stdarg.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <unistd.h>

#define EXPORT __attribute__((visibility("default")))
#define MAX_ARGS 20 /* WL_CLOSURE_MAX_ARGS */

/* libwayland-client's ABI, declared here so that the probe builds without its headers. */
struct wl_message {
  const char *name, *signature;
  const struct wl_interface **types;
};
struct wl_interface {
  const char *name;
  int version, method_count;
  const struct wl_message *methods;
  int event_count;
  const struct wl_message *events;
};
struct wl_array {
  size_t size, alloc;
  void *data;
};
union wl_argument {
  int32_t i;
  uint32_t u;
  int32_t f;
  const char *s;
  void *o;
  uint32_t n;
  struct wl_array *a;
  int32_t h;
};
struct wl_proxy;
struct wl_display;
struct wl_event_queue;
typedef int (*dispatcher_t)(const void *, void *, uint32_t, const struct wl_message *, union wl_argument *);

static void *(*real_dlsym)(void *, const char *);
static void *lib;
#define REAL(ret, name, ...) static ret (*real_##name)(__VA_ARGS__)
REAL(struct wl_display *, wl_display_connect, const char *);
REAL(struct wl_display *, wl_display_connect_to_fd, int);
REAL(void, wl_display_disconnect, struct wl_display *);
REAL(struct wl_event_queue *, wl_display_create_queue, struct wl_display *);
REAL(int, wl_display_roundtrip_queue, struct wl_display *, struct wl_event_queue *);
REAL(int, wl_display_dispatch_queue_pending, struct wl_display *, struct wl_event_queue *);
REAL(int, wl_display_read_events, struct wl_display *);
REAL(int, wl_display_dispatch, struct wl_display *);
REAL(int, wl_display_dispatch_queue, struct wl_display *, struct wl_event_queue *);
REAL(int, wl_display_dispatch_timeout, struct wl_display *, const struct timespec *);
REAL(int, wl_display_dispatch_queue_timeout, struct wl_display *, struct wl_event_queue *, const struct timespec *);
REAL(int, wl_display_roundtrip, struct wl_display *);
REAL(void *, wl_proxy_create_wrapper, void *);
REAL(void, wl_proxy_wrapper_destroy, void *);
REAL(void, wl_proxy_set_queue, struct wl_proxy *, struct wl_event_queue *);
REAL(int, wl_proxy_add_listener, struct wl_proxy *, void (**)(void), void *);
REAL(int, wl_proxy_add_dispatcher, struct wl_proxy *, dispatcher_t, const void *, void *);
REAL(void, wl_proxy_destroy, struct wl_proxy *);
REAL(uint32_t, wl_proxy_get_id, struct wl_proxy *);
REAL(const char *, wl_proxy_get_class, struct wl_proxy *);
REAL(const struct wl_interface *, wl_proxy_get_interface, struct wl_proxy *);
REAL(uint32_t, wl_proxy_get_version, struct wl_proxy *);
REAL(struct wl_proxy *, wl_proxy_marshal_array_flags, struct wl_proxy *, uint32_t, const struct wl_interface *, uint32_t,
     uint32_t, union wl_argument *);
REAL(void, wl_proxy_marshal_array, struct wl_proxy *, uint32_t, union wl_argument *);
REAL(struct wl_proxy *, wl_proxy_marshal_array_constructor, struct wl_proxy *, uint32_t, union wl_argument *,
     const struct wl_interface *);
REAL(struct wl_proxy *, wl_proxy_marshal_array_constructor_versioned, struct wl_proxy *, uint32_t, union wl_argument *,
     const struct wl_interface *, uint32_t);
static const struct wl_interface *registry_iface, *surface_iface, *output_iface;

/* wp_presentation version 1, from presentation-time.xml. */
static const struct wl_interface presentation_iface, feedback_iface;
static const struct wl_interface *feedback_types[] = { NULL, NULL };
static const struct wl_message presentation_requests[] = { { "destroy", "", NULL }, { "feedback", "on", feedback_types } };
static const struct wl_message presentation_events[] = { { "clock_id", "u", NULL } };
static const struct wl_interface presentation_iface = { "wp_presentation", 1, 2, presentation_requests, 1,
                                                        presentation_events };
static const struct wl_interface *sync_types[] = { NULL };
static const struct wl_message feedback_events[] = {
  { "sync_output", "o", sync_types }, { "presented", "uuuuuuu", NULL }, { "discarded", "", NULL } };
static const struct wl_interface feedback_iface = { "wp_presentation_feedback", 1, 0, NULL, 3, feedback_events };

static int out = -1, broken;
static pthread_mutex_t lock = PTHREAD_MUTEX_INITIALIZER;
static struct wl_display *display;
static struct wl_event_queue *queue;
static struct wl_proxy *registry, *presentation;
/* Above 0 while the probe itself calls libwayland, or libwayland one of its own functions: those calls pass straight
 * on, so that each request the program sends is looked at once. */
static __thread int depth;
/* The window: the wl_surface that was given an xdg_toplevel role first, found through its xdg_surface. */
static uint32_t xdg_surface_of[64][2], nxdg, window_surface, attached;
static uint32_t nframes;
static int64_t (*commits)[2]; /* when each frame was committed, and the program's CPU time then */
static size_t ncommits;

static int64_t clock_ns(clockid_t c) {
  struct timespec t;
  clock_gettime(c, &t);
  return t.tv_sec * 1000000000LL + t.tv_nsec;
}

static int64_t now(void) { return clock_ns(CLOCK_MONOTONIC); }

static void say(const char *fmt, ...) {
  char b[256];
  va_list ap;
  va_start(ap, fmt);
  int n = vsnprintf(b, sizeof b, fmt, ap);
  va_end(ap);
  if (out >= 0 && n > 0) (void)!write(out, b, n < (int)sizeof b ? n : (int)sizeof b - 1);
}

static void *find(const char *name) {
  if (!lib) lib = dlopen("libwayland-client.so.0", RTLD_NOW | RTLD_NOLOAD);
  if (!lib) lib = dlopen("libwayland-client.so.0", RTLD_NOW | RTLD_LOCAL);
  void *p = lib ? real_dlsym(lib, name) : NULL;
  if (!p) {
    say("error libwayland-client has no %s\n", name);
    broken = 1;
  }
  return p;
}

#define LOAD(name) (real_##name = find(#name))
static void load(void) {
  static int done;
  if (done) return;
  done = 1;
  LOAD(wl_display_connect), LOAD(wl_display_connect_to_fd), LOAD(wl_display_disconnect);
  LOAD(wl_display_create_queue), LOAD(wl_display_roundtrip_queue), LOAD(wl_display_dispatch_queue_pending);
  LOAD(wl_display_read_events), LOAD(wl_display_dispatch), LOAD(wl_display_dispatch_queue), LOAD(wl_display_roundtrip);
  LOAD(wl_proxy_create_wrapper), LOAD(wl_proxy_wrapper_destroy), LOAD(wl_proxy_set_queue);
  LOAD(wl_proxy_add_listener), LOAD(wl_proxy_add_dispatcher), LOAD(wl_proxy_destroy), LOAD(wl_proxy_get_id);
  LOAD(wl_proxy_get_class), LOAD(wl_proxy_get_version), LOAD(wl_proxy_marshal_array_flags);
  LOAD(wl_proxy_marshal_array), LOAD(wl_proxy_marshal_array_constructor);
  LOAD(wl_proxy_marshal_array_constructor_versioned);
  if (lib) { /* newer: libwayland 1.23 and 1.24 */
    real_wl_display_dispatch_timeout = real_dlsym(lib, "wl_display_dispatch_timeout");
    real_wl_display_dispatch_queue_timeout = real_dlsym(lib, "wl_display_dispatch_queue_timeout");
    real_wl_proxy_get_interface = real_dlsym(lib, "wl_proxy_get_interface");
  }
  registry_iface = find("wl_registry_interface");
  surface_iface = find("wl_surface_interface");
  output_iface = find("wl_output_interface");
  feedback_types[0] = surface_iface;
  feedback_types[1] = &feedback_iface;
  sync_types[0] = output_iface;
}

static void find_dlsym(void) {
  real_dlsym = dlvsym(RTLD_NEXT, "dlsym", "GLIBC_2.34");
  if (!real_dlsym) real_dlsym = dlvsym(RTLD_NEXT, "dlsym", "GLIBC_2.2.5");
}

__attribute__((constructor)) static void init(void) {
  find_dlsym();
  const char *path = getenv("ARBOR_PROBE");
  if (path && *path) out = open(path, O_WRONLY | O_CREAT | O_APPEND | O_CLOEXEC, 0600);
  say("start %d %lld\n", (int)getpid(), (long long)now());
}

/* The interface of a proxy. Before libwayland 1.24, the first field of struct wl_proxy (its wl_object), checked against
 * its class name. The varargs requests cannot be passed on without it, so a program it does not hold for cannot be
 * watched at all. */
static const struct wl_interface *iface(struct wl_proxy *p) {
  if (real_wl_proxy_get_interface) return real_wl_proxy_get_interface(p);
  const struct wl_interface *i = *(const struct wl_interface **)p;
  if (!i || !i->name || strcmp(i->name, real_wl_proxy_get_class(p))) {
    say("error the layout of struct wl_proxy is not the one this probe knows\n");
    fprintf(stderr, "arbor probe: the layout of struct wl_proxy is not the one this probe knows\n");
    abort();
  }
  return i;
}

/* ---- presentation feedback, on the probe's own queue ---- */

static void fb_sync(void *d, struct wl_proxy *p, struct wl_proxy *o) { (void)d, (void)p, (void)o; }
static void fb_presented(void *d, struct wl_proxy *p, uint32_t sec_hi, uint32_t sec_lo, uint32_t nsec, uint32_t refresh,
                         uint32_t seq_hi, uint32_t seq_lo, uint32_t flags) {
  uint32_t f = (uint32_t)(uintptr_t)d;
  pthread_mutex_lock(&lock);
  int64_t commit = f < ncommits ? commits[f][0] : 0, cpu = f < ncommits ? commits[f][1] : 0;
  pthread_mutex_unlock(&lock);
  say("p %u %lld %lld %u %llu %u %lld\n", f, (long long)commit,
      (long long)(((uint64_t)sec_hi << 32 | sec_lo) * 1000000000ULL + nsec), refresh,
      (unsigned long long)((uint64_t)seq_hi << 32 | seq_lo), flags, (long long)cpu);
  real_wl_proxy_destroy(p);
}
static void fb_discarded(void *d, struct wl_proxy *p) {
  uint32_t f = (uint32_t)(uintptr_t)d;
  pthread_mutex_lock(&lock);
  int64_t commit = f < ncommits ? commits[f][0] : 0;
  pthread_mutex_unlock(&lock);
  say("d %u %lld\n", f, (long long)commit);
  real_wl_proxy_destroy(p);
}
static void (*feedback_listener[])(void) = { (void (*)(void))fb_sync, (void (*)(void))fb_presented,
                                             (void (*)(void))fb_discarded };

static void clock_id(void *d, struct wl_proxy *p, uint32_t id) {
  (void)d, (void)p;
  say("clock %u\n", id);
}
static void (*presentation_listener[])(void) = { (void (*)(void))clock_id };

static void global(void *d, struct wl_proxy *r, uint32_t name, const char *interface, uint32_t version) {
  (void)d, (void)version;
  if (presentation || strcmp(interface, "wp_presentation")) return;
  union wl_argument a[4] = { { .u = name }, { .s = "wp_presentation" }, { .u = 1 }, { .n = 0 } };
  presentation = real_wl_proxy_marshal_array_flags(r, 0, &presentation_iface, 1, 0, a); /* wl_registry.bind */
  real_wl_proxy_add_listener(presentation, presentation_listener, NULL);
}
static void global_remove(void *d, struct wl_proxy *r, uint32_t name) { (void)d, (void)r, (void)name; }
static void (*registry_listener[])(void) = { (void (*)(void))global, (void (*)(void))global_remove };

/* After the program read from the connection: the replies to the probe that came with what it read. */
static void mine(struct wl_display *d) {
  if (!d || d != display || !queue || depth) return;
  depth++;
  real_wl_display_dispatch_queue_pending(d, queue);
  depth--;
}

/* Called right after the program connected, before it sends anything: a registry of the probe's own, on its own
 * queue, to bind wp_presentation. */
static void watch(struct wl_display *d) {
  if (display || !d || broken) return;
  display = d;
  say("connect %d %lld\n", (int)getpid(), (long long)now());
  depth++;
  queue = real_wl_display_create_queue(d);
  void *wrapper = real_wl_proxy_create_wrapper(d);
  real_wl_proxy_set_queue(wrapper, queue);
  union wl_argument a[1] = { { .n = 0 } };
  registry = real_wl_proxy_marshal_array_flags(wrapper, 1, registry_iface, real_wl_proxy_get_version(wrapper), 0, a);
  real_wl_proxy_wrapper_destroy(wrapper);
  real_wl_proxy_add_listener(registry, registry_listener, NULL);
  real_wl_display_roundtrip_queue(d, queue); /* the globals: binds wp_presentation */
  real_wl_display_roundtrip_queue(d, queue); /* its clock */
  depth--;
  if (!presentation) {
    say("error the compositor does not support the presentation-time protocol (wp_presentation)\n");
    return;
  }
  say("ready %d %lld\n", (int)getpid(), (long long)now());
}

/* ---- what the program sends ---- */

static uint32_t surface_of_xdg(uint32_t xdg) {
  for (uint32_t i = 0; i < nxdg; i++)
    if (xdg_surface_of[i][0] == xdg) return xdg_surface_of[i][1];
  return 0;
}

enum { OTHER, WINDOW_COMMIT, NEW_XDG_SURFACE };

/* Before a request goes out: what it is to the probe. A request may destroy its proxy, so nothing reads the proxy
 * after it went out. */
static int before(struct wl_proxy *p, uint32_t opcode, union wl_argument *args) {
  if (broken || !presentation || !p) return OTHER;
  const char *name = iface(p)->name;
  if (!strcmp(name, "xdg_wm_base") && opcode == 2 && args[1].o) return NEW_XDG_SURFACE; /* get_xdg_surface */
  if (!strcmp(name, "xdg_surface") && opcode == 1) {                                  /* get_toplevel */
    pthread_mutex_lock(&lock);
    if (!window_surface) window_surface = surface_of_xdg(real_wl_proxy_get_id(p));
    pthread_mutex_unlock(&lock);
  } else if (!strcmp(name, "wl_surface") && (opcode == 1 || opcode == 6)) {
    int kind = OTHER;
    pthread_mutex_lock(&lock);
    if (window_surface && real_wl_proxy_get_id(p) == window_surface) {
      if (opcode == 1) attached = args[0].o != NULL; /* attach: a buffer, or none */
      else if (attached) kind = WINDOW_COMMIT;       /* commit */
    }
    pthread_mutex_unlock(&lock);
    return kind;
  }
  return OTHER;
}

/* After get_xdg_surface went out: which wl_surface the new xdg_surface belongs to. */
static void after(int kind, union wl_argument *args, struct wl_proxy *made) {
  if (kind != NEW_XDG_SURFACE || !made) return;
  pthread_mutex_lock(&lock);
  if (nxdg < 64) {
    xdg_surface_of[nxdg][0] = real_wl_proxy_get_id(made);
    xdg_surface_of[nxdg++][1] = real_wl_proxy_get_id(args[1].o);
  }
  pthread_mutex_unlock(&lock);
}

/* Asks when the commit about to go out is shown. The request is queued before the commit, on the same connection. */
static void ask(struct wl_proxy *surface) {
  pthread_mutex_lock(&lock);
  uint32_t f = nframes++;
  if (f >= ncommits) {
    size_t n = ncommits ? ncommits * 2 : 4096;
    int64_t (*c)[2] = realloc(commits, n * sizeof *c);
    if (c) commits = c, ncommits = n;
  }
  if (f < ncommits) commits[f][0] = now(), commits[f][1] = clock_ns(CLOCK_PROCESS_CPUTIME_ID);
  attached = 0;
  pthread_mutex_unlock(&lock);
  union wl_argument a[2] = { { .o = surface }, { .n = 0 } };
  struct wl_proxy *fb = real_wl_proxy_marshal_array_flags(presentation, 1, &feedback_iface, 1, 0, a);
  if (fb) real_wl_proxy_add_listener(fb, feedback_listener, (void *)(uintptr_t)f);
}

/* The varargs requests, turned into the array form libwayland itself turns them into. */
static void from_va(struct wl_proxy *p, uint32_t opcode, union wl_argument *args, va_list ap) {
  const char *sig = iface(p)->methods[opcode].signature;
  for (int n = 0; *sig && n < MAX_ARGS; sig++) {
    switch (*sig) {
    case 'i': case 'h': args[n++].i = va_arg(ap, int32_t); break;
    case 'u': args[n++].u = va_arg(ap, uint32_t); break;
    case 'f': args[n++].f = va_arg(ap, int32_t); break;
    case 's': args[n++].s = va_arg(ap, const char *); break;
    case 'o': args[n++].o = va_arg(ap, void *); break;
    case 'n': args[n++].o = va_arg(ap, void *); break;
    case 'a': args[n++].a = va_arg(ap, struct wl_array *); break;
    default: break; /* '?' and the version digits */
    }
  }
}

EXPORT struct wl_proxy *wl_proxy_marshal_array_flags(struct wl_proxy *p, uint32_t opcode, const struct wl_interface *in,
                                                     uint32_t version, uint32_t flags, union wl_argument *args) {
  load();
  if (depth) return real_wl_proxy_marshal_array_flags(p, opcode, in, version, flags, args);
  depth++;
  int kind = before(p, opcode, args);
  if (kind == WINDOW_COMMIT) ask(p);
  struct wl_proxy *r = real_wl_proxy_marshal_array_flags(p, opcode, in, version, flags, args);
  after(kind, args, r);
  depth--;
  return r;
}

EXPORT struct wl_proxy *wl_proxy_marshal_flags(struct wl_proxy *p, uint32_t opcode, const struct wl_interface *in,
                                               uint32_t version, uint32_t flags, ...) {
  union wl_argument args[MAX_ARGS];
  va_list ap;
  load();
  va_start(ap, flags);
  from_va(p, opcode, args, ap);
  va_end(ap);
  return wl_proxy_marshal_array_flags(p, opcode, in, version, flags, args);
}

EXPORT void wl_proxy_marshal_array(struct wl_proxy *p, uint32_t opcode, union wl_argument *args) {
  load();
  if (depth) {
    real_wl_proxy_marshal_array(p, opcode, args);
    return;
  }
  depth++;
  if (before(p, opcode, args) == WINDOW_COMMIT) ask(p);
  real_wl_proxy_marshal_array(p, opcode, args);
  depth--;
}

EXPORT void wl_proxy_marshal(struct wl_proxy *p, uint32_t opcode, ...) {
  union wl_argument args[MAX_ARGS];
  va_list ap;
  load();
  va_start(ap, opcode);
  from_va(p, opcode, args, ap);
  va_end(ap);
  wl_proxy_marshal_array(p, opcode, args);
}

EXPORT struct wl_proxy *wl_proxy_marshal_array_constructor_versioned(struct wl_proxy *p, uint32_t opcode,
                                                                     union wl_argument *args,
                                                                     const struct wl_interface *in, uint32_t version) {
  load();
  if (depth) return real_wl_proxy_marshal_array_constructor_versioned(p, opcode, args, in, version);
  depth++;
  int kind = before(p, opcode, args);
  if (kind == WINDOW_COMMIT) ask(p);
  struct wl_proxy *r = real_wl_proxy_marshal_array_constructor_versioned(p, opcode, args, in, version);
  after(kind, args, r);
  depth--;
  return r;
}

EXPORT struct wl_proxy *wl_proxy_marshal_array_constructor(struct wl_proxy *p, uint32_t opcode,
                                                           union wl_argument *args, const struct wl_interface *in) {
  load();
  if (depth) return real_wl_proxy_marshal_array_constructor(p, opcode, args, in);
  depth++;
  int kind = before(p, opcode, args);
  if (kind == WINDOW_COMMIT) ask(p);
  struct wl_proxy *r = real_wl_proxy_marshal_array_constructor(p, opcode, args, in);
  after(kind, args, r);
  depth--;
  return r;
}

EXPORT struct wl_proxy *wl_proxy_marshal_constructor(struct wl_proxy *p, uint32_t opcode,
                                                     const struct wl_interface *in, ...) {
  union wl_argument args[MAX_ARGS];
  va_list ap;
  load();
  va_start(ap, in);
  from_va(p, opcode, args, ap);
  va_end(ap);
  return wl_proxy_marshal_array_constructor(p, opcode, args, in);
}

EXPORT struct wl_proxy *wl_proxy_marshal_constructor_versioned(struct wl_proxy *p, uint32_t opcode,
                                                               const struct wl_interface *in, uint32_t version, ...) {
  union wl_argument args[MAX_ARGS];
  va_list ap;
  load();
  va_start(ap, version);
  from_va(p, opcode, args, ap);
  va_end(ap);
  return wl_proxy_marshal_array_constructor_versioned(p, opcode, args, in, version);
}

/* ---- what the window is told: its size and whether it has the focus (xdg_toplevel.configure) ---- */

static void configured(struct wl_array *states, int32_t w, int32_t h) {
  int active = 0;
  for (size_t k = 0; states && k < states->size / 4; k++) active |= ((uint32_t *)states->data)[k] == 4;
  say("c %lld %d %d %d\n", (long long)now(), w, h, active); /* 4: XDG_TOPLEVEL_STATE_ACTIVATED */
}

struct toplevel {
  struct wl_proxy *proxy;
  void (**listener)(void);
  dispatcher_t dispatcher;
  const void *implementation;
};
static struct toplevel toplevels[8];
static int ntoplevels;

static struct toplevel *toplevel_of(struct wl_proxy *p) {
  for (int k = 0; k < ntoplevels; k++)
    if (toplevels[k].proxy == p) return &toplevels[k];
  return NULL;
}

static void tl_configure(void *data, struct wl_proxy *p, int32_t w, int32_t h, struct wl_array *states) {
  configured(states, w, h);
  struct toplevel *t = toplevel_of(p);
  if (t && t->listener[0]) ((void (*)(void *, struct wl_proxy *, int32_t, int32_t, struct wl_array *))t->listener[0])(
      data, p, w, h, states);
}

EXPORT int wl_proxy_add_listener(struct wl_proxy *p, void (**listener)(void), void *data) {
  load();
  const struct wl_interface *i = broken || !p ? NULL : iface(p);
  if (i && !strcmp(i->name, "xdg_toplevel") && i->event_count > 0 && ntoplevels < 8) {
    void (**mine)(void) = malloc(i->event_count * sizeof *mine);
    if (mine) {
      memcpy(mine, listener, i->event_count * sizeof *mine);
      toplevels[ntoplevels++] = (struct toplevel){ p, listener, NULL, NULL };
      mine[0] = (void (*)(void))tl_configure;
      return real_wl_proxy_add_listener(p, mine, data);
    }
  }
  return real_wl_proxy_add_listener(p, listener, data);
}

static int tl_dispatch(const void *impl, void *target, uint32_t opcode, const struct wl_message *msg,
                       union wl_argument *args) {
  const struct toplevel *t = impl;
  if (opcode == 0) configured(args[2].a, args[0].i, args[1].i);
  return t->dispatcher(t->implementation, target, opcode, msg, args);
}

EXPORT int wl_proxy_add_dispatcher(struct wl_proxy *p, dispatcher_t dispatcher, const void *implementation,
                                   void *data) {
  load();
  const struct wl_interface *i = broken || !p ? NULL : iface(p);
  if (i && !strcmp(i->name, "xdg_toplevel") && ntoplevels < 8) {
    struct toplevel *t = &toplevels[ntoplevels++];
    *t = (struct toplevel){ p, NULL, dispatcher, implementation };
    return real_wl_proxy_add_dispatcher(p, tl_dispatch, t, data);
  }
  return real_wl_proxy_add_dispatcher(p, dispatcher, implementation, data);
}

/* ---- connecting and disconnecting ---- */

EXPORT struct wl_display *wl_display_connect(const char *name) {
  load();
  depth++;
  struct wl_display *d = real_wl_display_connect(name);
  depth--;
  if (!depth) watch(d);
  return d;
}

EXPORT struct wl_display *wl_display_connect_to_fd(int fd) {
  load();
  struct wl_display *d = real_wl_display_connect_to_fd(fd);
  if (!depth) watch(d);
  return d;
}

EXPORT void wl_display_disconnect(struct wl_display *d) {
  load();
  if (d && d == display) display = NULL, queue = NULL; /* its proxies go with the connection */
  real_wl_display_disconnect(d);
}

/* ---- reading: the program's reads bring the probe's replies too ---- */

EXPORT int wl_display_read_events(struct wl_display *d) {
  load();
  int r = real_wl_display_read_events(d);
  mine(d);
  return r;
}

EXPORT int wl_display_dispatch(struct wl_display *d) {
  load();
  int r = real_wl_display_dispatch(d);
  mine(d);
  return r;
}

EXPORT int wl_display_dispatch_queue(struct wl_display *d, struct wl_event_queue *q) {
  load();
  int r = real_wl_display_dispatch_queue(d, q);
  mine(d);
  return r;
}

EXPORT int wl_display_dispatch_timeout(struct wl_display *d, const struct timespec *t) {
  load();
  if (!real_wl_display_dispatch_timeout) return errno = ENOSYS, -1;
  int r = real_wl_display_dispatch_timeout(d, t);
  mine(d);
  return r;
}

EXPORT int wl_display_dispatch_queue_timeout(struct wl_display *d, struct wl_event_queue *q,
                                             const struct timespec *t) {
  load();
  if (!real_wl_display_dispatch_queue_timeout) return errno = ENOSYS, -1;
  int r = real_wl_display_dispatch_queue_timeout(d, q, t);
  mine(d);
  return r;
}

EXPORT int wl_display_roundtrip(struct wl_display *d) {
  load();
  int r = real_wl_display_roundtrip(d);
  mine(d);
  return r;
}

EXPORT int wl_display_roundtrip_queue(struct wl_display *d, struct wl_event_queue *q) {
  load();
  int r = real_wl_display_roundtrip_queue(d, q);
  mine(d);
  return r;
}

/* Programs that load libwayland-client with dlopen find the functions above through dlsym. */
static void *own(const char *name) {
  static const struct { const char *name; void *fn; } table[] = {
    { "wl_display_connect", (void *)wl_display_connect },
    { "wl_display_connect_to_fd", (void *)wl_display_connect_to_fd },
    { "wl_display_disconnect", (void *)wl_display_disconnect },
    { "wl_proxy_marshal_array_flags", (void *)wl_proxy_marshal_array_flags },
    { "wl_proxy_marshal_flags", (void *)wl_proxy_marshal_flags },
    { "wl_proxy_marshal_array", (void *)wl_proxy_marshal_array },
    { "wl_proxy_marshal", (void *)wl_proxy_marshal },
    { "wl_proxy_marshal_array_constructor", (void *)wl_proxy_marshal_array_constructor },
    { "wl_proxy_marshal_array_constructor_versioned", (void *)wl_proxy_marshal_array_constructor_versioned },
    { "wl_proxy_marshal_constructor", (void *)wl_proxy_marshal_constructor },
    { "wl_proxy_marshal_constructor_versioned", (void *)wl_proxy_marshal_constructor_versioned },
    { "wl_proxy_add_listener", (void *)wl_proxy_add_listener },
    { "wl_proxy_add_dispatcher", (void *)wl_proxy_add_dispatcher },
    { "wl_display_read_events", (void *)wl_display_read_events },
    { "wl_display_dispatch", (void *)wl_display_dispatch },
    { "wl_display_dispatch_queue", (void *)wl_display_dispatch_queue },
    { "wl_display_dispatch_timeout", (void *)wl_display_dispatch_timeout },
    { "wl_display_dispatch_queue_timeout", (void *)wl_display_dispatch_queue_timeout },
    { "wl_display_roundtrip", (void *)wl_display_roundtrip },
    { "wl_display_roundtrip_queue", (void *)wl_display_roundtrip_queue },
  };
  for (size_t k = 0; k < sizeof table / sizeof *table; k++)
    if (!strcmp(name, table[k].name)) return table[k].fn;
  return NULL;
}

/* Other names go on as a tail call, so that libc's dlsym still sees its real caller, which decides what RTLD_NEXT and
 * RTLD_DEFAULT search. */
#if defined(__has_attribute) && __has_attribute(musttail)
#define TAIL __attribute__((musttail))
#else
#define TAIL
#endif
EXPORT void *dlsym(void *handle, const char *name) {
  if (!real_dlsym) find_dlsym();
  void *mine = own(name);
  if (!mine) TAIL return real_dlsym(handle, name);
  return real_dlsym(handle, name) ? mine : NULL;
}
