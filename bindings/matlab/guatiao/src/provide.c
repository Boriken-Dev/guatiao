/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

/*
 * A library written in M: its descriptor is built in C memory once and
 * kept for the session, since a host may hold it as long as it lives. The
 * descriptor's own slots (the entry, unload, and each provider's
 * available, create and destroy) are libffi closures over handlers here.
 * The entry and unload answer from C alone, on any thread; the others run
 * M code only on the interpreter's thread during a call into the gateway.
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "kinds.h"

struct gt_library;

typedef struct gt_provider {
  struct gt_library *lib;
  int index; /* the M side's provider record */
} gt_provider;

typedef struct gt_library {
  guatiao_library_info info;
  guatiao_provider_info *providers;
  gt_provider *recs;
  int nproviders;
  int closed;
  long live; /* instances built and not yet destroyed; guarded by gt_lock */
  void *entry_code;
} gt_library;

/* ---- closures over the descriptor's slots -------------------------------------- */

static ffi_cif entry_cif, unload_cif, available_cif, create_cif, destroy_cif;
static ffi_type *entry_args[1], *available_args[2], *create_args[4], *destroy_args[2];
static int cifs_ready = 0;

static int prep_cifs(gt_call *c) {
  if (cifs_ready) return 1;
  entry_args[0] = &ffi_type_pointer;
  available_args[0] = available_args[1] = &ffi_type_pointer;
  for (int i = 0; i < 4; i++) create_args[i] = &ffi_type_pointer;
  destroy_args[0] = destroy_args[1] = &ffi_type_pointer;
  if (ffi_prep_cif(&entry_cif, FFI_DEFAULT_ABI, 1, &ffi_type_pointer, entry_args) != FFI_OK ||
      ffi_prep_cif(&unload_cif, FFI_DEFAULT_ABI, 0, &ffi_type_uint32, NULL) != FFI_OK ||
      ffi_prep_cif(&available_cif, FFI_DEFAULT_ABI, 2, &ffi_type_uint8, available_args) != FFI_OK ||
      ffi_prep_cif(&create_cif, FFI_DEFAULT_ABI, 4, &ffi_type_uint32, create_args) != FFI_OK ||
      ffi_prep_cif(&destroy_cif, FFI_DEFAULT_ABI, 2, &ffi_type_void, destroy_args) != FFI_OK)
    return gt_fail(c, GUATIAO_ERR_INTERNAL, "libffi refused a descriptor's call interface");
  cifs_ready = 1;
  return 1;
}

static void *closure(gt_call *c, ffi_cif *cif, void (*fn)(ffi_cif *, void *, void **, void *), void *user) {
  void *code = NULL;
  ffi_closure *cl = (ffi_closure *)ffi_closure_alloc(sizeof(ffi_closure), &code);
  if (!cl) {
    gt_fail(c, GUATIAO_ERR_ALLOC, "libffi could not allocate a closure");
    return NULL;
  }
  if (ffi_prep_closure_loc(cl, cif, fn, user, code) != FFI_OK) {
    ffi_closure_free(cl);
    gt_fail(c, GUATIAO_ERR_INTERNAL, "libffi refused a closure");
    return NULL;
  }
  return code;
}

static void on_entry(ffi_cif *cif, void *ret, void **args, void *user) {
  (void)cif;
  (void)args;
  gt_library *lib = (gt_library *)user;
  *(void **)ret = lib->closed ? NULL : (void *)&lib->info;
}

static void on_unload(ffi_cif *cif, void *ret, void **args, void *user) {
  (void)cif;
  (void)args;
  gt_library *lib = (gt_library *)user;
  gt_lock();
  long live = lib->live;
  gt_unlock();
  *(ffi_arg *)ret = live > 0 ? GUATIAO_ERR_BUSY : GUATIAO_OK;
}

/* A reason a provider gave, kept for every reader: a host may read it any
 * time after. One copy per distinct text. */
typedef struct reason {
  struct reason *next;
  size_t len;
  char text[1];
} reason;
static reason *reasons = NULL;

static const char *keep_reason(const char *text, size_t *len) {
  size_t n = strlen(text);
  gt_lock();
  for (reason *r = reasons; r; r = r->next) {
    if (r->len == n && memcmp(r->text, text, n) == 0) {
      gt_unlock();
      *len = n;
      return r->text;
    }
  }
  reason *r = (reason *)malloc(sizeof(reason) + n);
  if (r) {
    r->len = n;
    memcpy(r->text, text, n + 1);
    r->next = reasons;
    reasons = r;
  }
  gt_unlock();
  *len = r ? n : 0;
  return r ? r->text : "";
}

static void on_available(ffi_cif *cif, void *ret, void **args, void *user) {
  (void)cif;
  gt_provider *p = (gt_provider *)user;
  guatiao_str *reason_out = *(guatiao_str **)args[1];
  const char *why = NULL;
  char buf[1024];
  if (!gt_may_run_m()) {
    why = GT_WRONG_THREAD;
  } else {
    mxArray *in[1] = {mxCreateDoubleScalar((double)p->index)};
    mxArray *out = NULL;
    char msg[512];
    if (!gt_call_m_quiet("guatiao.internal.available", 1, &out, 1, in, msg, sizeof msg)) {
      snprintf(buf, sizeof buf, "asking whether the provider can run failed: %s", msg);
      why = buf;
    } else if (out && mxIsChar(out) && !mxIsEmpty(out)) {
      gt_call tmp;
      memset(&tmp, 0, sizeof tmp);
      guatiao_str t;
      if (gt_mx_utf8(&tmp, out, "a reason", &t)) snprintf(buf, sizeof buf, "%.*s", (int)t.len, t.ptr);
      else snprintf(buf, sizeof buf, "the provider cannot run here");
      gt_scratch_release(&tmp);
      why = buf;
    }
    mxDestroyArray(in[0]);
    if (out) mxDestroyArray(out);
  }
  if (!why) {
    *(ffi_arg *)ret = 1;
    return;
  }
  size_t len = 0;
  const char *kept = keep_reason(why, &len);
  if (reason_out) *reason_out = guatiao_str_from(kept, len);
  *(ffi_arg *)ret = 0;
}

static void on_create(ffi_cif *cif, void *ret, void **args, void *user) {
  (void)cif;
  gt_provider *p = (gt_provider *)user;
  const guatiao_value *config = *(const guatiao_value **)args[1];
  void **out = *(void ***)args[2];
  guatiao_provider_error *err = *(guatiao_provider_error **)args[3];
  if (out) *out = NULL;
  if (!out) {
    gt_write_err(err, GUATIAO_ERR_NULL, "no slot for the instance");
    *(ffi_arg *)ret = GUATIAO_ERR_NULL;
    return;
  }
  if (!gt_may_run_m()) {
    gt_write_err(err, GUATIAO_ERR_UNSUPPORTED, GT_WRONG_THREAD);
    *(ffi_arg *)ret = GUATIAO_ERR_UNSUPPORTED;
    return;
  }
  guatiao_value null_config;
  memset(&null_config, 0, sizeof null_config);
  null_config.tag = GUATIAO_NULL;
  uint64_t owner = gt_owner_borrow(config ? (guatiao_value *)config : &null_config, &gt_binding_alloc, 1);
  if (!owner) {
    *(ffi_arg *)ret = GUATIAO_ERR_ALLOC;
    return;
  }
  mxArray *in[2] = {mxCreateDoubleScalar((double)p->index), mxCreateDoubleScalar((double)owner)};
  mxArray *res = NULL;
  char msg[1024];
  guatiao_status st = GUATIAO_ERR_INTERNAL;
  double ctx = 0;
  if (!gt_call_m_quiet("guatiao.internal.create", 1, &res, 2, in, msg, sizeof msg)) {
    st = GUATIAO_ERR_INTERNAL;
  } else if (!res || !mxIsCell(res) || mxGetNumberOfElements(res) < 3) {
    snprintf(msg, sizeof msg, "the creator answered something else");
  } else {
    st = (guatiao_status)mxGetScalar(mxGetCell(res, 0));
    const mxArray *m = mxGetCell(res, 1);
    msg[0] = '\0';
    if (m && mxIsChar(m)) {
      gt_call tmp;
      memset(&tmp, 0, sizeof tmp);
      guatiao_str t;
      if (gt_mx_utf8(&tmp, m, "a message", &t)) snprintf(msg, sizeof msg, "%.*s", (int)t.len, t.ptr);
      gt_scratch_release(&tmp);
    }
    ctx = mxGetScalar(mxGetCell(res, 2));
  }
  gt_owner_end(owner);
  mxDestroyArray(in[0]);
  mxDestroyArray(in[1]);
  if (res) mxDestroyArray(res);
  if (st == GUATIAO_OK) {
    *out = (void *)(uintptr_t)ctx;
    gt_lock();
    p->lib->live++;
    gt_unlock();
  } else {
    gt_write_err(err, st, msg);
  }
  *(ffi_arg *)ret = st;
}

static void on_destroy(ffi_cif *cif, void *ret, void **args, void *user) {
  (void)cif;
  (void)ret;
  gt_provider *p = (gt_provider *)user;
  double id = gt_id_of_ctx(*(void **)args[1]);
  if (id < 0) return;
  gt_lock();
  if (p->lib->live > 0) p->lib->live--;
  gt_unlock();
  if (!gt_may_run_m()) {
    gt_queue_destroy(id);
    return;
  }
  mxArray *in[1] = {mxCreateDoubleScalar(id)};
  char msg[512];
  if (!gt_call_m_quiet("guatiao.internal.release_id", 0, NULL, 1, in, msg, sizeof msg))
    fprintf(stderr, "guatiao: releasing an instance failed: %s\n", msg);
  mxDestroyArray(in[0]);
}

/* ---- building the descriptor --------------------------------------------------- */

static int copy_text(gt_call *c, const mxArray *x, guatiao_str *out) {
  guatiao_str t = guatiao_str_from("", 0);
  if (x && !mxIsEmpty(x) && !gt_mx_utf8(c, x, "text", &t)) return 0;
  char *p = (char *)malloc(t.len + 1);
  if (!p) return gt_fail(c, GUATIAO_ERR_ALLOC, "out of memory");
  if (t.len) memcpy(p, t.ptr, t.len);
  p[t.len] = '\0';
  *out = guatiao_str_from(p, t.len);
  return 1;
}

/* A tree the descriptor keeps for the session; NULL for []. */
static int keep_tree(gt_call *c, const mxArray *x, int want_map, guatiao_value **out) {
  *out = NULL;
  if (!x || (mxIsDouble(x) && mxIsEmpty(x))) return 1;
  guatiao_value *v = (guatiao_value *)calloc(1, sizeof(guatiao_value));
  if (!v) return gt_fail(c, GUATIAO_ERR_ALLOC, "out of memory");
  if (!gt_build(c, x, &gt_binding_alloc, v)) {
    free(v);
    return 0;
  }
  if (want_map && v->tag != GUATIAO_MAP) {
    gt_free(v);
    free(v);
    return gt_fail(c, GUATIAO_ERR_BAD_VALUE, "meta is a map (a struct)");
  }
  *out = v;
  return 1;
}

static const mxArray *field(const mxArray *s, mwIndex i, const char *name) { return mxGetField(s, i, name); }

/* library_new(id, version, meta, descs) -> id. `descs` is a struct array,
 * one per provider: id, display_name, version, kinds (a cell of names),
 * tables (a cell of addresses), sizes, schema, meta, available,
 * configured, ctx, index. */
int cmd_library_new(gt_call *c) {
  if (!gt_need(c, GT_LIB_CORE) || !prep_cifs(c)) return 0;
  gt_lock_gateway();
  const mxArray *descs = gt_arg(c, 3);
  if (!descs || !mxIsStruct(descs)) return gt_fail(c, GUATIAO_ERR_BAD_VALUE, "a library's providers are a struct array");
  int n = (int)mxGetNumberOfElements(descs);
  gt_library *lib = (gt_library *)calloc(1, sizeof(gt_library));
  guatiao_provider_info *infos = (guatiao_provider_info *)calloc((size_t)(n ? n : 1), sizeof(guatiao_provider_info));
  gt_provider *recs = (gt_provider *)calloc((size_t)(n ? n : 1), sizeof(gt_provider));
  if (!lib || !infos || !recs) {
    free(lib);
    free(infos);
    free(recs);
    return gt_fail(c, GUATIAO_ERR_ALLOC, "out of memory");
  }
  /* What is built from here is kept for the session, refused or not: a
   * host may hold what it read of an earlier build. */
  lib->providers = infos;
  lib->recs = recs;
  lib->nproviders = n;
  for (int i = 0; i < n; i++) {
    guatiao_provider_info *p = &infos[i];
    gt_provider *rec = &recs[i];
    rec->lib = lib;
    rec->index = (int)mxGetScalar(field(descs, (mwIndex)i, "index"));
    const mxArray *kinds = field(descs, (mwIndex)i, "kinds");
    const mxArray *tables = field(descs, (mwIndex)i, "tables");
    const mxArray *sizes = field(descs, (mwIndex)i, "sizes");
    int nk = kinds && mxIsCell(kinds) ? (int)mxGetNumberOfElements(kinds) : 0;
    if (!tables || !mxIsCell(tables) || (int)mxGetNumberOfElements(tables) != nk || !sizes ||
        (int)mxGetNumberOfElements(sizes) != nk)
      return gt_fail(c, GUATIAO_ERR_BAD_VALUE, "a provider's kinds, tables and sizes go together");
    guatiao_str *names = (guatiao_str *)calloc((size_t)(nk ? nk : 1), sizeof(guatiao_str));
    guatiao_kind_table *kt = (guatiao_kind_table *)calloc((size_t)(nk ? nk : 1), sizeof(guatiao_kind_table));
    if (!names || !kt) return gt_fail(c, GUATIAO_ERR_ALLOC, "out of memory");
    for (int k = 0; k < nk; k++) {
      void *t = NULL;
      if (!copy_text(c, mxGetCell(kinds, (mwIndex)k), &names[k])) return 0;
      if (!gt_mx_addr(mxGetCell(tables, (mwIndex)k), &t) || !t) return gt_fail(c, GUATIAO_ERR_BAD_VALUE, "a table is an address");
      kt[k].struct_size = (uint32_t)sizeof(guatiao_kind_table);
      kt[k].vtable_size = (uint32_t)((const double *)mxGetData(sizes))[k];
      kt[k].kind = names[k];
      kt[k].vtable = t;
    }
    p->struct_size = (uint32_t)sizeof(guatiao_provider_info);
    p->vtable_size = 0;
    p->kinds.ptr = names;
    p->kinds.len = (size_t)nk;
    if (!copy_text(c, field(descs, (mwIndex)i, "id"), &p->id) ||
        !copy_text(c, field(descs, (mwIndex)i, "display_name"), &p->display_name) ||
        !copy_text(c, field(descs, (mwIndex)i, "version"), &p->version))
      return 0;
    guatiao_value *schema = NULL, *meta = NULL;
    if (!keep_tree(c, field(descs, (mwIndex)i, "schema"), 0, &schema) ||
        !keep_tree(c, field(descs, (mwIndex)i, "meta"), 1, &meta))
      return 0;
    p->config = schema;
    p->meta = meta ? &meta->payload.map : NULL;
    p->vtable = NULL;
    double ctx = mxGetScalar(field(descs, (mwIndex)i, "ctx"));
    p->ctx = ctx > 0 ? (void *)(uintptr_t)ctx : NULL;
    p->tables.ptr = kt;
    p->tables.len = (size_t)nk;
    p->tables.stride = sizeof(guatiao_kind_table);
    if (mxIsLogicalScalarTrue(field(descs, (mwIndex)i, "available"))) {
      void *code = closure(c, &available_cif, on_available, rec);
      if (!code) return 0;
      memcpy(&p->available, &code, sizeof code);
    }
    if (mxIsLogicalScalarTrue(field(descs, (mwIndex)i, "configured"))) {
      void *create = closure(c, &create_cif, on_create, rec);
      void *destroy = create ? closure(c, &destroy_cif, on_destroy, rec) : NULL;
      if (!create || !destroy) return 0;
      memcpy(&p->create, &create, sizeof create);
      memcpy(&p->destroy, &destroy, sizeof destroy);
    }
  }
  lib->info.struct_size = (uint32_t)sizeof(guatiao_library_info);
  lib->info.abi_version = GUATIAO_ABI_VERSION;
  if (!copy_text(c, gt_arg(c, 0), &lib->info.id) || !copy_text(c, gt_arg(c, 1), &lib->info.version)) return 0;
  lib->info.providers.ptr = infos;
  lib->info.providers.len = (size_t)n;
  lib->info.providers.stride = sizeof(guatiao_provider_info);
  guatiao_value *lm = NULL;
  if (!keep_tree(c, gt_arg(c, 2), 1, &lm)) return 0;
  lib->info.meta = lm ? &lm->payload.map : NULL;
  void *unload = closure(c, &unload_cif, on_unload, lib);
  if (!unload) return 0;
  memcpy(&lib->info.unload, &unload, sizeof unload);
  lib->entry_code = closure(c, &entry_cif, on_entry, lib);
  if (!lib->entry_code) return 0;
  uint64_t id = gt_handle_add(GT_H_LIBRARY, lib);
  if (!id) return gt_fail(c, GUATIAO_ERR_ALLOC, "out of memory");
  gt_answer(c, 0, mxCreateDoubleScalar((double)id));
  return 1;
}

static int library_arg(gt_call *c, gt_library **lib) {
  return gt_arg_handle(c, 0, GT_H_LIBRARY, "library", (void **)lib);
}

int cmd_library_entry(gt_call *c) {
  gt_library *lib = NULL;
  if (!library_arg(c, &lib)) return 0;
  gt_answer(c, 0, gt_addr_mx(lib->entry_code));
  return 1;
}

/* library_close(id): the entry answers no descriptor from now on. */
int cmd_library_close(gt_call *c) {
  gt_library *lib = NULL;
  if (!library_arg(c, &lib)) return 0;
  lib->closed = 1;
  return 1;
}

int cmd_library_live(gt_call *c) {
  gt_library *lib = NULL;
  if (!library_arg(c, &lib)) return 0;
  gt_lock();
  long live = lib->live;
  gt_unlock();
  gt_answer(c, 0, mxCreateDoubleScalar((double)live));
  return 1;
}

/* The descriptor a library's entry answers, for a test reading it. */
int cmd_library_info(gt_call *c) {
  gt_library *lib = NULL;
  if (!library_arg(c, &lib)) return 0;
  gt_answer(c, 0, gt_addr_mx(&lib->info));
  return 1;
}
