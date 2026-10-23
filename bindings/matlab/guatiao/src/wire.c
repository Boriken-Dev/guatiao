/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

/*
 * The wire encoding and channels: a value as bytes, the frames that
 * announce a schema, carry a value and end a channel, and a receiver that
 * checks every value against its channel's schema. A refusal is recorded
 * with the refusal's name as the identifier's third part, or answered as
 * data when the caller asks for it; neither quotes what was received.
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "gt.h"

/* A receiver and the allocator everything it answers is built in. */
typedef struct gt_receiver {
  guatiao_wire_receiver *recv;
  const guatiao_alloc *alloc;
} gt_receiver;

static const char *const kind_names[3] = {"schema", "value", "close"};

/* A uint8 array (or an empty numeric one) as a view of its bytes, valid
 * for the call. */
static int arg_bytes(gt_call *c, int i, const char *what, guatiao_bytes *out) {
  const mxArray *a = gt_arg(c, i);
  out->ptr = NULL;
  out->len = 0;
  if (a && mxIsNumeric(a) && mxIsEmpty(a)) return 1;
  if (!a || !mxIsUint8(a) || mxIsComplex(a) || mxIsSparse(a))
    return gt_fail(c, GUATIAO_ERR_BAD_VALUE, "%s is a uint8 vector", what);
  out->len = mxGetNumberOfElements(a);
  out->ptr = out->len ? (const uint8_t *)mxGetData(a) : NULL;
  return 1;
}

/* A channel: a uint64, or a whole double from 0 to 2^53. */
static int arg_channel(gt_call *c, int i, uint64_t *out) {
  const mxArray *a = gt_arg(c, i);
  if (a && mxIsUint64(a) && !mxIsComplex(a) && mxGetNumberOfElements(a) == 1) {
    *out = *(const uint64_t *)mxGetData(a);
    return 1;
  }
  double d = 0;
  if (!gt_arg_double(c, i, "the channel", &d)) return 0;
  if (!(d >= 0 && d <= 9007199254740992.0 && d == (double)(uint64_t)d))
    return gt_fail(c, GUATIAO_ERR_BAD_VALUE, "a channel is a whole number from 0 to 2^64 - 1 (a uint64 past 2^53)");
  *out = (uint64_t)d;
  return 1;
}

static mxArray *bytes_mx(const guatiao_value *v) {
  guatiao_bytes b = guatiao_bytes_data(v);
  mxArray *x = mxCreateNumericMatrix(1, (mwSize)b.len, mxUINT8_CLASS, mxREAL);
  if (b.len) memcpy(mxGetData(x), b.ptr, b.len);
  return x;
}

static mxArray *uint64_mx(uint64_t n) {
  mxArray *x = mxCreateNumericMatrix(1, 1, mxUINT64_CLASS, mxREAL);
  *(uint64_t *)mxGetData(x) = n;
  return x;
}

/* A string node's text into `buf`, cut to fit; "" for any other kind. */
static void text_into(const guatiao_value *v, char *buf, size_t n) {
  guatiao_str t = guatiao_string_text(v);
  size_t len = t.len < n - 1 ? t.len : n - 1;
  if (len) memcpy(buf, t.ptr, len);
  buf[len] = '\0';
}

/* wire_encode(id, path) -> the node's bytes, a uint8 row */
int cmd_wire_encode(gt_call *c) {
  guatiao_value *v = NULL;
  if (!gt_arg_node(c, 0, NULL, &v) || !GT_NEED_CORE(c, guatiao_wire_encode)) return 0;
  guatiao_value out;
  memset(&out, 0, sizeof out);
  guatiao_status st = gt_core.guatiao_wire_encode(&gt_binding_alloc, v, &out);
  if (st) {
    gt_free(&out);
    return gt_fail(c, st, "the value could not be encoded");
  }
  gt_answer(c, 0, bytes_mx(&out));
  gt_free(&out);
  return 1;
}

/* wire_decode(bytes, alloc) -> the id of a new value */
int cmd_wire_decode(gt_call *c) {
  guatiao_bytes b;
  const guatiao_alloc *a = NULL;
  if (!arg_bytes(c, 0, "the bytes", &b) || !gt_arg_alloc(c, 1, &a) || !GT_NEED_CORE(c, guatiao_wire_decode)) return 0;
  guatiao_value out;
  memset(&out, 0, sizeof out);
  guatiao_status st = gt_core.guatiao_wire_decode(a, b, &out);
  if (st == GUATIAO_ERR_BAD_VALUE) {
    gt_free(&out);
    return gt_fail_detail(c, st, "malformed", "bytes that are not a value in the wire encoding");
  }
  if (st) {
    gt_free(&out);
    return gt_fail(c, st, "the bytes could not be decoded");
  }
  uint64_t id = gt_owner_new(&out, a);
  if (!id) {
    gt_free(&out);
    return gt_fail(c, GUATIAO_ERR_ALLOC, "out of memory");
  }
  gt_answer(c, 0, mxCreateDoubleScalar((double)id));
  return 1;
}

/* wire_frame(kind, channel[, tree]) -> the frame, a uint8 row; kind 0
 * announces a schema, 1 carries a value, 2 ends the channel */
int cmd_wire_frame(gt_call *c) {
  double kind = 0;
  uint64_t channel = 0;
  if (!gt_arg_double(c, 0, "the frame kind", &kind) || !arg_channel(c, 1, &channel)) return 0;
  if (kind != 0 && kind != 1 && kind != 2) return gt_fail(c, GUATIAO_ERR_BAD_VALUE, "a frame kind is 0, 1 or 2");
  if (!GT_NEED_CORE(c, guatiao_wire_close)) return 0;
  guatiao_value out;
  memset(&out, 0, sizeof out);
  guatiao_status st;
  if (kind == 2) {
    st = gt_core.guatiao_wire_close(&gt_binding_alloc, channel, &out);
  } else {
    guatiao_value t[1];
    const guatiao_value *p[1];
    int at[1] = {2};
    if (!gt_tree_args(c, 1, at, t, p)) return 0;
    st = kind == 0 ? gt_core.guatiao_wire_announce(&gt_binding_alloc, channel, p[0], &out)
                   : gt_core.guatiao_wire_send(&gt_binding_alloc, channel, p[0], &out);
    gt_free(&t[0]);
  }
  if (st) {
    gt_free(&out);
    return gt_fail(c, st, "the frame could not be written");
  }
  gt_answer(c, 0, bytes_mx(&out));
  gt_free(&out);
  return 1;
}

/* receiver_new(alloc) -> id */
int cmd_receiver_new(gt_call *c) {
  const guatiao_alloc *a = NULL;
  if (!gt_arg_alloc(c, 0, &a) || !GT_NEED_CORE(c, guatiao_wire_receiver_new)) return 0;
  gt_receiver *r = (gt_receiver *)calloc(1, sizeof *r);
  if (!r) return gt_fail(c, GUATIAO_ERR_ALLOC, "out of memory");
  r->alloc = a;
  r->recv = gt_core.guatiao_wire_receiver_new(a);
  if (!r->recv) {
    free(r);
    return gt_fail(c, GUATIAO_ERR_ALLOC, "the allocator was refused");
  }
  uint64_t id = gt_handle_add(GT_H_RECEIVER, r);
  if (!id) {
    gt_core.guatiao_wire_receiver_free(r->recv);
    free(r);
    return gt_fail(c, GUATIAO_ERR_ALLOC, "out of memory");
  }
  gt_answer(c, 0, mxCreateDoubleScalar((double)id));
  return 1;
}

/* receiver_free(id): the schemas it kept are freed; a stale id is nothing */
int cmd_receiver_free(gt_call *c) {
  uint64_t id = 0;
  if (!gt_arg_id(c, 0, &id)) {
    c->status = GUATIAO_OK;
    return 1;
  }
  gt_receiver *r = (gt_receiver *)gt_handle_take(id, GT_H_RECEIVER);
  if (r) {
    gt_core.guatiao_wire_receiver_free(r->recv);
    free(r);
  }
  return 1;
}

int cmd_receiver_is_open(gt_call *c) {
  uint64_t id = 0;
  int open = gt_arg_id(c, 0, &id) && gt_handle_get(id, GT_H_RECEIVER) != NULL;
  c->status = GUATIAO_OK;
  gt_answer(c, 0, mxCreateLogicalScalar(open ? true : false));
  return 1;
}

/* The refused field, what it accepts and the fields there are, from the
 * detail map guatiao_schema_validate's shape gives. */
static void read_detail(const guatiao_value *d, char *key, size_t nkey, char *expected, size_t nexp, mxArray **known) {
  key[0] = expected[0] = '\0';
  *known = NULL;
  const guatiao_value *k = guatiao_map_find(d, guatiao_cstr("key"));
  if (k) text_into(k, key, nkey);
  const guatiao_value *e = guatiao_map_find(d, guatiao_cstr("expected"));
  if (e) text_into(e, expected, nexp);
  const guatiao_value *names = guatiao_map_find(d, guatiao_cstr("known"));
  if (names && guatiao_tag_of(names) == GUATIAO_LIST) {
    guatiao_values items = guatiao_list_items(names);
    mxArray *cell = mxCreateCellMatrix(1, (mwSize)items.len);
    for (size_t i = 0; i < items.len; i++) {
      mxArray *s = gt_utf8_mx(guatiao_string_text(&items.ptr[i]));
      mxSetCell(cell, (mwIndex)i, s ? s : gt_cstr_mx(""));
    }
    *known = cell;
  }
}

/* receiver_accept(id, frame, soft) -> [kind, channel, value id or [],
 * problem or []]. Without `soft` a refusal raises
 * guatiao:<Status>:<refusal>; with it the refusal is the fourth answer. */
int cmd_receiver_accept(gt_call *c) {
  gt_receiver *r = NULL;
  guatiao_bytes frame;
  int soft = 0;
  if (!gt_arg_handle(c, 0, GT_H_RECEIVER, "receiver", (void **)&r) || !arg_bytes(c, 1, "the frame", &frame)) return 0;
  if (gt_arg(c, 2) && !gt_arg_bool(c, 2, "soft", &soft)) return 0;
  guatiao_wire_event e;
  memset(&e, 0, sizeof e);
  guatiao_status st = gt_core.guatiao_wire_receiver_accept(r->recv, frame, &e);

  char words[1024], key[256], expected[512];
  mxArray *known = NULL;
  text_into(&e.message, words, sizeof words);
  read_detail(&e.detail, key, sizeof key, expected, sizeof expected, &known);
  gt_free(&e.message);
  gt_free(&e.detail);

  if (!st) {
    mxArray *value = NULL;
    if (e.value.tag != GUATIAO_ABSENT) {
      uint64_t id = gt_owner_new(&e.value, r->alloc);
      if (!id) {
        gt_free(&e.value);
        if (known) mxDestroyArray(known);
        return gt_fail(c, GUATIAO_ERR_ALLOC, "out of memory");
      }
      value = mxCreateDoubleScalar((double)id);
    }
    if (known) mxDestroyArray(known);
    gt_answer(c, 0, gt_cstr_mx(e.kind <= 2 ? kind_names[e.kind] : "unknown"));
    gt_answer(c, 1, uint64_mx(e.channel));
    gt_answer(c, 2, value ? value : mxCreateDoubleMatrix(0, 0, mxREAL));
    gt_answer(c, 3, mxCreateDoubleMatrix(0, 0, mxREAL));
    return 1;
  }
  gt_free(&e.value);

  const char *refusal = NULL;
  switch (e.refusal) {
  case GUATIAO_WIRE_MALFORMED: refusal = "malformed"; break;
  case GUATIAO_WIRE_NO_SCHEMA: refusal = "noSchema"; break;
  case GUATIAO_WIRE_NOT_A_SCHEMA: refusal = "notASchema"; break;
  case GUATIAO_WIRE_INVALID: refusal = "invalid"; break;
  default: break;
  }
  if (!refusal) {
    if (known) mxDestroyArray(known);
    return gt_fail(c, st, "%s", words[0] ? words : "the frame was refused");
  }
  if (!soft) {
    if (known) mxDestroyArray(known);
    return gt_fail_detail(c, st, refusal, "%s", words[0] ? words : refusal);
  }
  const char *names[9] = {"refusal", "status", "channel", "kind", "at", "key", "expected", "known", "message"};
  mxArray *p = mxCreateStructMatrix(1, 1, 9, names);
  mxSetField(p, 0, "refusal", gt_cstr_mx(refusal));
  mxSetField(p, 0, "status", gt_cstr_mx(gt_status_name(st)));
  mxSetField(p, 0, "channel", uint64_mx(e.channel));
  mxSetField(p, 0, "kind", mxCreateDoubleScalar((double)e.kind));
  mxSetField(p, 0, "at", mxCreateDoubleScalar((double)e.at));
  mxArray *k = gt_utf8_mx(guatiao_str_from(key, strlen(key)));
  mxSetField(p, 0, "key", k ? k : gt_cstr_mx(""));
  mxArray *x = gt_utf8_mx(guatiao_str_from(expected, strlen(expected)));
  mxSetField(p, 0, "expected", x ? x : gt_cstr_mx(""));
  mxSetField(p, 0, "known", known ? known : mxCreateCellMatrix(1, 0));
  mxArray *m = gt_utf8_mx(guatiao_str_from(words, strlen(words)));
  mxSetField(p, 0, "message", m ? m : gt_cstr_mx(""));
  gt_answer(c, 0, gt_cstr_mx(""));
  gt_answer(c, 1, uint64_mx(e.channel));
  gt_answer(c, 2, mxCreateDoubleMatrix(0, 0, mxREAL));
  gt_answer(c, 3, p);
  return 1;
}
