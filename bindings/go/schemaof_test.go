// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package guatiao_test

import (
	"errors"
	"reflect"
	"testing"
	"time"

	guatiao "github.com/Boriken-Dev/guatiao/bindings/go"
)

func TestRedactHidesEverySecret(t *testing.T) {
	str := map[string]any{"type": "string"}
	secret := map[string]any{"type": "string", "x-sensitive": true}
	schema, err := guatiao.NewValue(map[string]any{
		"type": "object",
		"properties": map[string]any{
			"user":     str,
			"password": map[string]any{"type": "string", "writeOnly": true, "x-sensitive": true},
			"tls": map[string]any{"type": "object", "properties": map[string]any{
				"ca": str, "key": map[string]any{"$ref": "#/$defs/Key"}}},
			"recovery": map[string]any{"type": "array", "items": secret},
		},
		"$defs": map[string]any{"Key": secret},
	})
	if err != nil {
		t.Fatal(err)
	}
	defer schema.Close()
	login := map[string]any{"user": "ana", "password": "hunter2", "extra": "?",
		"tls": map[string]any{"ca": "/etc/ca.pem", "key": "-----BEGIN KEY-----"}, "recovery": []string{"r1", "r2"}}
	shown, err := guatiao.Redact(schema, login, "*")
	if err != nil {
		t.Fatal(err)
	}
	defer shown.Close()
	got, _ := shown.Any()
	want := map[string]any{"user": "ana", "password": "*", "extra": "*",
		"tls": map[string]any{"ca": "/etc/ca.pem", "key": "*"}, "recovery": []any{"*", "*"}}
	if !reflect.DeepEqual(got, want) {
		t.Fatalf("%#v", got)
	}
	if _, err := guatiao.Redact(5, login, "*"); !errors.Is(err, guatiao.ErrWrongKind) {
		t.Fatalf("%v", err)
	}
}

type Listen struct {
	Host    string        `guatiao:"host" schema:"minLength=1,title=Host"`
	Port    uint16        `guatiao:"port" schema:"default=5900"`
	Prefix  *string       `guatiao:"prefix"`
	Tags    []string      `guatiao:"tags,omitempty" schema:"uniqueItems=true"`
	Timeout time.Duration `guatiao:"timeout,omitempty"`
	Mode    string        `guatiao:"mode" schema:"enum=[\"fast\",\"safe\"]"`
}

func TestSchemaOfAConfigurationStruct(t *testing.T) {
	s, err := guatiao.SchemaOf[Listen]()
	if err != nil {
		t.Fatal(err)
	}
	defer s.Close()
	props, _ := s.Key("properties").Map()
	if got := props.Keys(); !reflect.DeepEqual(got, []string{"host", "port", "prefix", "tags", "timeout", "mode"}) {
		t.Fatalf("properties keep the fields' order: %v", got)
	}
	var required []string
	s.Key("required").Decode(&required)
	if !reflect.DeepEqual(required, []string{"host", "mode"}) {
		t.Fatalf("required %v", required)
	}
	if s.Key("additionalProperties").BoolOr(true) {
		t.Fatal("a struct refuses keys it does not declare")
	}
	if got, _ := s.Key("properties").Key("port").Key("maximum").NumberText(); got != "65535" {
		t.Fatalf("a uint16 states its bound: %q", got)
	}
	if got, _ := s.Key("properties").Key("port").Key("default").NumberText(); got != "5900" {
		t.Fatalf("default %q", got)
	}
	if s.Key("properties").Key("host").Key("title").StringOr("") != "Host" ||
		s.Key("properties").Key("host").Key("minLength").IntOr(0) != 1 {
		t.Fatal("the schema tag's keywords")
	}
	enum, _ := s.Key("properties").Key("mode").Key("enum").Any()
	if !reflect.DeepEqual(enum, []any{"fast", "safe"}) {
		t.Fatalf("a JSON value in the tag: %#v", enum)
	}
}

func TestSchemaOfRefusesWhatItCannotDescribe(t *testing.T) {
	type loop struct{ Next *loop }
	if _, err := guatiao.SchemaOf[loop](); err == nil {
		t.Fatal("a recursive type")
	}
	if _, err := guatiao.SchemaOf[chan int](); err == nil {
		t.Fatal("a channel")
	}
}
