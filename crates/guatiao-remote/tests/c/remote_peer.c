/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

/*
 * The remote protocol spoken from C, over TCP at the trusted level, with
 * guatiao's C API for the values inside frames:
 *
 *   remote_peer client PORT KEY
 *       lists a server's catalog, opens the library serving `greeter`,
 *       calls greet, takes a refusal, and lends a listener the server
 *       calls back during greet_into;
 *   remote_peer server KEY DESCRIPTION
 *       serves one greeter written here to one client; DESCRIPTION is the
 *       greeter kind's description, wire-encoded. Prints `port N` first.
 *
 * A test peer: its nonces are not unpredictable.
 */

#ifdef _WIN32
#define _CRT_SECURE_NO_WARNINGS
#define WIN32_LEAN_AND_MEAN
#include <winsock2.h>
#include <ws2tcpip.h>
typedef SOCKET sock_t;
#define CLOSE_SOCKET closesocket
#else
#include <arpa/inet.h>
#include <netinet/in.h>
#include <netinet/tcp.h>
#include <sys/socket.h>
#include <unistd.h>
typedef int sock_t;
#define CLOSE_SOCKET close
#define INVALID_SOCKET (-1)
#endif

#include <stdio.h>
#include <time.h>

#include "guatiao.h"
#include "remote_codec.h"

static guatiao_alloc ALLOC = GUATIAO_ALLOC_MALLOC;

_Noreturn static void die(const char *what) {
  printf("FAILED: %s\n", what);
  fflush(stdout);
  exit(1);
}

/* ---- the socket ------------------------------------------------------ */

static void send_all(sock_t s, const uint8_t *p, size_t n) {
  while (n) {
    int k = send(s, (const char *)p, (int)n, 0);
    if (k <= 0) {
      die("send");
    }
    p += k;
    n -= (size_t)k;
  }
}

static int recv_all(sock_t s, uint8_t *p, size_t n) {
  while (n) {
    int k = recv(s, (char *)p, (int)n, 0);
    if (k <= 0) {
      return 0;
    }
    p += k;
    n -= (size_t)k;
  }
  return 1;
}

/* One frame's body into `b`; 0 when the peer closed. */
static int read_frame(sock_t s, gr_buf *b) {
  uint8_t len[4];
  if (!recv_all(s, len, 4)) {
    return 0;
  }
  size_t n = (size_t)len[0] | (size_t)len[1] << 8 | (size_t)len[2] << 16 | (size_t)len[3] << 24;
  b->len = 0;
  gr_reserve(b, n);
  if (n && !recv_all(s, b->p, n)) {
    return 0;
  }
  b->len = n;
  return 1;
}

static void write_frame(sock_t s, gr_buf *b) {
  send_all(s, b->p, b->len);
  b->len = 0;
}

static void nonce(uint8_t out[16]) {
  unsigned seed = (unsigned)time(NULL) ^ (unsigned)clock();
  int i;
  for (i = 0; i < 16; i++) {
    seed = seed * 1103515245u + 12345u;
    out[i] = (uint8_t)(seed >> 16);
  }
}

/* An empty map, as an options value: its wire bytes behind their length. */
static void empty_options(gr_buf *b) {
  static const uint8_t empty_map[2] = {GUATIAO_MAP, 0};
  gr_bytes(b, empty_map, 2);
}

/* ---- values ---------------------------------------------------------- */

static guatiao_value decode_value(const uint8_t *p, size_t n) {
  guatiao_value v;
  if (guatiao_wire_decode(&ALLOC, guatiao_bytes_from(p, n), &v) != GUATIAO_OK) {
    die("a value in a frame did not decode");
  }
  return v;
}

/* Appends `v` as a value: its wire bytes behind their length. */
static void put_value(gr_buf *b, const guatiao_value *v) {
  guatiao_value bytes;
  if (guatiao_wire_encode(&ALLOC, v, &bytes) != GUATIAO_OK) {
    die("a value did not encode");
  }
  guatiao_bytes data = guatiao_bytes_data(&bytes);
  gr_bytes(b, data.ptr, data.len);
  guatiao_value_free(&bytes);
}

static int text_eq(const guatiao_value *v, const char *s) {
  return guatiao_str_eq(guatiao_string_text(v), guatiao_cstr(s));
}

static void set_text(guatiao_value *map, const char *key, const char *text) {
  guatiao_value v;
  guatiao_value_string(&ALLOC, guatiao_cstr(text), &v);
  guatiao_map_set(&ALLOC, map, guatiao_cstr(key), &v);
}

/* ---- the client ------------------------------------------------------ */

/* Reads frames until the REPLY to `call`, answering the server's calls
   on the listener this side lent (object 1) as they arrive. */
static void await_reply(sock_t s, gr_buf *in, uint64_t call, uint64_t *status,
                        gr_reader *body, int *heard) {
  for (;;) {
    if (!read_frame(s, in)) {
      die("the server closed the link");
    }
    gr_reader r = {in->p, in->len, 0};
    uint8_t type;
    uint64_t id, parent, target, slot;
    gr_get_byte(&r, &type);
    if (type == 0x11) {
      gr_get_uint(&r, &id);
      gr_get_uint(&r, status);
      if (id != call) {
        die("a reply to another call");
      }
      *body = r;
      return;
    }
    if (type != 0x10) {
      die("a frame the client does not expect");
    }
    /* A call made during ours: the listener's `heard`, on object 1. */
    gr_get_uint(&r, &id);
    gr_get_uint(&r, &parent);
    gr_get_uint(&r, &target);
    gr_get_uint(&r, &slot);
    const uint8_t *text;
    size_t n;
    gr_buf out = {NULL, 0, 0};
    size_t at = gr_start(&out, 0x11);
    gr_uint(&out, id);
    if (parent == call && target == ((1u << 1) | 1u) && slot == 0 &&
        gr_get_bytes(&r, &text, &n) == GR_OK) {
      printf("heard: %.*s\n", (int)n, (const char *)text);
      (*heard)++;
      gr_uint(&out, 0);
    } else {
      gr_uint(&out, GUATIAO_ERR_NOT_FOUND);
      gr_text(&out, "no such object here");
    }
    gr_finish(&out, at);
    write_frame(s, &out);
    gr_free(&out);
  }
}

static int client(const char *port, const char *key) {
  sock_t s = socket(AF_INET, SOCK_STREAM, 0);
  struct sockaddr_in addr;
  memset(&addr, 0, sizeof addr);
  addr.sin_family = AF_INET;
  addr.sin_port = htons((unsigned short)atoi(port));
  addr.sin_addr.s_addr = htonl(INADDR_LOOPBACK);
  if (connect(s, (struct sockaddr *)&addr, sizeof addr) != 0) {
    die("connect");
  }
  int one = 1;
  setsockopt(s, IPPROTO_TCP, TCP_NODELAY, (const char *)&one, sizeof one);

  gr_buf out = {NULL, 0, 0}, in = {NULL, 0, 0};
  uint8_t mine[16], theirs[16], proof[32];
  nonce(mine);

  /* HELLO, trusted. */
  size_t at = gr_start(&out, 0x01);
  gr_raw(&out, "GTRM", 4);
  gr_byte(&out, 1);
  gr_byte(&out, 1);
  gr_raw(&out, mine, 16);
  empty_options(&out);
  gr_finish(&out, at);
  write_frame(s, &out);

  /* WELCOME: the server proves the key. */
  if (!read_frame(s, &in)) {
    die("no WELCOME");
  }
  gr_reader r = {in.p, in.len, 0};
  uint8_t head[6];
  const uint8_t *p;
  size_t n;
  if (gr_get_fixed(&r, head, 6) || memcmp(head, "\x02GTRM\x01", 6) ||
      gr_get_fixed(&r, theirs, 16) || gr_get_bytes(&r, &p, &n) || n != 32) {
    die("not a WELCOME");
  }
  gr_proof((const uint8_t *)key, strlen(key), "guatiao-remote server", mine, theirs, proof);
  if (memcmp(proof, p, 32)) {
    die("the server does not hold the key");
  }
  printf("the server proved the key\n");
  gr_proof((const uint8_t *)key, strlen(key), "guatiao-remote client", mine, theirs, proof);
  at = gr_start(&out, 0x03);
  gr_bytes(&out, proof, 32);
  gr_finish(&out, at);
  write_frame(s, &out);

  /* LIST. */
  uint64_t status;
  gr_reader body;
  int heard = 0;
  at = gr_start(&out, 0x04);
  gr_uint(&out, 1);
  gr_finish(&out, at);
  write_frame(s, &out);
  await_reply(s, &in, 1, &status, &body, &heard);
  if (status != 0 || gr_get_bytes(&body, &p, &n)) {
    die("LIST was refused");
  }
  guatiao_value catalog = decode_value(p, n);

  /* The greeter kind's slots, and the table serving it. */
  int64_t greet = -1, greet_into = -1, table = -1;
  const guatiao_value *library = NULL;
  guatiao_values kinds = guatiao_list_items(guatiao_map_find(&catalog, guatiao_cstr("kinds")));
  size_t i, j, k;
  for (i = 0; i < kinds.len; i++) {
    const guatiao_value *kind = &kinds.ptr[i];
    if (!text_eq(guatiao_map_find(kind, guatiao_cstr("name")), "greeter")) {
      continue;
    }
    guatiao_values slots = guatiao_list_items(guatiao_map_find(kind, guatiao_cstr("slots")));
    for (j = 0; j < slots.len; j++) {
      const guatiao_value *name = guatiao_map_find(&slots.ptr[j], guatiao_cstr("name"));
      if (text_eq(name, "greet")) {
        greet = (int64_t)j;
      } else if (text_eq(name, "greet_into")) {
        greet_into = (int64_t)j;
      }
    }
  }
  guatiao_values libraries =
      guatiao_list_items(guatiao_map_find(&catalog, guatiao_cstr("libraries")));
  for (i = 0; i < libraries.len && table < 0; i++) {
    guatiao_values providers =
        guatiao_list_items(guatiao_map_find(&libraries.ptr[i], guatiao_cstr("providers")));
    for (j = 0; j < providers.len && table < 0; j++) {
      guatiao_values tables =
          guatiao_list_items(guatiao_map_find(&providers.ptr[j], guatiao_cstr("tables")));
      for (k = 0; k < tables.len; k++) {
        if (text_eq(guatiao_map_find(&tables.ptr[k], guatiao_cstr("kind")), "greeter")) {
          table = guatiao_int_or(guatiao_map_find(&tables.ptr[k], guatiao_cstr("table")), -1);
          library = &libraries.ptr[i];
          break;
        }
      }
    }
  }
  if (greet < 0 || greet_into < 0 || table < 0) {
    die("the catalog has no greeter");
  }
  printf("greeter on table %lld, greet is slot %lld\n", (long long)table, (long long)greet);

  /* OPEN the library. */
  guatiao_str id = guatiao_string_text(guatiao_map_find(library, guatiao_cstr("id")));
  guatiao_str stamp = guatiao_string_text(guatiao_map_find(library, guatiao_cstr("stamp")));
  at = gr_start(&out, 0x05);
  gr_uint(&out, 2);
  gr_bytes(&out, id.ptr, id.len);
  gr_bytes(&out, stamp.ptr, stamp.len);
  gr_finish(&out, at);
  write_frame(s, &out);
  await_reply(s, &in, 2, &status, &body, &heard);
  if (status != 0) {
    die("OPEN was refused");
  }

  /* greet("from C"): a map answered. */
  at = gr_start(&out, 0x10);
  gr_uint(&out, 3);
  gr_uint(&out, 0);
  gr_uint(&out, (uint64_t)table << 1);
  gr_uint(&out, (uint64_t)greet);
  gr_text(&out, "from C");
  gr_finish(&out, at);
  write_frame(s, &out);
  await_reply(s, &in, 3, &status, &body, &heard);
  if (status != 0 || gr_get_bytes(&body, &p, &n)) {
    die("greet was refused");
  }
  guatiao_value answer = decode_value(p, n);
  guatiao_str greeting = guatiao_string_text(guatiao_map_find(&answer, guatiao_cstr("greeting")));
  printf("greeting: %.*s\n", (int)greeting.len, (const char *)greeting.ptr);
  guatiao_value_free(&answer);

  /* greet(""): a refusal, with its status and words. */
  at = gr_start(&out, 0x10);
  gr_uint(&out, 4);
  gr_uint(&out, 0);
  gr_uint(&out, (uint64_t)table << 1);
  gr_uint(&out, (uint64_t)greet);
  gr_text(&out, "");
  gr_finish(&out, at);
  write_frame(s, &out);
  await_reply(s, &in, 4, &status, &body, &heard);
  if (status != GUATIAO_ERR_BAD_VALUE || gr_get_bytes(&body, &p, &n)) {
    die("an empty name was not refused");
  }
  printf("refused (%u): %.*s\n", (unsigned)status, (int)n, (const char *)p);

  /* greet_into("C", listener 1, lent): the server calls back during it. */
  at = gr_start(&out, 0x10);
  gr_uint(&out, 5);
  gr_uint(&out, 0);
  gr_uint(&out, (uint64_t)table << 1);
  gr_uint(&out, (uint64_t)greet_into);
  gr_text(&out, "C");
  gr_uint(&out, 1u << 1);
  gr_finish(&out, at);
  write_frame(s, &out);
  await_reply(s, &in, 5, &status, &body, &heard);
  if (status != 0 || heard != 2) {
    die("greet_into did not call the listener back twice");
  }

  /* GOODBYE. */
  at = gr_start(&out, 0x16);
  gr_uint(&out, 0);
  gr_text(&out, "done");
  gr_finish(&out, at);
  write_frame(s, &out);
  guatiao_value_free(&catalog);
  gr_free(&out);
  gr_free(&in);
  CLOSE_SOCKET(s);
  printf("all checks passed\n");
  return 0;
}

/* ---- the server ------------------------------------------------------ */

static guatiao_value catalog_of(const guatiao_value *description) {
  guatiao_value catalog, kinds, kind, libraries, library, providers, provider, tables, table,
      slots, slot, number;
  guatiao_value_map(&ALLOC, &catalog);
  guatiao_value_list(&ALLOC, &kinds);
  guatiao_value_clone(&ALLOC, description, &kind);
  guatiao_list_push(&ALLOC, &kinds, &kind);
  guatiao_map_set(&ALLOC, &catalog, guatiao_cstr("kinds"), &kinds);

  guatiao_value_map(&ALLOC, &table);
  set_text(&table, "kind", "greeter");
  guatiao_value_number(&ALLOC, guatiao_cstr("1"), &number);
  guatiao_map_set(&ALLOC, &table, guatiao_cstr("table"), &number);
  guatiao_value_list(&ALLOC, &slots);
  guatiao_value_string(&ALLOC, guatiao_cstr("greet"), &slot);
  guatiao_list_push(&ALLOC, &slots, &slot);
  guatiao_map_set(&ALLOC, &table, guatiao_cstr("slots"), &slots);
  guatiao_value_list(&ALLOC, &tables);
  guatiao_list_push(&ALLOC, &tables, &table);

  guatiao_value_map(&ALLOC, &provider);
  set_text(&provider, "id", "c_greeter");
  set_text(&provider, "name", "a greeter written in C");
  guatiao_map_set(&ALLOC, &provider, guatiao_cstr("tables"), &tables);
  guatiao_value_list(&ALLOC, &providers);
  guatiao_list_push(&ALLOC, &providers, &provider);

  guatiao_value_map(&ALLOC, &library);
  set_text(&library, "id", "c_library");
  set_text(&library, "version", "1.0.0");
  set_text(&library, "stamp", "c1");
  guatiao_map_set(&ALLOC, &library, guatiao_cstr("providers"), &providers);
  guatiao_value_list(&ALLOC, &libraries);
  guatiao_list_push(&ALLOC, &libraries, &library);
  guatiao_map_set(&ALLOC, &catalog, guatiao_cstr("libraries"), &libraries);
  return catalog;
}

static int server(const char *key, const char *description_path) {
  FILE *f = fopen(description_path, "rb");
  if (!f) {
    die("the description file");
  }
  gr_buf file = {NULL, 0, 0};
  uint8_t chunk[4096];
  size_t got;
  while ((got = fread(chunk, 1, sizeof chunk, f)) > 0) {
    gr_raw(&file, chunk, got);
  }
  fclose(f);
  guatiao_value description = decode_value(file.p, file.len);
  gr_free(&file);
  guatiao_value catalog = catalog_of(&description);

  sock_t l = socket(AF_INET, SOCK_STREAM, 0);
  struct sockaddr_in addr;
  memset(&addr, 0, sizeof addr);
  addr.sin_family = AF_INET;
  addr.sin_addr.s_addr = htonl(INADDR_LOOPBACK);
  if (bind(l, (struct sockaddr *)&addr, sizeof addr) != 0 || listen(l, 1) != 0) {
    die("listen");
  }
  socklen_t len = sizeof addr;
  getsockname(l, (struct sockaddr *)&addr, &len);
  printf("port %u\n", (unsigned)ntohs(addr.sin_port));
  fflush(stdout);
  sock_t s = accept(l, NULL, NULL);
  if (s == INVALID_SOCKET) {
    die("accept");
  }
  int one = 1;
  setsockopt(s, IPPROTO_TCP, TCP_NODELAY, (const char *)&one, sizeof one);

  gr_buf out = {NULL, 0, 0}, in = {NULL, 0, 0};
  uint8_t theirs[16], mine[16], proof[32], head[7];
  const uint8_t *p;
  size_t n;
  nonce(mine);
  if (!read_frame(s, &in)) {
    die("no HELLO");
  }
  gr_reader r = {in.p, in.len, 0};
  if (gr_get_fixed(&r, head, 7) || memcmp(head, "\x01GTRM\x01\x01", 7) ||
      gr_get_fixed(&r, theirs, 16)) {
    die("not a trusted HELLO");
  }
  gr_proof((const uint8_t *)key, strlen(key), "guatiao-remote server", theirs, mine, proof);
  size_t at = gr_start(&out, 0x02);
  gr_raw(&out, "GTRM", 4);
  gr_byte(&out, 1);
  gr_raw(&out, mine, 16);
  gr_bytes(&out, proof, 32);
  empty_options(&out);
  gr_finish(&out, at);
  write_frame(s, &out);
  if (!read_frame(s, &in)) {
    die("no PROOF");
  }
  gr_proof((const uint8_t *)key, strlen(key), "guatiao-remote client", theirs, mine, proof);
  r = (gr_reader){in.p, in.len, 0};
  uint8_t type;
  if (gr_get_byte(&r, &type) || type != 0x03 || gr_get_bytes(&r, &p, &n) || n != 32 ||
      memcmp(p, proof, 32)) {
    die("the client does not hold the key");
  }

  int calls = 0;
  while (read_frame(s, &in)) {
    r = (gr_reader){in.p, in.len, 0};
    uint64_t call, parent, target, slot;
    gr_get_byte(&r, &type);
    switch (type) {
    case 0x04: /* LIST */
      gr_get_uint(&r, &call);
      at = gr_start(&out, 0x11);
      gr_uint(&out, call);
      gr_uint(&out, 0);
      put_value(&out, &catalog);
      gr_finish(&out, at);
      write_frame(s, &out);
      break;
    case 0x05: /* OPEN */
      gr_get_uint(&r, &call);
      at = gr_start(&out, 0x11);
      gr_uint(&out, call);
      gr_uint(&out, 0);
      gr_text(&out, "");
      gr_finish(&out, at);
      write_frame(s, &out);
      break;
    case 0x10: /* CALL */
      gr_get_uint(&r, &call);
      gr_get_uint(&r, &parent);
      gr_get_uint(&r, &target);
      gr_get_uint(&r, &slot);
      at = gr_start(&out, 0x11);
      gr_uint(&out, call);
      if (target != (1u << 1) || slot != 0 || gr_get_bytes(&r, &p, &n)) {
        gr_uint(&out, GUATIAO_ERR_UNSUPPORTED);
        gr_text(&out, "this greeter serves greet alone");
      } else if (n == 0) {
        gr_uint(&out, GUATIAO_ERR_BAD_VALUE);
        gr_text(&out, "nobody to greet, says C");
      } else {
        char words[256];
        guatiao_value answer;
        snprintf(words, sizeof words, "hello, %.*s, from C", (int)n, (const char *)p);
        guatiao_value_map(&ALLOC, &answer);
        set_text(&answer, "greeting", words);
        gr_uint(&out, 0);
        put_value(&out, &answer);
        guatiao_value_free(&answer);
      }
      gr_finish(&out, at);
      write_frame(s, &out);
      calls++;
      break;
    case 0x14: /* PING */
      gr_get_uint(&r, &call);
      at = gr_start(&out, 0x15);
      gr_uint(&out, call);
      gr_finish(&out, at);
      write_frame(s, &out);
      break;
    case 0x16: /* GOODBYE */
      goto done;
    default:
      break;
    }
  }
done:
  printf("served %d calls\n", calls);
  guatiao_value_free(&catalog);
  guatiao_value_free(&description);
  gr_free(&out);
  gr_free(&in);
  CLOSE_SOCKET(s);
  CLOSE_SOCKET(l);
  return 0;
}

int main(int argc, char **argv) {
  setvbuf(stdout, NULL, _IONBF, 0);
#ifdef _WIN32
  WSADATA wsa;
  if (WSAStartup(MAKEWORD(2, 2), &wsa) != 0) {
    die("WSAStartup");
  }
#endif
  if (argc == 4 && strcmp(argv[1], "client") == 0) {
    return client(argv[2], argv[3]);
  }
  if (argc == 4 && strcmp(argv[1], "server") == 0) {
    return server(argv[2], argv[3]);
  }
  printf("usage: remote_peer client PORT KEY | remote_peer server KEY DESCRIPTION\n");
  return 2;
}
