package alien

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"log/slog"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	baml_go "github.com/boundaryml/baml-go"

	"example.com/alien-baml-sdk/go/baml_sdk"
)

func TestConvert(t *testing.T) {
	thrown := &baml_go.BamlError{Kind: "error", ClassName: alienErrorClass, Value: map[string]any{
		"code": "NOT_FOUND", "message": "no such manager", "retryable": true, "http_status": int64(404),
		"context": map[string]any{"requestId": "req_1"},
	}}
	got := Err(thrown)
	var alienErr *AlienError
	if !errors.As(got, &alienErr) {
		t.Fatalf("thrown AlienError: got %T %v", got, got)
	}
	if alienErr.Code != "NOT_FOUND" || alienErr.Message != "no such manager" || !alienErr.Retryable ||
		alienErr.HTTPStatus == nil || *alienErr.HTTPStatus != 404 || alienErr.Context["requestId"] != "req_1" {
		t.Errorf("fields: %+v", alienErr)
	}
	if errors.Unwrap(got) != thrown {
		t.Errorf("Unwrap: got %v", errors.Unwrap(got))
	}
	if got.Error() != "NOT_FOUND: no such manager" {
		t.Errorf("Error(): %q", got.Error())
	}

	plain := errors.New("plain")
	for name, err := range map[string]error{
		"nil":            nil,
		"plain":          plain,
		"context":        context.Canceled,
		"panic":          &baml_go.BamlError{Kind: "panic", ClassName: "baml.panic"},
		"other class":    &baml_go.BamlError{Kind: "error", ClassName: "user.other.Error", Value: map[string]any{"code": "X"}},
		"wrapped plain":  fmt.Errorf("call: %w", plain),
		"compile failed": &baml_go.CompilationError{BamlError: baml_go.BamlError{Kind: "error", ClassName: "reflect.errors.CompilationError"}},
	} {
		if got := Err(err); got != err {
			t.Errorf("%s: want the error unchanged, got %v", name, got)
		}
	}

	// The Go bridge leaves Value nil when it cannot decode a thrown value.
	undecoded := &baml_go.BamlError{Kind: "error", ClassName: alienErrorClass}
	got = Err(undecoded)
	if errors.As(got, &alienErr) || !errors.Is(got, undecoded) || !strings.Contains(got.Error(), "has no code") {
		t.Errorf("undecoded AlienError: got %T %v", got, got)
	}

	if v, err := Do("value", nil); v != "value" || err != nil {
		t.Errorf("Do on success: %q %v", v, err)
	}
}

// newClient is New for options that have an API key.
func newClient(t *testing.T, options Options) *Client {
	t.Helper()
	c, err := New(options)
	if err != nil {
		t.Fatal(err)
	}
	return c
}

func TestNew(t *testing.T) {
	c, err := New(Options{Workspace: "ws"})
	var alienErr *AlienError
	if c != nil || !errors.As(err, &alienErr) || alienErr.Code != "INVALID_REQUEST" || !strings.Contains(alienErr.Message, "ALIEN_API_KEY") {
		t.Errorf("no API key: got %v, %v", c, err)
	}
	t.Setenv("ALIEN_API_KEY", "key_from_env")
	c = newClient(t, Options{})
	if c.Platform.ApiKey != "key_from_env" || c.Platform.BaseUrl != DefaultServerURL || c.Platform.Workspace != nil {
		t.Errorf("defaults: %#v", c.Platform)
	}
	t.Setenv("ALIEN_WORKSPACE", "ws_from_env")
	if c = newClient(t, Options{}); c.Platform.Workspace == nil || *c.Platform.Workspace != "ws_from_env" {
		t.Errorf("workspace from env: %#v", c.Platform)
	}
	c = newClient(t, Options{APIKey: "key", ServerURL: "http://127.0.0.1:9", Workspace: "ws"})
	if c.Platform.ApiKey != "key" || c.Platform.BaseUrl != "http://127.0.0.1:9" || c.Platform.Workspace == nil || *c.Platform.Workspace != "ws" {
		t.Errorf("explicit options: %#v", c.Platform)
	}
}

func TestNewManager(t *testing.T) {
	if m := NewManager(ManagerOptions{ServerURL: "http://127.0.0.1:9"}).Generated; m.BaseUrl != "http://127.0.0.1:9" || m.Token != nil {
		t.Errorf("no token: %#v", m)
	}
	if m := NewManager(ManagerOptions{ServerURL: "u", Token: "tok"}).Generated; m.Token == nil || *m.Token != "tok" {
		t.Errorf("token: %#v", m)
	}
}

func TestTimeout(t *testing.T) {
	ms := func(v *int64) any {
		if v == nil {
			return nil
		}
		return *v
	}
	for _, c := range []struct {
		timeout time.Duration
		want    any
	}{
		{0, nil},
		{300 * time.Millisecond, int64(300)},
		{1500 * time.Microsecond, int64(2)},
		{time.Nanosecond, int64(1)},
		{2 * time.Minute, int64(120_000)},
		{-time.Second, int64(-1)},
	} {
		c1 := newClient(t, Options{APIKey: "key", Timeout: c.timeout})
		m := NewManager(ManagerOptions{ServerURL: "u", Timeout: c.timeout})
		if got := ms(c1.Platform.TimeoutMs); got != c.want {
			t.Errorf("Options.Timeout %v: TimeoutMs %v, want %v", c.timeout, got, c.want)
		}
		if got := ms(m.Generated.TimeoutMs); got != c.want {
			t.Errorf("ManagerOptions.Timeout %v: TimeoutMs %v, want %v", c.timeout, got, c.want)
		}
	}
}

func TestPrintingHidesSecrets(t *testing.T) {
	const secret = "sk_secret_for_print_test"
	c := newClient(t, Options{APIKey: secret, Workspace: "ws"})
	values := map[string]any{
		"Options":        Options{APIKey: secret, Workspace: "ws"},
		"ManagerOptions": ManagerOptions{ServerURL: "http://127.0.0.1:9", Token: secret},
		"*Client":        c,
		"Client":         *c,
		"nested":         struct{ C *Client }{c},
	}
	for name, v := range values {
		for _, verb := range []string{"%v", "%+v", "%#v", "%s", "%q", "%x", "%d"} {
			if out := fmt.Sprintf(verb, v); strings.Contains(out, secret) || !strings.Contains(out, "<redacted>") {
				t.Errorf("%s with %s: %s", name, verb, out)
			}
		}
		if name == "nested" {
			continue // slog resolves LogValue only for the value it is given
		}
		var buf bytes.Buffer
		slog.New(slog.NewJSONHandler(&buf, nil)).Info("x", "v", v)
		slog.New(slog.NewTextHandler(&buf, nil)).Info("x", "v", v)
		if out := buf.String(); strings.Contains(out, secret) {
			t.Errorf("%s with slog: %s", name, out)
		}
	}
	if out := fmt.Sprint(Options{}); out != `alien.Options{APIKey:"" ServerURL:"" Workspace:"" Timeout:0s}` {
		t.Errorf("empty options: %s", out)
	}
}

// server answers one request, after checking it, with a JSON body.
func server(t *testing.T, want string, header string, status int, body string) *httptest.Server {
	t.Helper()
	s := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if got := r.Method + " " + r.URL.RequestURI(); got != want {
			t.Errorf("request: got %q, want %q", got, want)
		}
		if got := r.Header.Get("Authorization"); got != header {
			t.Errorf("Authorization: got %q, want %q", got, header)
		}
		w.Header().Set("Content-Type", "application/json")
		w.WriteHeader(status)
		_, _ = w.Write([]byte(body))
	}))
	t.Cleanup(s.Close)
	return s
}

func TestCalls(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), callTimeout)
	defer cancel()

	t.Run("Err with the API key from the environment", func(t *testing.T) {
		s := server(t, "DELETE /v1/api-keys/key_1", "Bearer key_from_env", 204, "")
		t.Setenv("ALIEN_API_KEY", "key_from_env")
		if err := Err(baml_sdk.PlatformApiKeysRevoke(ctx, newClient(t, Options{ServerURL: s.URL}).Platform, "key_1")); err != nil {
			t.Fatal(err)
		}
	})

	t.Run("a method without a response body", func(t *testing.T) {
		s := server(t, "DELETE /v1/api-keys/key_1", "Bearer key", 204, "")
		if err := newClient(t, Options{APIKey: "key", ServerURL: s.URL}).APIKeys.Revoke(ctx, "key_1"); err != nil {
			t.Fatal(err)
		}
	})

	t.Run("params: set fields only, nil params, zero values", func(t *testing.T) {
		const page = `{"items":[]}`
		s := server(t, "GET /v1/deployments?workspace=ws&project=web&status=running&status=failed&limit=10", "Bearer key", 200, page)
		c := newClient(t, Options{APIKey: "key", ServerURL: s.URL, Workspace: "ws"})
		if _, err := c.Deployments.List(ctx, &ListDeploymentsParams{Project: Ptr("web"), Status: []string{"running", "failed"}, Limit: Ptr[int64](10)}); err != nil {
			t.Fatal(err)
		}
		s = server(t, "GET /v1/deployments", "Bearer key", 200, page)
		if _, err := newClient(t, Options{APIKey: "key", ServerURL: s.URL}).Deployments.List(ctx, nil); err != nil {
			t.Fatal(err)
		}
		// A zero value is set, not left out: only nil fields are.
		s = server(t, "GET /v1/deployments?project=&limit=0", "Bearer key", 200, page)
		if _, err := newClient(t, Options{APIKey: "key", ServerURL: s.URL}).Deployments.List(ctx, &ListDeploymentsParams{Project: Ptr(""), Limit: Ptr[int64](0)}); err != nil {
			t.Fatal(err)
		}
	})

	t.Run("an optional body, set and nil", func(t *testing.T) {
		var bodies []string
		s := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
			data, _ := io.ReadAll(r.Body)
			bodies = append(bodies, r.Method+" "+r.URL.Path+" "+string(data))
			w.Header().Set("Content-Type", "application/json")
			_, _ = w.Write([]byte(`{"token":"t","deploymentLink":"https://example.com/l"}`))
		}))
		t.Cleanup(s.Close)
		c := newClient(t, Options{APIKey: "key", ServerURL: s.URL})
		_, err1 := c.DeploymentGroups.CreateToken(ctx, "dg_1", &baml_sdk.PlatformCreateDeploymentGroupTokenRequest{Description: Ptr("CI")})
		_, err2 := c.DeploymentGroups.CreateToken(ctx, "dg_1", nil)
		if err1 != nil || err2 != nil {
			t.Fatal(err1, err2)
		}
		want := []string{`POST /v1/deployment-groups/dg_1/tokens {"description":"CI"}`, `POST /v1/deployment-groups/dg_1/tokens `}
		if fmt.Sprint(bodies) != fmt.Sprint(want) {
			t.Errorf("got %q", bodies)
		}
	})

	t.Run("a union tagged by two fields has one catch-all", func(t *testing.T) {
		// A deployment with an S3 binding and a `storage` binding whose
		// `service`, `minio`, this SDK does not know (docs/api-shape.md, "Unions").
		data, err := os.ReadFile(filepath.Join(repoRoot(), "conformance", "scenarios", "ops.platform.deployments.get.external_binding_new_service.json"))
		if err != nil {
			t.Fatal(err)
		}
		var file struct {
			Exchanges []struct {
				Respond struct {
					JSON json.RawMessage `json:"json"`
				} `json:"respond"`
			} `json:"exchanges"`
		}
		if err := json.Unmarshal(data, &file); err != nil {
			t.Fatal(err)
		}
		s := server(t, "GET /v1/deployments/dep_1", "Bearer key", 200, string(file.Exchanges[0].Respond.JSON))
		d, err := newClient(t, Options{APIKey: "key", ServerURL: s.URL}).Deployments.Get(ctx, "dep_1", nil)
		if err != nil {
			t.Fatal(err)
		}
		got := map[string]any{}
		for name, b := range *d.StackSettings.ExternalBindings {
			switch b := b.(type) {
			case baml_sdk.PlatformExternalBindingOther:
				got[name] = b.Raw["service"]
			case baml_sdk.PlatformExternalBindingStorageS3:
				got[name] = b.BucketName
			}
		}
		if fmt.Sprint(got) != "map[files:files media:minio]" {
			t.Errorf("got %v", got)
		}
	})

	t.Run("Err converts a failure", func(t *testing.T) {
		s := server(t, "DELETE /v1/api-keys/key_1", "Bearer key", 404, `{"code":"NOT_FOUND","message":"no such key","retryable":false}`)
		err := Err(baml_sdk.PlatformApiKeysRevoke(ctx, newClient(t, Options{APIKey: "key", ServerURL: s.URL}).Platform, "key_1"))
		var alienErr *AlienError
		if !errors.As(err, &alienErr) || alienErr.Code != "NOT_FOUND" || alienErr.HTTPStatus == nil || *alienErr.HTTPStatus != 404 {
			t.Fatalf("got %T %v", err, err)
		}
	})

	t.Run("Setup uses the client as it is now, with nil parameters", func(t *testing.T) {
		s := server(t, "GET /v1/projects/my-project/template-urls?workspace=later", "Bearer key", 200,
			`{"aws":{"templateUrl":"https://example.com/setup.yaml","launchStackUrl":"https://example.com/launch"}}`)
		c := newClient(t, Options{APIKey: "key", ServerURL: s.URL, Workspace: "first"})
		c.Platform.Workspace = Ptr("later")
		url, err := c.Setup.LaunchURLForProject(ctx, "my-project", "us-west-2", "alien-setup", nil)
		if err != nil {
			t.Fatal(err)
		}
		if want := "https://us-west-2.console.aws.amazon.com/cloudformation/home?region=us-west-2#/stacks/create/review?templateURL=https%3A%2F%2Fexample.com%2Fsetup.yaml&stackName=alien-setup"; url != want {
			t.Errorf("got %s", url)
		}
	})

	t.Run("Manager token", func(t *testing.T) {
		s := server(t, "GET /v1/commands/cmd_1", "Bearer tok", 404, `{"code":"COMMAND_NOT_FOUND","message":"no such command"}`)
		_, err := NewManager(ManagerOptions{ServerURL: s.URL, Token: "tok"}).Commands.GetCommandStatus(ctx, "cmd_1")
		var alienErr *AlienError
		if !errors.As(err, &alienErr) || alienErr.Code != "COMMAND_NOT_FOUND" {
			t.Fatalf("got %T %v", err, err)
		}
	})

	t.Run("a context deadline passes through unchanged", func(t *testing.T) {
		s := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
			select {
			case <-r.Context().Done():
			case <-time.After(10 * time.Second):
			}
		}))
		t.Cleanup(s.Close)
		short, cancel := context.WithTimeout(ctx, 200*time.Millisecond)
		defer cancel()
		err := Err(baml_sdk.PlatformApiKeysRevoke(short, newClient(t, Options{APIKey: "key", ServerURL: s.URL}).Platform, "key_1"))
		var alienErr *AlienError
		if !errors.Is(err, context.DeadlineExceeded) || errors.As(err, &alienErr) {
			t.Fatalf("got %T %v", err, err)
		}
	})
}

func TestCheckIsolated(t *testing.T) {
	user, err := os.UserHomeDir()
	if err != nil {
		t.Fatal(err)
	}
	for _, c := range []struct{ home, bridge, noDownload, want string }{
		{"/srv/baml-home", "/lib.dylib", "", ""},
		{"/srv/baml-home", "", "true", ""},
		{"", "/lib.dylib", "true", "BAML_HOME is not set"},
		{filepath.Join(user, ".baml") + "/", "/lib.dylib", "true", "BAML_HOME is ~/.baml"},
		{"/srv/baml-home", "", "", "BAML_BRIDGE_DISABLE_DOWNLOAD=true"},
	} {
		env := map[string]string{"BAML_HOME": c.home, "BAML_BRIDGE_PATH": c.bridge, "BAML_BRIDGE_DISABLE_DOWNLOAD": c.noDownload}
		err := checkIsolated(func(name string) string { return env[name] })
		if (c.want == "") != (err == nil) || (err != nil && !strings.Contains(err.Error(), c.want)) {
			t.Errorf("%+v: got %v", c, err)
		}
	}
}
