/* Frame pacing client for the screen section: it opens a small window, animates it for a fixed time, drawing one new
 * frame each time the compositor asks for one (wl_surface.frame), and prints when each frame was presented, as the
 * compositor reports it through the presentation-time protocol (wp_presentation). It speaks the Wayland wire protocol
 * itself, so it needs only libc and builds static like hello.c. It draws only its own window and reads nothing from
 * the screen.
 *   frames SECONDS
 * stdout, once the time is up and every frame is accounted for:
 *   clock ID                                            clock of every timestamp below (1 = CLOCK_MONOTONIC)
 *   focus FIRST LAST                                    1 when the window had the focus at its first frame, at the end
 *   p FRAME COMMIT_NS PRESENT_NS REFRESH_NS SEQ FLAGS   a frame that was shown, from wp_presentation_feedback.presented
 *   d FRAME COMMIT_NS                                   a frame that was never shown (discarded)
 *   end FRAMES                                          frames committed
 * Exit status 0, or 2 with the reason on stderr.
 */
#define _GNU_SOURCE
#include <poll.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/socket.h>
#include <sys/un.h>
#include <time.h>
#include <unistd.h>

#define W 480
#define H 270
#define NBUF 3

/* What each object id is. The client picks ids from 1 (wl_display) up, and an id comes back for reuse through
 * wl_display.delete_id. */
enum { NONE, DISPLAY, REGISTRY, SYNC, COMPOSITOR, SHM, WM_BASE, PRESENTATION, SURFACE, XDG_SURFACE, TOPLEVEL, POOL,
       BUFFER, FRAME_CB, FEEDBACK };

struct frame { int64_t commit, present; uint64_t seq; uint32_t refresh, flags; int state; /* 1 shown, 2 discarded */ };

static int sock;
static uint8_t *kind;
static uint32_t *aux, *freed, nids = 1, nfreed, cap;
static uint32_t obuf[4096], olen, ohdr, ibuf[16384];
static size_t ilen;
static int ofd = -1;

static uint32_t g_compositor, g_shm, g_wm, g_presentation, v_compositor;
static uint32_t registry, compositor, shm, wm_base, presentation, surface, xdg_surface, toplevel, buffer[NBUF];
static int globals_done, configured, closed, focus, focus_first = -1, busy[NBUF], waiting_buffer;
static clockid_t clk = CLOCK_MONOTONIC;
static int64_t deadline;
static uint32_t *pixels, nframes, fcap, pending;
static struct frame *frames;

static void die(const char *why) {
  fprintf(stderr, "frames: %s\n", why);
  exit(2);
}

static void *grow(void *p, size_t n) {
  if (!(p = realloc(p, n))) die("out of memory");
  return p;
}

static int64_t now(void) {
  struct timespec t;
  clock_gettime(clk, &t);
  return t.tv_sec * 1000000000LL + t.tv_nsec;
}

static uint32_t new_id(int k, uint32_t a) {
  uint32_t id = nfreed ? freed[--nfreed] : nids++;
  if (id >= cap) {
    cap = cap ? cap * 2 : 1024;
    kind = grow(kind, cap);
    aux = grow(aux, cap * sizeof *aux);
    freed = grow(freed, cap * sizeof *freed);
  }
  kind[id] = k;
  aux[id] = a;
  return id;
}

/* Requests are queued as 32-bit words and sent together; an fd (the pool's memory) goes with them as SCM_RIGHTS. */
static void flush(void) {
  union { char b[CMSG_SPACE(sizeof(int))]; struct cmsghdr align; } c;
  struct iovec iov = { obuf, olen * 4 };
  struct msghdr m = { .msg_iov = &iov, .msg_iovlen = 1 };
  if (!olen) return;
  if (ofd >= 0) {
    m.msg_control = c.b;
    m.msg_controllen = sizeof c.b;
    struct cmsghdr *h = CMSG_FIRSTHDR(&m);
    h->cmsg_level = SOL_SOCKET;
    h->cmsg_type = SCM_RIGHTS;
    h->cmsg_len = CMSG_LEN(sizeof(int));
    memcpy(CMSG_DATA(h), &ofd, sizeof(int));
  }
  if (sendmsg(sock, &m, MSG_NOSIGNAL) != (ssize_t)(olen * 4)) die("cannot write to the compositor");
  olen = 0;
  ofd = -1;
}

static void req(uint32_t id, uint32_t opcode) {
  if (olen > 4096 - 64) flush();
  ohdr = olen;
  obuf[olen++] = id;
  obuf[olen++] = opcode;
}

static void u(uint32_t v) { obuf[olen++] = v; }

static void str(const char *s) {
  uint32_t n = strlen(s) + 1, words = (n + 3) / 4;
  obuf[olen++] = n;
  obuf[olen + words - 1] = 0;
  memcpy(&obuf[olen], s, n);
  olen += words;
}

static void end(void) { obuf[ohdr + 1] |= (olen - ohdr) * 4 << 16; }

static uint32_t bind_global(uint32_t name, const char *iface, uint32_t version, int k) {
  uint32_t id = new_id(k, 0);
  req(registry, 0);
  u(name);
  str(iface);
  u(version);
  u(id);
  end();
  return id;
}

static void draw(int b, uint32_t n) {
  /* A light bar crossing a dark field, 4 px a frame: calm, and every frame differs from the one before. */
  uint32_t *p = pixels + (size_t)b * W * H;
  int x0 = (int)(n * 4 % (W + 40)) - 40;
  for (int y = 0; y < H; y++)
    for (int x = 0; x < W; x++) p[y * W + x] = x >= x0 && x < x0 + 40 ? 0xd8d8d8 : 0x2b2b2b;
}

static void next_frame(void) {
  int b = 0;
  while (b < NBUF && busy[b]) b++;
  if (b == NBUF) {
    waiting_buffer = 1;
    return;
  }
  if (nframes == fcap) frames = grow(frames, (fcap = fcap ? fcap * 2 : 4096) * sizeof *frames);
  draw(b, nframes);
  req(surface, 1); /* attach */
  u(buffer[b]);
  u(0);
  u(0);
  end();
  req(surface, 2); /* damage */
  u(0);
  u(0);
  u(W);
  u(H);
  end();
  req(surface, 3); /* frame: the compositor asks for the next one */
  u(new_id(FRAME_CB, 0));
  end();
  req(presentation, 1); /* feedback for this commit */
  u(surface);
  u(new_id(FEEDBACK, nframes));
  end();
  req(surface, 6); /* commit */
  end();
  busy[b] = 1;
  if (focus_first < 0) focus_first = focus;
  frames[nframes++] = (struct frame){ .commit = now() };
  pending++;
  flush();
}

static void dispatch(uint32_t id, uint32_t op, const uint32_t *a) {
  if (id == 0 || id >= nids) return;
  switch (kind[id]) {
  case DISPLAY:
    if (op == 0) { /* error: object, code, message */
      fprintf(stderr, "frames: protocol error %u on object %u: %s\n", a[1], a[0], (const char *)&a[3]);
      exit(2);
    }
    if (op == 1 && a[0] < nids && (kind[a[0]] == SYNC || kind[a[0]] == FRAME_CB || kind[a[0]] == FEEDBACK)) {
      kind[a[0]] = NONE; /* delete_id: the id may be used again */
      freed[nfreed++] = a[0];
    }
    break;
  case REGISTRY:
    if (op == 0) { /* global: name, interface, version */
      const char *iface = (const char *)&a[2];
      uint32_t version = a[2 + (a[1] + 3) / 4];
      if (!strcmp(iface, "wl_compositor")) g_compositor = a[0], v_compositor = version;
      else if (!strcmp(iface, "wl_shm")) g_shm = a[0];
      else if (!strcmp(iface, "xdg_wm_base")) g_wm = a[0];
      else if (!strcmp(iface, "wp_presentation")) g_presentation = a[0];
    }
    break;
  case SYNC:
    globals_done = 1;
    break;
  case WM_BASE: /* ping: answer, or the window is marked as not responding */
    req(wm_base, 3);
    u(a[0]);
    end();
    flush();
    break;
  case XDG_SURFACE: /* configure: acknowledged now, applied by the next commit */
    req(xdg_surface, 4);
    u(a[0]);
    end();
    if (configured) flush();
    configured = 1;
    break;
  case TOPLEVEL:
    if (op == 0) { /* configure: width, height, states */
      focus = 0;
      for (uint32_t i = 0; i < a[2] / 4; i++) focus |= a[3 + i] == 4; /* XDG_TOPLEVEL_STATE_ACTIVATED */
    } else if (op == 1) {
      closed = 1;
    }
    break;
  case PRESENTATION: /* clock_id */
    clk = a[0];
    break;
  case BUFFER: /* release */
    busy[aux[id]] = 0;
    if (waiting_buffer) {
      waiting_buffer = 0;
      next_frame();
    }
    break;
  case FRAME_CB: /* done: draw the next frame until the time is up */
    if (now() < deadline && !closed) next_frame();
    break;
  case FEEDBACK:
    if (op == 1) { /* presented: tv_sec_hi, tv_sec_lo, tv_nsec, refresh, seq_hi, seq_lo, flags */
      struct frame *f = &frames[aux[id]];
      f->present = (int64_t)(((uint64_t)a[0] << 32 | a[1]) * 1000000000ULL + a[2]);
      f->refresh = a[3];
      f->seq = (uint64_t)a[4] << 32 | a[5];
      f->flags = a[6];
      f->state = 1;
      pending--;
    } else if (op == 2) { /* discarded */
      frames[aux[id]].state = 2;
      pending--;
    }
    break;
  }
}

/* Waits up to timeout_ms for events and handles every complete message. Returns 0 on a timeout. */
static int pump(int timeout_ms) {
  union { char b[CMSG_SPACE(sizeof(int) * 16)]; struct cmsghdr align; } c;
  struct pollfd p = { sock, POLLIN, 0 };
  int r = poll(&p, 1, timeout_ms);
  if (r < 0) die("poll failed");
  if (r == 0) return 0;
  struct iovec iov = { (char *)ibuf + ilen, sizeof ibuf - ilen };
  struct msghdr m = { .msg_iov = &iov, .msg_iovlen = 1, .msg_control = c.b, .msg_controllen = sizeof c.b };
  ssize_t got = recvmsg(sock, &m, MSG_CMSG_CLOEXEC);
  if (got <= 0) die("the compositor closed the connection");
  for (struct cmsghdr *h = CMSG_FIRSTHDR(&m); h; h = CMSG_NXTHDR(&m, h))
    if (h->cmsg_level == SOL_SOCKET && h->cmsg_type == SCM_RIGHTS)
      for (int *f = (int *)CMSG_DATA(h); (char *)(f + 1) <= (char *)h + h->cmsg_len; f++) close(*f);
  ilen += got;
  size_t off = 0;
  while (ilen - off >= 8) {
    const uint32_t *w = (const uint32_t *)((char *)ibuf + off);
    uint32_t size = w[1] >> 16;
    if (size < 8 || size % 4) die("malformed message from the compositor");
    if (ilen - off < size) break;
    dispatch(w[0], w[1] & 0xffff, w + 2);
    off += size;
  }
  memmove(ibuf, (char *)ibuf + off, ilen - off);
  ilen -= off;
  return 1;
}

static void wait_for(const int *flag, const char *why) {
  int64_t limit = now() + 5000000000LL;
  while (!*flag)
    if (!pump(1000) && now() > limit) die(why);
}

int main(int argc, char **argv) {
  double seconds = argc == 2 ? atof(argv[1]) : 0;
  const char *name = getenv("WAYLAND_DISPLAY"), *dir = getenv("XDG_RUNTIME_DIR");
  struct sockaddr_un addr = { .sun_family = AF_UNIX };
  if (seconds <= 0) die("usage: frames SECONDS");
  if (!name || !*name) die("WAYLAND_DISPLAY is not set: no Wayland session");
  int n = name[0] == '/' ? snprintf(addr.sun_path, sizeof addr.sun_path, "%s", name)
                         : snprintf(addr.sun_path, sizeof addr.sun_path, "%s/%s", dir ? dir : "", name);
  if (n < 0 || n >= (int)sizeof addr.sun_path) die("the Wayland socket path is too long");
  sock = socket(AF_UNIX, SOCK_STREAM | SOCK_CLOEXEC, 0);
  if (sock < 0 || connect(sock, (struct sockaddr *)&addr, sizeof addr) < 0) die("cannot connect to the compositor");

  new_id(DISPLAY, 0); /* id 1 */
  registry = new_id(REGISTRY, 0);
  req(1, 1); /* get_registry */
  u(registry);
  end();
  req(1, 0); /* sync: done once every global has been announced */
  u(new_id(SYNC, 0));
  end();
  flush();
  wait_for(&globals_done, "the compositor did not list its globals");
  if (!g_compositor || !g_shm || !g_wm) die("the compositor offers no wl_compositor, wl_shm or xdg_wm_base");
  if (!g_presentation) die("the compositor does not support the presentation-time protocol (wp_presentation)");
  compositor = bind_global(g_compositor, "wl_compositor", v_compositor < 4 ? v_compositor : 4, COMPOSITOR);
  shm = bind_global(g_shm, "wl_shm", 1, SHM);
  wm_base = bind_global(g_wm, "xdg_wm_base", 1, WM_BASE);
  presentation = bind_global(g_presentation, "wp_presentation", 1, PRESENTATION);

  surface = new_id(SURFACE, 0);
  req(compositor, 0); /* create_surface */
  u(surface);
  end();
  xdg_surface = new_id(XDG_SURFACE, 0);
  req(wm_base, 2); /* get_xdg_surface */
  u(xdg_surface);
  u(surface);
  end();
  toplevel = new_id(TOPLEVEL, 0);
  req(xdg_surface, 1); /* get_toplevel */
  u(toplevel);
  end();
  req(toplevel, 2); /* set_title */
  str("arbor bench: frame pacing");
  end();
  req(toplevel, 3); /* set_app_id */
  str("arbor-bench");
  end();
  req(surface, 6); /* commit with no buffer: asks for the first configure */
  end();
  flush();
  wait_for(&configured, "the compositor never configured the window");

  size_t size = (size_t)W * H * 4 * NBUF;
  int fd = memfd_create("frames", MFD_CLOEXEC);
  if (fd < 0 || ftruncate(fd, size) < 0) die("cannot create the shared memory");
  pixels = mmap(NULL, size, PROT_READ | PROT_WRITE, MAP_SHARED, fd, 0);
  if (pixels == MAP_FAILED) die("cannot map the shared memory");
  uint32_t pool = new_id(POOL, 0);
  req(shm, 0); /* create_pool: id, fd (sent alongside), size */
  u(pool);
  u(size);
  end();
  ofd = fd;
  flush();
  close(fd);
  for (int b = 0; b < NBUF; b++) {
    buffer[b] = new_id(BUFFER, b);
    req(pool, 0); /* create_buffer: id, offset, width, height, stride, format */
    u(buffer[b]);
    u(b * W * H * 4);
    u(W);
    u(H);
    u(W * 4);
    u(1); /* WL_SHM_FORMAT_XRGB8888 */
    end();
  }

  deadline = now() + (int64_t)(seconds * 1e9);
  next_frame();
  while (!closed) {
    int64_t t = now();
    if (t >= deadline + 2000000000LL || (t >= deadline && !pending)) break;
    pump(t < deadline ? (int)((deadline - t) / 1000000) + 1 : 100);
  }

  printf("clock %d\nfocus %d %d\n", (int)clk, focus_first, focus);
  for (uint32_t i = 0; i < nframes; i++) {
    struct frame *f = &frames[i];
    if (f->state == 1)
      printf("p %u %lld %lld %u %llu %u\n", i, (long long)f->commit, (long long)f->present, f->refresh,
             (unsigned long long)f->seq, f->flags);
    else if (f->state == 2)
      printf("d %u %lld\n", i, (long long)f->commit);
  }
  printf("end %u\n", nframes);
  return 0;
}
