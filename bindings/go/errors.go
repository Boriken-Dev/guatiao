// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package guatiao

import (
	"errors"
	"fmt"
	"strings"

	"github.com/Boriken-Dev/guatiao/bindings/go/internal/native"
)

// Status is a guatiao_status: 0 for success, a code for each failure.
// A code this package does not name is still a Status.
type Status uint32

// The codes the C surface defines.
const (
	OK           Status = Status(native.OK)
	BadValue     Status = Status(native.ErrBadValue)
	AllocFailed  Status = Status(native.ErrAlloc)
	WrongKind    Status = Status(native.ErrWrongKind)
	NotFound     Status = Status(native.ErrNotFound)
	NullArgument Status = Status(native.ErrNull)
	Gone         Status = Status(native.ErrGone)
	Unsupported  Status = Status(native.ErrUnsupport)
	Busy         Status = Status(native.ErrBusy)
	Internal     Status = Status(native.ErrInternal)
)

var statusNames = map[Status]string{
	OK:           "ok",
	BadValue:     "bad value",
	AllocFailed:  "allocation failed",
	WrongKind:    "wrong kind",
	NotFound:     "not found",
	NullArgument: "null argument",
	Gone:         "gone",
	Unsupported:  "unsupported",
	Busy:         "busy",
	Internal:     "internal error",
}

func (s Status) String() string {
	if name, ok := statusNames[s]; ok {
		return name
	}
	return fmt.Sprintf("status %d", uint32(s))
}

// Error is a failing Status and the other side's own words, empty when
// it gave none. errors.Is matches it against the Err sentinel of its
// status.
type Error struct {
	Status  Status
	Message string
}

func (e *Error) Error() string {
	if e.Message == "" {
		return "guatiao: " + e.Status.String()
	}
	return "guatiao: " + e.Status.String() + ": " + e.Message
}

// Is reports whether target is the sentinel of e's status.
func (e *Error) Is(target error) bool {
	t, ok := target.(*Error)
	return ok && t.Message == "" && t.Status == e.Status
}

// The sentinels, one per status, for errors.Is.
var (
	ErrBadValue     = &Error{Status: BadValue}
	ErrAllocFailed  = &Error{Status: AllocFailed}
	ErrWrongKind    = &Error{Status: WrongKind}
	ErrNotFound     = &Error{Status: NotFound}
	ErrNullArgument = &Error{Status: NullArgument}
	ErrGone         = &Error{Status: Gone}
	ErrUnsupported  = &Error{Status: Unsupported}
	ErrBusy         = &Error{Status: Busy}
	ErrInternal     = &Error{Status: Internal}
)

// Errorf is an *Error with status and a message formatted as fmt.Sprintf
// formats one: what a provider returns to refuse with its own words.
func Errorf(status Status, format string, args ...any) error {
	return &Error{Status: status, Message: fmt.Sprintf(format, args...)}
}

// check is nil for OK, else the *Error for status and message.
func check(status uint32, message string) error {
	if status == native.OK {
		return nil
	}
	return &Error{Status: Status(status), Message: message}
}

// ErrClosed is the error of any use of a closed Value, Registry,
// Instance or object, and of a view whose source is closed or whose call
// has returned.
var ErrClosed = errors.New("guatiao: closed")

// ErrReadOnly is the error of a write through a borrowed view.
var ErrReadOnly = errors.New("guatiao: a borrowed value is read-only")

// LibraryNotFound says no file for a library opened, and where each route
// looked.
type LibraryNotFound struct {
	Basename string
	Looked   []string
}

func (e *LibraryNotFound) Error() string {
	return fmt.Sprintf("guatiao: could not find the %s library: looked in %s", e.Basename, strings.Join(e.Looked, "; "))
}

// MissingSymbol says an export the loaded build lacks, and the feature
// that build would need.
type MissingSymbol struct {
	Path, Symbol, Feature string
}

func (e *MissingSymbol) Error() string {
	if e.Feature != "" {
		return fmt.Sprintf("guatiao: %s was not built with %s (feature %q)", e.Path, e.Symbol, e.Feature)
	}
	return fmt.Sprintf("guatiao: %s was not built with %s", e.Path, e.Symbol)
}

// FloorMismatch says a table is not the one a kind describes: too small
// for its required slots, or another floor hash.
type FloorMismatch struct {
	Message string
}

func (e *FloorMismatch) Error() string { return "guatiao: " + e.Message }

// lift turns the native layer's failures into this package's errors.
func lift(err error) error {
	var nf *native.NotFound
	if errors.As(err, &nf) {
		return &LibraryNotFound{Basename: nf.Basename, Looked: nf.Looked}
	}
	var ms *native.Missing
	if errors.As(err, &ms) {
		return &MissingSymbol{Path: ms.Path, Symbol: ms.Symbol, Feature: ms.Feature}
	}
	return err
}

// core opens the guatiao library, lifting a failure.
func core() (*native.Lib, error) {
	lib, err := native.Core()
	if err != nil {
		return nil, lift(err)
	}
	return lib, nil
}

// UseLibrary names the guatiao library to load instead of looking for
// one: a file exporting the ABI, whatever it is called, or a directory
// holding guatiao.dll, libguatiao.so or libguatiao.dylib. guatiao_serde
// and guatiao_intake are then looked for beside it. Call it before the
// first call that needs the library; naming another file once one is
// loaded is an error, naming the same one is not.
func UseLibrary(where string) error { return native.Use(where) }
