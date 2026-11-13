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
// Source is the error the server names as this one's cause, and Unwrap
// returns it, so errors.As and errors.Is walk the server's chain. The BAML
// runtime's own error, which carried the failure, is not in the chain.
//
// Three kinds of error are not *AlienError:
//   - context.Canceled and context.DeadlineExceeded when ctx ends; they pass
//     through unchanged, so errors.Is(err, context.DeadlineExceeded) works.
//   - every failure of a generated alienapi call that is not wrapped in Do or
//     Err: it is a *baml_go.BamlError (docs/gaps.md#b5).
//   - runtime failures: a runtime that cannot start, a BAML panic, a call
//     made after Shutdown.
type AlienError struct {
	Code      string `json:"code"`
	Message   string `json:"message"`
	Retryable bool   `json:"retryable"`
	// HTTPStatus is the response's status; nil when there was no response.
	// On a Source level: the status that level names, if any.
	HTTPStatus *int `json:"http_status"`
	// Context is JSON data as encoding/json decodes it into an any: a
	// map[string]any, a []any, a string, a float64, a bool, or nil. For a
	// server error it is the error body's context exactly as sent; for an
	// error the SDK raises itself, nil or an object with the SDK's own keys
	// (docs/errors.md). Every number is a float64, so an integer above 2^53
	// is rounded.
	Context any `json:"context"`
	// Hint is the server's remediation hint; "" when it sent none.
	Hint string `json:"hint"`
	// RequestID is the server's request ID: the error body's requestId, else
	// the response's x-request-id header. "" when no response arrived or it
	// named none.
	RequestID string `json:"request_id"`
	// Source is the error that caused this one, from the server's source
	// chain (at most 8 levels below the returned error); nil when there is
	// none.
	Source *AlienError `json:"source"`
	// Internal is the server's internal flag; nil when the error is not from
	// a server's error body.
	Internal *bool `json:"internal"`
}

func (e *AlienError) Error() string { return e.Code + ": " + e.Message }

// Unwrap returns Source, or nil (an untyped nil, so errors.Unwrap(err) == nil)
// when there is none.
func (e *AlienError) Unwrap() error {
	if e.Source == nil {
		return nil
	}
	return e.Source
}

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
	decoded := &AlienError{}
	if decodeErr := decodeThrown(bamlErr.Value, decoded); decodeErr != nil {
		return fmt.Errorf("alien: cannot read thrown AlienError: %w (original: %w)", decodeErr, err)
	}
	if decoded.Code == "" {
		// The Go bridge leaves Value nil when it cannot decode a thrown value.
		return fmt.Errorf("alien: thrown AlienError has no code: %w", err)
	}
	return decoded
}

// decodeThrown reads a thrown value, a dynamic map keyed by the BAML field
// names, into out through its JSON form; a nested source decodes the same
// way.
func decodeThrown(value any, out *AlienError) error {
	encoded, err := json.Marshal(value)
	if err != nil {
		return err
	}
	return json.Unmarshal(encoded, out)
}
