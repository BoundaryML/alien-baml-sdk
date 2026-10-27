package alien

import (
	"encoding/json"
	"errors"
	"fmt"

	baml_go "github.com/boundaryml/baml-go"
)

// AlienError is the error an Alien SDK call returns when the SDK, the Platform
// API or the Manager API reports a failure (docs/errors.md). Use errors.As to
// read it.
//
// Three kinds of error are not *AlienError:
//   - context.Canceled and context.DeadlineExceeded when ctx ends; they pass
//     through unchanged, so errors.Is(err, context.DeadlineExceeded) works.
//   - every failure of a generated baml_sdk call that is not wrapped in Do or
//     Err: it is a *baml_go.BamlError (docs/gaps.md#b5).
//   - runtime failures: a runtime that cannot start, a BAML panic, a call
//     made after Shutdown.
type AlienError struct {
	Code       string            `json:"code"`
	Message    string            `json:"message"`
	Retryable  bool              `json:"retryable"`
	HTTPStatus *int              `json:"http_status"`
	Context    map[string]string `json:"context"`
	// Cause is the BAML error this was decoded from.
	Cause error `json:"-"`
}

func (e *AlienError) Error() string { return e.Code + ": " + e.Message }

func (e *AlienError) Unwrap() error { return e.Cause }

const alienErrorClass = "user.errors.AlienError"

// convert turns a BAML-thrown AlienError into *AlienError. The generated Go
// SDK exposes thrown values only as a dynamic map (docs/gaps.md#b5). Any other
// error is returned unchanged.
func convert(err error) error {
	if err == nil {
		return nil
	}
	var bamlErr *baml_go.BamlError
	if !errors.As(err, &bamlErr) || bamlErr.Kind != "error" || bamlErr.ClassName != alienErrorClass {
		return err
	}
	decoded := &AlienError{Cause: err}
	if decodeErr := decodeThrown(bamlErr.Value, decoded); decodeErr != nil {
		return fmt.Errorf("alien: cannot read thrown AlienError: %w (original: %w)", decodeErr, err)
	}
	if decoded.Code == "" {
		// The Go bridge leaves Value nil when it cannot decode a thrown value.
		return fmt.Errorf("alien: thrown AlienError has no code: %w", err)
	}
	return decoded
}

// decodeThrown reads a thrown value, a dynamic map, into out through its JSON
// form.
func decodeThrown(value any, out *AlienError) error {
	encoded, err := json.Marshal(value)
	if err != nil {
		return err
	}
	return json.Unmarshal(encoded, out)
}
