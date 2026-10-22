/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

/*
 * The gateway: guatiao_mex('command', args...). The one place a refusal
 * becomes a MATLAB error (identifier guatiao:<Status>[:<detail>]), at this
 * frame, so it never unwinds through the library's frames, a provider's
 * or libffi's.
 */

#include <stdio.h>
#include <string.h>

#include "commands.h"
#include "gt.h"

#define GT_DECLARE(name) int cmd_##name(gt_call *c);
GT_COMMANDS(GT_DECLARE)
#undef GT_DECLARE

typedef struct gt_command {
  const char *name;
  int (*run)(gt_call *c);
} gt_command;

#define GT_ENTRY(name) {#name, cmd_##name},
static const gt_command commands[] = {GT_COMMANDS(GT_ENTRY){NULL, NULL}};
#undef GT_ENTRY

static int started;
static int depth;

/* How many calls into the gateway are in progress on the interpreter's
 * thread: a served slot runs M code only while one is. */
int gt_gateway_depth(void) { return depth; }

static void at_exit(void) {
  /* Octave calls this when it lets go of the function, which happens on a
   * `clear` before the gateway locked itself and when it finds the
   * function again in another file, not only at exit: nothing is freed
   * here, and the next call registers again. At exit the process takes
   * what is left; the library stays mapped until then. */
  started = 0;
}

void mexFunction(int nlhs, mxArray *plhs[], int nrhs, const mxArray *prhs[]) {
  if (!started) {
    started = 1;
    gt_note_main_thread();
    mexAtExit(at_exit);
  }
  char name[48];
  if (nrhs < 1 || !mxIsChar(prhs[0]) || mxGetString(prhs[0], name, sizeof name))
    mexErrMsgIdAndTxt("guatiao:BadValue", "guatiao_mex is called by the guatiao package: guatiao_mex(command, ...)");
  const gt_command *cmd = NULL;
  for (const gt_command *k = commands; k->name; k++)
    if (!strcmp(k->name, name)) {
      cmd = k;
      break;
    }
  if (!cmd) mexErrMsgIdAndTxt("guatiao:BadValue", "guatiao_mex has no command '%s'", name);

  gt_call call;
  memset(&call, 0, sizeof call);
  call.nlhs = nlhs;
  call.plhs = plhs;
  call.nrhs = nrhs - 1;
  call.prhs = prhs + 1;
  for (int i = 0; i < (nlhs > 0 ? nlhs : 1); i++) plhs[i] = NULL;
  depth++;
  int ok = cmd->run(&call);
  depth--;
  gt_scratch_release(&call);
  if (ok && !call.status) {
    /* An answer a command had nothing for is [], so every output asked for
     * is assigned. */
    for (int i = 0; i < nlhs; i++)
      if (!plhs[i]) plhs[i] = mxCreateDoubleMatrix(0, 0, mxREAL);
    return;
  }

  char id[96];
  if (call.detail[0])
    snprintf(id, sizeof id, "guatiao:%s:%s", gt_status_name(call.status), call.detail);
  else
    snprintf(id, sizeof id, "guatiao:%s", gt_status_name(call.status));
  /* Answers already written are freed by the interpreter with the error. */
  mexErrMsgIdAndTxt(id, "%s", call.msg[0] ? call.msg : gt_status_name(call.status));
}
