// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package guest

import (
	"testing"
)

// told is what the glue told a stand-in host.
type told struct {
	answers [][]byte
	written map[uint32][]byte
	calls   []int64
	dropped []int64
}

func standIn(t *testing.T) *told {
	t.Helper()
	got := &told{written: map[uint32][]byte{}}
	hostAnswer = func(b []byte) { got.answers = append(got.answers, append([]byte{}, b...)) }
	hostWriteBack = func(i uint32, b []byte) uint32 { got.written[i] = b; return 0 }
	hostObjectCall = func(h int64, request []byte) (uint32, []byte) {
		got.calls = append(got.calls, h)
		return 0, nil
	}
	hostDrop = func(h int64) { got.dropped = append(got.dropped, h) }
	objects = map[int64]Handler{}
	return got
}

func request(t *testing.T, kind, method string, args ...any) []byte {
	t.Helper()
	b, err := Encode(append([]any{kind, method}, args...))
	if err != nil {
		t.Fatal(err)
	}
	return b
}

type counter struct{ n int64 }

func (c *counter) Call(call *Call) (any, error) {
	switch call.Method() {
	case "count":
		c.n++
		return c.n, nil
	case "fill":
		m, err := call.LentMap(0)
		if err != nil {
			return nil, err
		}
		m.Set("filled", true)
		b, err := call.Buffer(1)
		if err != nil {
			return nil, err
		}
		copy(b, "ok")
		return nil, nil
	case "start":
		listener, err := call.Object(0)
		if err != nil {
			return nil, err
		}
		return &Object{Handler: &talk{listener}}, nil
	case "panic":
		var m map[string]int
		m["x"] = 1
	}
	return nil, Refuse(StatusUnsupported, "no `%s`", call.Method())
}

type talk struct{ listener *HostObject }

func (t *talk) Call(c *Call) (any, error) {
	_, err := t.listener.Call("heard", "hi")
	return nil, err
}

func (t *talk) Release() { t.listener.Drop() }

func TestTheGlueDeclaresDispatchesAndRefuses(t *testing.T) {
	got := standIn(t)
	Serve(&Library{Providers: []Provider{{
		ID: "counter", Handler: &counter{},
		Serves: []Serves{{Kind: "counter", Slots: []string{"count"}}},
	}}})
	if declare() != 0 {
		t.Fatal("declared")
	}
	d, err := Decode(got.answers[0])
	if err != nil {
		t.Fatal(err)
	}
	if got := tree(d); len(got.([]any)) != 2 {
		t.Fatalf("%v", got)
	}
	for want := int64(1); want <= 2; want++ {
		if s := call(false, 0, request(t, "counter", "count")); s != 0 {
			t.Fatalf("status %d", s)
		}
		v, _ := Decode(got.answers[len(got.answers)-1])
		if n, _ := v.(Number).Int(); n != want {
			t.Errorf("counted %v", v)
		}
	}
	if s := call(false, 0, request(t, "counter", "nope")); s != uint32(StatusUnsupported) {
		t.Errorf("status %d", s)
	}
	if s := call(false, 0, request(t, "counter", "panic")); s != uint32(StatusInternal) {
		t.Errorf("a panic is %d", s)
	}
	if s := call(false, 9, request(t, "counter", "count")); s != uint32(StatusNotFound) {
		t.Errorf("no provider 9: %d", s)
	}
	if s := call(false, 0, []byte{0xff}); s != uint32(StatusBadValue) {
		t.Errorf("garbage: %d", s)
	}
}

func TestLentArgumentsAreWrittenBackAndObjectsKeptUntilReleased(t *testing.T) {
	got := standIn(t)
	Serve(&Library{Providers: []Provider{{ID: "c", Handler: &counter{}}}})
	if s := call(false, 0, request(t, "counter", "fill", NewMap(), make([]byte, 4))); s != 0 {
		t.Fatalf("status %d", s)
	}
	buffer, _ := Decode(got.written[1])
	if string(buffer.([]byte)) != "ok\x00\x00" {
		t.Errorf("%q", buffer)
	}
	if s := call(false, 0, request(t, "counter", "start", Number("42"))); s != 0 {
		t.Fatalf("status %d", s)
	}
	id, _ := Decode(got.answers[len(got.answers)-1])
	n, _ := id.(Number).Int()
	if s := call(true, n, request(t, "conversation", "say", "hi")); s != 0 || len(got.calls) != 1 || got.calls[0] != 42 {
		t.Errorf("status %d, calls %v", s, got.calls)
	}
	release(n)
	if len(got.dropped) != 1 || got.dropped[0] != 42 {
		t.Errorf("dropped %v", got.dropped)
	}
	if s := call(true, n, request(t, "conversation", "say", "hi")); s != uint32(StatusGone) {
		t.Errorf("released, then %d", s)
	}
}
