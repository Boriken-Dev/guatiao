/*
 * A LIBRARY, written in C, whose providers a host lists and configures
 * before building any of them.
 *
 * Four providers, each a different answer a settings screen draws:
 *
 *   catalog_configured  built from a configuration, carrying its own form
 *                       in `meta` under "form", at a version of its own
 *   catalog_implied     built from a configuration, no form: its schema
 *                       implies one (a section, a masked secret)
 *   catalog_unavailable its own instance, refusing to run here, with why
 *   catalog_plain       its own instance, nothing to configure
 *
 * Each serves the example `counter` kind. Every `create` counts, then
 * refuses, saying whether it was the first call: so a host that lists
 * everything and then asks for one instance proves, from the refusal's
 * words alone, that listing built nothing. `catalog_built` answers the
 * count to a host that can look a symbol up.
 *
 * Links nothing: the schemas and the form are literals in the image.
 */

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include <string.h>

#include "greeter_kind.h"
#include "guatiao.h"

#ifdef _WIN32
#define EXPORT __declspec(dllexport)
#else
#define EXPORT __attribute__((visibility("default")))
#endif

/* ---- counting what was built ----------------------------------------- */

static int BUILT = 0;

/* How many times any `create` here was called. */
EXPORT int catalog_built(void) { return BUILT; }

static const char FIRST[] = "built nothing: the first create this library was asked";
static const char LATER[] = "built nothing: not the first create this library was asked";

static guatiao_status create(void *ctx, const guatiao_value *config, void **out,
                             guatiao_provider_error *err) {
  (void)ctx;
  (void)config;
  BUILT++;
  if (out) {
    *out = NULL;
  }
  if (err) {
    /* A literal: cap 0 and no allocator, so nobody frees it. */
    const char *words = BUILT == 1 ? FIRST : LATER;
    err->status = GUATIAO_ERR_UNSUPPORTED;
    err->message.ptr = (uint8_t *)words;
    err->message.len = strlen(words);
    err->message.cap = 0;
    err->message.alloc = NULL;
  }
  return GUATIAO_ERR_UNSUPPORTED;
}

static void destroy(void *ctx, void *instance) {
  (void)ctx;
  (void)instance;
}

static guatiao_status count(void *ctx, int64_t *out) {
  (void)ctx;
  *out = 0;
  return GUATIAO_OK;
}

static const counter_vtable COUNTER = {
    {(uint32_t)sizeof(counter_vtable), counter_vtable_FLOOR_HASH},
    count,
};

/* ---- catalog_configured: a schema and the form it is shown with ------ */

static guatiao_entry HOST_FIELD[] = {
    GUATIAO_ENTRY_LIT("type", GUATIAO_VALUE_STRING_LIT("string")),
    GUATIAO_ENTRY_LIT("x-section", GUATIAO_VALUE_STRING_LIT("net")),
};
static guatiao_entry PORT_FIELD[] = {
    GUATIAO_ENTRY_LIT("type", GUATIAO_VALUE_STRING_LIT("integer")),
    GUATIAO_ENTRY_LIT("x-section", GUATIAO_VALUE_STRING_LIT("net")),
};
static guatiao_entry NET_PROPERTIES[] = {
    GUATIAO_ENTRY_LIT("host", GUATIAO_VALUE_MAP_LIT(HOST_FIELD)),
    GUATIAO_ENTRY_LIT("port", GUATIAO_VALUE_MAP_LIT(PORT_FIELD)),
};
static guatiao_value NET_REQUIRED[] = {
    GUATIAO_VALUE_STRING_LIT("host"),
    GUATIAO_VALUE_STRING_LIT("port"),
};
static guatiao_entry NET_SCHEMA_ENTRIES[] = {
    GUATIAO_ENTRY_LIT("type", GUATIAO_VALUE_STRING_LIT("object")),
    GUATIAO_ENTRY_LIT("properties", GUATIAO_VALUE_MAP_LIT(NET_PROPERTIES)),
    GUATIAO_ENTRY_LIT("required", GUATIAO_VALUE_LIST_LIT(NET_REQUIRED)),
};
static guatiao_value NET_SCHEMA = GUATIAO_VALUE_MAP_LIT(NET_SCHEMA_ENTRIES);

static guatiao_entry NET_SECTION[] = {
    GUATIAO_ENTRY_LIT("id", GUATIAO_VALUE_STRING_LIT("net")),
    GUATIAO_ENTRY_LIT("title", GUATIAO_VALUE_STRING_LIT("Network")),
};
static guatiao_value SECTIONS[] = {GUATIAO_VALUE_MAP_LIT(NET_SECTION)};
static guatiao_entry PORT_HINTS[] = {
    GUATIAO_ENTRY_LIT("placeholder", GUATIAO_VALUE_STRING_LIT("8080")),
};
static guatiao_entry FORM_FIELDS[] = {
    GUATIAO_ENTRY_LIT("port", GUATIAO_VALUE_MAP_LIT(PORT_HINTS)),
};
static guatiao_entry FORM_ENTRIES[] = {
    GUATIAO_ENTRY_LIT("sections", GUATIAO_VALUE_LIST_LIT(SECTIONS)),
    GUATIAO_ENTRY_LIT("fields", GUATIAO_VALUE_MAP_LIT(FORM_FIELDS)),
};
static guatiao_entry CONFIGURED_META_ENTRIES[] = {
    GUATIAO_ENTRY_LIT("form", GUATIAO_VALUE_MAP_LIT(FORM_ENTRIES)),
};
static guatiao_map CONFIGURED_META = {CONFIGURED_META_ENTRIES, 1, 0, NULL};

/* ---- catalog_implied: a schema and no form --------------------------- */

static guatiao_entry USER_FIELD[] = {
    GUATIAO_ENTRY_LIT("type", GUATIAO_VALUE_STRING_LIT("string")),
    GUATIAO_ENTRY_LIT("x-section", GUATIAO_VALUE_STRING_LIT("account")),
};
static guatiao_entry TOKEN_FIELD[] = {
    GUATIAO_ENTRY_LIT("type", GUATIAO_VALUE_STRING_LIT("string")),
    GUATIAO_ENTRY_LIT("x-section", GUATIAO_VALUE_STRING_LIT("account")),
    GUATIAO_ENTRY_LIT("x-sensitive", GUATIAO_VALUE_BOOL_LIT(true)),
};
static guatiao_entry ACCOUNT_PROPERTIES[] = {
    GUATIAO_ENTRY_LIT("user", GUATIAO_VALUE_MAP_LIT(USER_FIELD)),
    GUATIAO_ENTRY_LIT("token", GUATIAO_VALUE_MAP_LIT(TOKEN_FIELD)),
};
static guatiao_entry ACCOUNT_SCHEMA_ENTRIES[] = {
    GUATIAO_ENTRY_LIT("type", GUATIAO_VALUE_STRING_LIT("object")),
    GUATIAO_ENTRY_LIT("properties", GUATIAO_VALUE_MAP_LIT(ACCOUNT_PROPERTIES)),
};
static guatiao_value ACCOUNT_SCHEMA = GUATIAO_VALUE_MAP_LIT(ACCOUNT_SCHEMA_ENTRIES);

/* ---- catalog_unavailable and catalog_plain: their own instances ------ */

static int INSTANCE = 0;

static bool unavailable(void *ctx, guatiao_str *reason) {
  (void)ctx;
  /* A literal in the image: it outlives every reader. */
  *reason = guatiao_cstr("no device here");
  return false;
}

/* ---- the envelope ---------------------------------------------------- */

static const guatiao_str KINDS[] = {{(const uint8_t *)"counter", 7}};

static const guatiao_kind_table TABLES[] = {{
    (uint32_t)sizeof(guatiao_kind_table),
    (uint32_t)sizeof(counter_vtable),
    {(const uint8_t *)"counter", 7},
    &COUNTER,
}};

static guatiao_provider_info PROVIDERS[4];
static guatiao_library_info LIBRARY;

static void provider(guatiao_provider_info *p, const char *id, const char *name) {
  memset(p, 0, sizeof(*p));
  p->struct_size = (uint32_t)sizeof(guatiao_provider_info);
  p->kinds.ptr = KINDS;
  p->kinds.len = 1;
  p->id = guatiao_cstr(id);
  p->display_name = guatiao_cstr(name);
  p->tables.ptr = TABLES;
  p->tables.len = 1;
  p->tables.stride = sizeof(guatiao_kind_table);
}

/* What a scanner reads before mapping this file. */
EXPORT const char guatiao_declares[] = "kind=counter\0";

EXPORT const guatiao_library_info *
guatiao_library_entry(const guatiao_host_info *host) {
  if (!host || host->abi_version != GUATIAO_ABI_VERSION) {
    return NULL; /* decline a host this file was not built for */
  }
  provider(&PROVIDERS[0], "catalog_configured", "Configured, with a form");
  PROVIDERS[0].config = &NET_SCHEMA;
  PROVIDERS[0].meta = &CONFIGURED_META;
  PROVIDERS[0].version = guatiao_cstr("2.0");
  PROVIDERS[0].create = create;
  PROVIDERS[0].destroy = destroy;

  provider(&PROVIDERS[1], "catalog_implied", "Configured, no form");
  PROVIDERS[1].config = &ACCOUNT_SCHEMA;
  PROVIDERS[1].create = create;
  PROVIDERS[1].destroy = destroy;

  provider(&PROVIDERS[2], "catalog_unavailable", "Needs a device");
  PROVIDERS[2].ctx = &INSTANCE;
  PROVIDERS[2].available = unavailable;

  provider(&PROVIDERS[3], "catalog_plain", "Nothing to set");
  PROVIDERS[3].ctx = &INSTANCE;

  memset(&LIBRARY, 0, sizeof(LIBRARY));
  LIBRARY.struct_size = (uint32_t)sizeof(guatiao_library_info);
  LIBRARY.abi_version = GUATIAO_ABI_VERSION;
  LIBRARY.id = guatiao_cstr("catalog_library");
  LIBRARY.version = guatiao_cstr("1.0");
  LIBRARY.providers.ptr = PROVIDERS;
  LIBRARY.providers.len = 4;
  LIBRARY.providers.stride = sizeof(guatiao_provider_info);
  return &LIBRARY;
}
