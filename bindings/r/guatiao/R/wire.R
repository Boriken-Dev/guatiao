# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

# Values as bytes (the wire encoding), and channels: a schema announced
# before its values, so a receiver knows what each value means and refuses
# what does not fit it. A frame is `kind:u8 channel:LEB128 payload`; the
# transport says where one ends.

from_wire <- function(bytes, alloc = NULL) .gt_root(.Call(C_wire_decode, bytes, alloc))

to_wire <- function(x) .Call(C_wire_encode, x)

wire_announce <- function(channel, schema) .Call(C_wire_frame, 0L, channel, schema)

wire_send <- function(channel, value) .Call(C_wire_frame, 1L, channel, value)

wire_close <- function(channel) .Call(C_wire_frame, 2L, channel, NULL)

.gt_frames <- c("schema", "value", "close")
.gt_refusals <- c("malformed", "no_schema", "not_a_schema", "invalid")

# The condition one refused frame becomes: guatiao_wire_<refusal>, a
# guatiao_wire_refused and the status's guatiao_error.
.gt_wire_refused <- function(status, refusal, kind, channel, at, message, detail) {
  name <- .gt_refusals[refusal]
  fields <- list(message = message, call = NULL, status = .gt_status_name(status), code = as.integer(status),
    refusal = name, channel = channel, kind = kind)
  if (name == "malformed") fields$at <- at
  if (name == "invalid") {
    fields$key <- if (is.character(detail$key)) detail$key else ""
    fields$expected <- detail$expected
    fields$known <- if (is.null(detail$known)) NULL else unlist(detail$known)
  }
  structure(class = c(paste0("guatiao_wire_", name), "guatiao_wire_refused", "guatiao_error", "error", "condition"),
    fields)
}

.gt_wire_handle <- function(self) {
  if (!.Call(C_value_is_open, self$handle)) .gt_closed()
  self$handle
}

.gt_wire_receiver_class <- methods::setRefClass("guatiao_wire_receiver",
  fields = list(handle = "externalptr"),
  methods = list(
    accept = function(frame) {
      "Reads one frame: an event, or the refusal signalled."
      r <- .Call(C_wire_accept, .gt_wire_handle(.self), frame)
      if (r[[1L]] == 0L) {
        owner <- r[[7L]]
        return(structure(class = "guatiao_wire_event", list(kind = .gt_frames[r[[3L]] + 1L], channel = r[[4L]],
          value = if (is.null(owner)) NULL else .gt_root(owner))))
      }
      if (r[[2L]] == 0L || r[[2L]] > length(.gt_refusals)) .gt_raise(r[[1L]], r[[6L]])
      stop(.gt_wire_refused(r[[1L]], r[[2L]], r[[3L]], r[[4L]], r[[5L]], r[[6L]], r[[8L]]))
    },
    close = function() {
      "Frees the receiver and the schemas it keeps; a second call does nothing."
      .Call(C_wire_receiver_free, handle)
      invisible(NULL)
    },
    is_open = function() {
      "Whether the receiver is open."
      .Call(C_value_is_open, handle)
    }
  )
)

wire_receiver <- function() .gt_wire_receiver_class$new(handle = .Call(C_wire_receiver_new))

print.guatiao_wire_event <- function(x, ...) {
  cat(sprintf("<wire event: %s on channel %s>\n", x$kind, format(x$channel)))
  invisible(x)
}
