// Generated operations (`call.op` in a scenario, conformance/README.md) run
// through their methods (c.<Group>.<Method>), from the table internal/genops
// writes into operations_table_test.go (gitignored; scripts/generate.sh runs
// it). The helpers below decode a scenario's arguments with the SDK itself.
package alien

import (
	"context"
	"encoding/json"
	"fmt"
	"reflect"
	"strings"

	"example.com/alien-baml-sdk/go/baml_sdk"
)

type operation func(ctx context.Context, args map[string]json.RawMessage) (any, error)

// harnessError is a broken scenario or binding, never an SDK outcome.
type harnessError struct{ error }

func runOperation(ctx context.Context, name string, args map[string]json.RawMessage) (value any, err error) {
	op, ok := operations[name]
	if !ok {
		return nil, harnessError{fmt.Errorf("no Go binding for operation %s", name)}
	}
	defer func() {
		if r := recover(); r != nil {
			failure, ok := r.(harnessError)
			if !ok {
				panic(r)
			}
			value, err = nil, failure
		}
	}()
	return op(ctx, args)
}

// arg decodes a scenario argument with the SDK's own JSON decoding (BAML's,
// through a generated helper), so every generated type works, unions included.
func arg[T any](ctx context.Context, args map[string]json.RawMessage, name string) T {
	raw, ok := args[name]
	if !ok {
		panic(harnessError{fmt.Errorf("scenario args have no %q", name)})
	}
	value, err := baml_sdk.InternalDecodeBody[T](ctx, string(raw), 200, "scenario argument "+name)
	if err != nil {
		panic(harnessError{fmt.Errorf("decode scenario argument %q: %w", name, err)})
	}
	return value
}

// optArg is an optional argument: nil when the scenario leaves it out.
func optArg[T any](ctx context.Context, args map[string]json.RawMessage, name string) *T {
	if _, ok := args[name]; !ok {
		return nil
	}
	value := arg[T](ctx, args, name)
	return &value
}

// optList is an optional list argument: nil when the scenario leaves it out.
func optList[T any](ctx context.Context, args map[string]json.RawMessage, name string) []T {
	if _, ok := args[name]; !ok {
		return nil
	}
	return arg[[]T](ctx, args, name)
}

func result[R any](value R, err error) (any, error) {
	if err != nil {
		return nil, err
	}
	return hostJSON(reflect.ValueOf(value)), nil
}

// hostJSON is a generated value as the other harnesses see theirs: struct
// fields under their json tags (the BAML field names), a union as its selected
// arm. encoding/json cannot see the arm, an unexported field.
func hostJSON(v reflect.Value) any {
	switch v.Kind() {
	case reflect.Invalid:
		return nil
	case reflect.Pointer, reflect.Interface:
		if v.IsNil() {
			return nil
		}
		return hostJSON(v.Elem())
	case reflect.Struct:
		if variant := v.MethodByName("Variant"); variant.IsValid() && v.NumField() == 1 && !v.Type().Field(0).IsExported() {
			return hostJSON(variant.Call(nil)[0].Elem().FieldByName("Value"))
		}
		out := map[string]any{}
		for i := 0; i < v.NumField(); i++ {
			field := v.Type().Field(i)
			if field.IsExported() {
				out[strings.Split(field.Tag.Get("json"), ",")[0]] = hostJSON(v.Field(i))
			}
		}
		return out
	case reflect.Slice, reflect.Array:
		out := make([]any, v.Len())
		for i := range out {
			out[i] = hostJSON(v.Index(i))
		}
		return out
	case reflect.Map:
		out := map[string]any{}
		for _, key := range v.MapKeys() {
			out[fmt.Sprint(key.Interface())] = hostJSON(v.MapIndex(key))
		}
		return out
	default:
		return v.Interface()
	}
}
