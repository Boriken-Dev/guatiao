// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// Command documents times a document to plain Go data and back, three
// ways: through a Value (serde.Parse then Decode, serde.Emit of
// guatiao.NewValue), through the wire bytes as a tape read and written in
// Go (serde.Unmarshal, serde.Emit of Go data), and through cgo callbacks
// (a visitor the library calls per event, a walker calling the library's
// visitor per event).
//
//	GUATIAO_LIBRARY=<target>/release go run . [-save NAME]
//
// in benchmarks/go writes benchmarks/results/NAME.json in the runner's
// shape.
package main

/*
#cgo CFLAGS: -I${SRCDIR}/../../crates/guatiao/include -I${SRCDIR}/../../crates/guatiao-serde/include
#include <stdlib.h>
#include "bench.h"
*/
import "C"

import (
	"encoding/json"
	"flag"
	"fmt"
	"os"
	"path/filepath"
	"reflect"
	"runtime"
	"slices"
	"strings"
	"time"
	"unsafe"

	guatiao "github.com/Boriken-Dev/guatiao/bindings/go/guatiao"
	serde "github.com/Boriken-Dev/guatiao/bindings/go/guatiao-serde"
)

const (
	slotNull    = C.SLOT_NULL
	slotListEnd = C.SLOT_LIST_END
	slotMapEnd  = C.SLOT_MAP_END
	slotNumber  = C.SLOT_NUMBER
	slotString  = C.SLOT_STRING
	slotKey     = C.SLOT_KEY
	slotList    = C.SLOT_LIST
	slotMap     = C.SLOT_MAP
)

func visitor(p unsafe.Pointer) *C.guatiao_visitor { return (*C.guatiao_visitor)(p) }

func event(into unsafe.Pointer, slot int) uint32 {
	return uint32(C.bench_event(visitor(into), C.int(slot)))
}

func boolean(into unsafe.Pointer, b bool) uint32 {
	return uint32(C.bench_boolean(visitor(into), C.bool(b)))
}

func text(into unsafe.Pointer, slot int, s string) uint32 {
	return uint32(C.bench_text(visitor(into), C.int(slot), (*C.char)(unsafe.Pointer(unsafe.StringData(s))), C.size_t(len(s))))
}

func count(into unsafe.Pointer, slot, n int) uint32 {
	return uint32(C.bench_count(visitor(into), C.int(slot), C.uint64_t(n)))
}

// library is guatiao_serde, opened again for the two entry points the
// package does not call.
var library = &guatiao.Surface{
	Basename: "guatiao_serde",
	Variable: "GUATIAO_SERDE_LIBRARY",
	Probe:    "guatiao_json_parse_ex",
	Exports:  map[string]string{"guatiao_json_visit": "", "guatiao_json_emit_walker": "", "guatiao_value_free": ""},
}

func must[T any](v T, err error) T {
	if err != nil {
		panic(err)
	}
	return v
}

func parseCallbacks(doc string) any {
	fn := must(library.Symbol("guatiao_json_visit"))
	building.stack, building.keys, building.key, building.root = building.stack[:0], building.keys[:0], "", nil
	st := C.bench_visit(fn, (*C.char)(unsafe.Pointer(unsafe.StringData(doc))), C.size_t(len(doc)), (*C.guatiao_alloc)(library.Alloc()))
	if st != 0 {
		panic(fmt.Sprintf("visit refused: %d", st))
	}
	return building.root
}

func renderCallbacks(x any) string {
	fn := must(library.Symbol("guatiao_json_emit_walker"))
	free := must(library.Symbol("guatiao_value_free"))
	walking.data = x
	out := (*C.guatiao_value)(C.calloc(1, C.sizeof_guatiao_value))
	defer C.bench_release(free, out)
	if st := C.bench_emit_walker(fn, (*C.guatiao_alloc)(library.Alloc()), out); st != 0 {
		panic(fmt.Sprintf("the walk refused: %d", st))
	}
	var n C.size_t
	p := C.bench_text_of(out, &n)
	return C.GoStringN(p, C.int(n))
}

func parseValue(doc string) any {
	v := must(serde.Parse(doc, serde.JSON))
	defer v.Close()
	var x any
	if err := v.Decode(&x); err != nil {
		panic(err)
	}
	return x
}

func parseTape(doc string) any {
	var x any
	if err := serde.Unmarshal(doc, serde.JSON, &x); err != nil {
		panic(err)
	}
	return x
}

func renderValue(x any) string {
	v := must(guatiao.NewValue(x))
	defer v.Close()
	return must(serde.Emit(v, serde.JSON))
}

func renderTape(x any) string { return must(serde.Emit(x, serde.JSON)) }

func config() any {
	servers := make([]any, 0, 50)
	for i := 0; i < 50; i++ {
		servers = append(servers, map[string]any{
			"host":    fmt.Sprintf("node-%d.example", i),
			"port":    8000 + i,
			"weight":  float64(i) / 7,
			"tags":    []any{"edge", "eu", fmt.Sprint(i)},
			"enabled": i%3 != 0,
			"limits":  map[string]any{"rps": 1000 + i, "burst": 2.5, "note": nil},
		})
	}
	return map[string]any{"name": "fleet", "servers": servers}
}

func cases() []struct {
	name string
	data any
} {
	floats := make([]any, 10_000)
	for i := range floats {
		floats[i] = float64(i)*1.25 + 0.5
	}
	return []struct {
		name string
		data any
	}{
		{"config", config()},
		{"floats_10k", floats},
		{"text_1mb", map[string]any{"blob": strings.Repeat("x", 1<<20)}},
	}
}

func timeIt(fn func(), target time.Duration, samples int) []float64 {
	fn()
	n := 1
	for {
		start := time.Now()
		for i := 0; i < n; i++ {
			fn()
		}
		if time.Since(start) >= target {
			break
		}
		n *= 2
	}
	out := make([]float64, samples)
	for s := range out {
		start := time.Now()
		for i := 0; i < n; i++ {
			fn()
		}
		out[s] = float64(time.Since(start).Nanoseconds()) / float64(n) / 1e6
	}
	return out
}

func median(xs []float64) float64 {
	s := slices.Clone(xs)
	slices.Sort(s)
	m := len(s) / 2
	if len(s)%2 == 1 {
		return s[m]
	}
	return (s[m-1] + s[m]) / 2
}

type metric struct {
	MinMs    float64 `json:"min_ms"`
	MedianMs float64 `json:"median_ms"`
	MaxMs    float64 `json:"max_ms"`
}

func main() {
	save := flag.String("save", "", "write benchmarks/results/NAME.json")
	samples := flag.Int("samples", 15, "samples per metric")
	targetMs := flag.Int("target-ms", 20, "milliseconds per sample, at least")
	only := flag.String("only", "", "time only the metrics whose name holds this")
	flag.Parse()

	type named struct {
		name string
		fn   func()
	}
	var metrics []named
	for _, c := range cases() {
		doc := renderTape(c.data)
		// Every path reads and writes the same thing.
		want := parseValue(doc)
		if got := parseTape(doc); !reflect.DeepEqual(got, want) {
			panic(c.name + ": the tape reads differently")
		}
		if got := parseCallbacks(doc); !reflect.DeepEqual(got, want) {
			panic(c.name + ": the callbacks read differently")
		}
		if renderValue(c.data) != doc || renderCallbacks(c.data) != doc {
			panic(c.name + ": a path writes differently")
		}
		data := c.data
		metrics = append(metrics,
			named{c.name + "_parse_value", func() { parseValue(doc) }},
			named{c.name + "_parse_tape", func() { parseTape(doc) }},
			named{c.name + "_parse_callbacks", func() { parseCallbacks(doc) }},
			named{c.name + "_render_value", func() { renderValue(data) }},
			named{c.name + "_render_tape", func() { renderTape(data) }},
			named{c.name + "_render_callbacks", func() { renderCallbacks(data) }},
		)
	}
	result := map[string]any{"name": *save, "metrics": map[string]metric{}, "raw": map[string]any{
		"processor": runtime.GOARCH, "os": runtime.GOOS, "go": runtime.Version(), "samples_ms": map[string][]float64{},
	}}
	if *save == "" {
		result["name"] = "documents"
	}
	for _, m := range metrics {
		if *only != "" && !strings.Contains(m.name, *only) {
			continue
		}
		xs := timeIt(m.fn, time.Duration(*targetMs)*time.Millisecond, *samples)
		result["metrics"].(map[string]metric)[m.name] = metric{slices.Min(xs), median(xs), slices.Max(xs)}
		result["raw"].(map[string]any)["samples_ms"].(map[string][]float64)[m.name] = xs
		fmt.Printf("%-32s %12.1f us\n", m.name, median(xs)*1000)
	}
	if *save != "" {
		text := must(json.MarshalIndent(result, "", "  "))
		path := filepath.Join("..", "results", *save+".json")
		if err := os.WriteFile(path, append(text, '\n'), 0o644); err != nil {
			panic(err)
		}
		fmt.Fprintln(os.Stderr, "saved", path)
	}
}
