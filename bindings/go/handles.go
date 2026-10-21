// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package guatiao

import (
	"errors"
	"fmt"
	"io"
	"log"
	"runtime/cgo"
	"sync/atomic"

	"github.com/Boriken-Dev/guatiao/bindings/go/internal/native"
)

// What C holds of a Go value is a cgo.Handle, shaped like an address: a
// served object's or an instance's ctx is LowUnmapped + 16 * handle. A
// handle number is never reused, so a ctx naming one that was released
// finds nothing and its call answers GUATIAO_ERR_GONE.

type heldValue struct {
	x      any
	object bool
}

var liveObjects atomic.Int64

func hold(x any, object bool) uintptr {
	h := cgo.NewHandle(heldValue{x: x, object: object})
	if object {
		liveObjects.Add(1)
	}
	return native.LowUnmapped + native.ContextStride*uintptr(h)
}

func handleOf(ctx uintptr) (cgo.Handle, error) {
	if ctx == 0 {
		return 0, &Error{Status: NullArgument, Message: "the call names no object"}
	}
	if ctx <= native.LowUnmapped || (ctx-native.LowUnmapped)%native.ContextStride != 0 {
		return 0, &Error{Status: Gone, Message: "the call names no object this binding serves"}
	}
	return cgo.Handle((ctx - native.LowUnmapped) / native.ContextStride), nil
}

// held is what ctx names.
func held(ctx uintptr) (x any, err error) {
	h, err := handleOf(ctx)
	if err != nil {
		return nil, err
	}
	defer func() {
		if recover() != nil {
			x, err = nil, &Error{Status: Gone, Message: "the object this call names is gone"}
		}
	}()
	return h.Value().(heldValue).x, nil
}

// release lets go of what ctx names and answers it; false for a ctx
// naming nothing.
func release(ctx uintptr) (x any, ok bool) {
	h, err := handleOf(ctx)
	if err != nil {
		return nil, false
	}
	defer func() {
		if recover() != nil {
			x, ok = nil, false
		}
	}()
	v := h.Value().(heldValue)
	h.Delete()
	if v.object {
		liveObjects.Add(-1)
	}
	return v.x, true
}

// LiveObjects is how many Go values a C caller can still reach as objects:
// served and not yet destroyed, or lent to a call still running.
func LiveObjects() int { return int(liveObjects.Load()) }

// closeHeld closes what a holder destroyed, when it is an io.Closer.
func closeHeld(x any, what string) {
	c, ok := x.(io.Closer)
	if !ok {
		return
	}
	defer func() {
		if p := recover(); p != nil {
			warn(fmt.Errorf("%s: Close panicked: %v", what, p))
		}
	}()
	if err := c.Close(); err != nil {
		warn(fmt.Errorf("%s: Close: %w", what, err))
	}
}

func init() {
	native.OnDestroyObject = func(ctx uintptr) {
		if x, ok := release(ctx); ok {
			closeHeld(x, "an object its holder destroyed")
		}
	}
}

// Warn receives what this binding cannot report to anyone else: a slot
// that cannot fail and failed, an object or instance whose Close failed
// after its holder destroyed it, a library that declined its host. It
// logs through the log package; set it to route them elsewhere.
var Warn = func(err error) { log.Printf("guatiao: %v", err) }

func warn(err error) {
	if Warn != nil && err != nil {
		Warn(err)
	}
}

// statusOf is the status and words an error crosses with: an *Error's
// own (its status and message, or the whole text when it wraps a
// sentinel), Internal and the text for anything else.
func statusOf(err error) (Status, string) {
	var e *Error
	if errors.As(err, &e) {
		status := e.Status
		if status == OK {
			status = Internal
		}
		if e.Message == "" && err != error(e) {
			return status, err.Error()
		}
		return status, e.Message
	}
	return Internal, err.Error()
}
