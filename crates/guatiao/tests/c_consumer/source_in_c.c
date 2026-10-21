/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

/*
 * A SOURCE, written in C: a shelf of counter libraries a host loads and
 * lets go while it runs.
 *
 * The claim: the source table is a C ABI. This file fills a
 * guatiao_source_info, exports guatiao_source and declares itself a
 * source -- and links NOTHING. Every value it answers is a literal (a
 * capacity of 0 owns nothing, so the registry frees nothing of it), every
 * descriptor static.
 *
 * The shelf holds up to three libraries, `shelf_a` to `shelf_c`, each
 * offering one counter (`<id>_counter`, kind `counter`). Its configuration
 * names which are on the shelf at the start (`{"ids": ["shelf_a", ..]}`).
 * The test changes the shelf through the `shelf_*` functions below -- puts
 * one on, takes one off, bumps one's stamp, makes `open` refuse -- and
 * reads back how often each was opened and closed.
 */

#include <string.h>

#include "guatiao.h"

#if defined(_WIN32)
#define EXPORT __declspec(dllexport)
#else
#define EXPORT __attribute__((visibility("default")))
#endif

/* What a scanner reads before mapping this file: a source, never a library. */
EXPORT const char guatiao_declares[] = "source=c-shelf\0";

#define SHELF 3
static const char *IDS[SHELF] = {"shelf_a", "shelf_b", "shelf_c"};
static const char *PROVIDERS[SHELF] = {"shelf_a_counter", "shelf_b_counter", "shelf_c_counter"};

static bool LISTED[SHELF];
static int STAMP[SHELF];
static int OPENED[SHELF];
static int CLOSED[SHELF];
static bool REFUSING = false;
static int64_t COUNTS[SHELF];

/* ---- the controls the test drives ------------------------------------- */

static int index_of(guatiao_str id) {
  for (int i = 0; i < SHELF; i++) {
    if (guatiao_str_eq(id, guatiao_cstr(IDS[i]))) {
      return i;
    }
  }
  return -1;
}

EXPORT void shelf_put(int i) { LISTED[i] = true; }
EXPORT void shelf_take(int i) { LISTED[i] = false; }
EXPORT void shelf_bump(int i) { STAMP[i]++; }
EXPORT void shelf_refuse(bool refuse) { REFUSING = refuse; }
EXPORT int shelf_opened(int i) { return OPENED[i]; }
EXPORT int shelf_closed(int i) { return CLOSED[i]; }

/* ---- the counter each library offers ---------------------------------- */

typedef struct counter_vtable {
  guatiao_kind_header header;
  guatiao_status (*count)(void *ctx, int64_t *out);
} counter_vtable;

/* `counter_vtable_FLOOR_HASH` in greeter_kind.h, written here so this file
 * includes nothing but guatiao.h. */
#define COUNTER_FLOOR_HASH 1227528937u

static guatiao_status shelf_count(void *ctx, int64_t *out) {
  int64_t *n = (int64_t *)ctx;
  *out = ++*n;
  return GUATIAO_OK;
}

static const counter_vtable COUNTER = {
    {(uint32_t)sizeof(counter_vtable), COUNTER_FLOOR_HASH}, shelf_count};

static const guatiao_str KINDS[] = {{(const uint8_t *)"counter", 7}};
static guatiao_kind_table TABLES[SHELF];
static guatiao_provider_info PROVIDER[SHELF];
static guatiao_library_info LIBRARY[SHELF];

/* ---- the list: literals all the way down ------------------------------- */

static char STAMP_TEXT[SHELF][16];
static guatiao_entry FIELDS[SHELF][4];
static guatiao_value KIND_LIST[1];
static guatiao_value CANDIDATES[SHELF];

static guatiao_value text(const char *s, size_t len) {
  guatiao_value v;
  memset(&v, 0, sizeof(v));
  v.tag = (uint32_t)GUATIAO_STRING;
  v.payload.text.ptr = (uint8_t *)s;
  v.payload.text.len = len;
  return v; /* cap 0: owns nothing */
}

static guatiao_string key(const char *s) {
  guatiao_string k = {(uint8_t *)s, strlen(s), 0, NULL};
  return k;
}

static int put_number(char *buf, int n) {
  char rev[16];
  int len = 0;
  do {
    rev[len++] = (char)('0' + n % 10);
    n /= 10;
  } while (n > 0 && len < 15);
  for (int i = 0; i < len; i++) {
    buf[i] = rev[len - 1 - i];
  }
  return len;
}

static guatiao_status shelf_list(void *ctx, const guatiao_host_info *host, guatiao_list *out) {
  (void)ctx;
  (void)host;
  KIND_LIST[0] = text("counter", 7);
  size_t n = 0;
  for (int i = 0; i < SHELF; i++) {
    if (!LISTED[i]) {
      continue;
    }
    int len = put_number(STAMP_TEXT[i], STAMP[i]);
    FIELDS[i][0].key = key("id");
    FIELDS[i][0].value = text(IDS[i], strlen(IDS[i]));
    FIELDS[i][1].key = key("version");
    FIELDS[i][1].value = text("1.0.0", 5);
    FIELDS[i][2].key = key("kinds");
    memset(&FIELDS[i][2].value, 0, sizeof(guatiao_value));
    FIELDS[i][2].value.tag = (uint32_t)GUATIAO_LIST;
    FIELDS[i][2].value.payload.list.ptr = KIND_LIST;
    FIELDS[i][2].value.payload.list.len = 1;
    FIELDS[i][3].key = key("stamp");
    FIELDS[i][3].value = text(STAMP_TEXT[i], (size_t)len);
    memset(&CANDIDATES[n], 0, sizeof(guatiao_value));
    CANDIDATES[n].tag = (uint32_t)GUATIAO_MAP;
    CANDIDATES[n].payload.map.ptr = FIELDS[i];
    CANDIDATES[n].payload.map.len = 4;
    n++;
  }
  /* Written over the carrier, which owns nothing; capacity 0, so nothing
   * here is freed by whoever reads it. */
  out->ptr = CANDIDATES;
  out->len = n;
  out->cap = 0;
  out->alloc = NULL;
  return GUATIAO_OK;
}

static guatiao_status refuse(guatiao_provider_error *err, const char *why) {
  err->status = GUATIAO_ERR_BAD_VALUE;
  err->message.ptr = (uint8_t *)why;
  err->message.len = strlen(why);
  err->message.cap = 0;
  err->message.alloc = NULL;
  return GUATIAO_ERR_BAD_VALUE;
}

static guatiao_status shelf_open(void *ctx, const guatiao_host_info *host, guatiao_str id,
                           const guatiao_library_info **out, guatiao_provider_error *err) {
  (void)ctx;
  (void)host;
  int i = index_of(id);
  if (i < 0) {
    return refuse(err, "nothing by that id is on the shelf");
  }
  if (REFUSING) {
    return refuse(err, "the shelf refuses, says C");
  }
  TABLES[i].struct_size = (uint32_t)sizeof(guatiao_kind_table);
  TABLES[i].vtable_size = (uint32_t)sizeof(counter_vtable);
  TABLES[i].kind = guatiao_cstr("counter");
  TABLES[i].vtable = &COUNTER;

  memset(&PROVIDER[i], 0, sizeof(PROVIDER[i]));
  PROVIDER[i].struct_size = (uint32_t)sizeof(guatiao_provider_info);
  PROVIDER[i].kinds.ptr = KINDS;
  PROVIDER[i].kinds.len = 1;
  PROVIDER[i].id = guatiao_cstr(PROVIDERS[i]);
  PROVIDER[i].ctx = &COUNTS[i];
  PROVIDER[i].tables.ptr = &TABLES[i];
  PROVIDER[i].tables.len = 1;
  PROVIDER[i].tables.stride = sizeof(guatiao_kind_table);

  memset(&LIBRARY[i], 0, sizeof(LIBRARY[i]));
  LIBRARY[i].struct_size = (uint32_t)sizeof(guatiao_library_info);
  LIBRARY[i].abi_version = GUATIAO_ABI_VERSION;
  LIBRARY[i].id = guatiao_cstr(IDS[i]);
  LIBRARY[i].version = guatiao_cstr("1.0.0");
  LIBRARY[i].providers.ptr = &PROVIDER[i];
  LIBRARY[i].providers.len = 1;
  LIBRARY[i].providers.stride = sizeof(guatiao_provider_info);
  OPENED[i]++;
  *out = &LIBRARY[i];
  return GUATIAO_OK;
}

static void shelf_close(void *ctx, guatiao_str id, const guatiao_library_info *info) {
  (void)ctx;
  int i = index_of(id);
  if (i >= 0 && info == &LIBRARY[i]) {
    CLOSED[i]++;
  }
}

static guatiao_source_info SOURCE;

EXPORT guatiao_status guatiao_source(const guatiao_host_info *host, const guatiao_value *config,
                                     const guatiao_source_info **out,
                                     guatiao_provider_error *err) {
  if (!host || host->abi_version != GUATIAO_ABI_VERSION) {
    return refuse(err, "a host this file was not built for");
  }
  const guatiao_value *ids = guatiao_map_find(config, guatiao_cstr("ids"));
  if (!ids || guatiao_tag_of(ids) != (uint32_t)GUATIAO_LIST) {
    return refuse(err, "the configuration names no `ids`");
  }
  guatiao_values items = guatiao_list_items(ids);
  for (size_t k = 0; k < items.len; k++) {
    int i = index_of(guatiao_string_text(&items.ptr[k]));
    if (i < 0) {
      return refuse(err, "the configuration names an id the shelf does not hold");
    }
    LISTED[i] = true;
  }
  memset(&SOURCE, 0, sizeof(SOURCE));
  SOURCE.struct_size = (uint32_t)sizeof(guatiao_source_info);
  SOURCE.abi_version = GUATIAO_ABI_VERSION;
  SOURCE.list = shelf_list;
  SOURCE.open = shelf_open;
  SOURCE.close = shelf_close;
  *out = &SOURCE;
  return GUATIAO_OK;
}
