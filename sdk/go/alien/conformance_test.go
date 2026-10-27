// Runs every conformance scenario (conformance/scenarios): Platform and setup
// calls through the facade, compat.* calls through the generated SDK with the
// facade's client and error conversion.
package alien

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"testing"
	"time"

	baml_go "github.com/boundaryml/baml-go"

	"example.com/alien-baml-sdk/go/baml_sdk"
)

type clientArgs struct {
	BaseURL   string `json:"base_url"`
	APIKey    string `json:"api_key"`
	Token     string `json:"token"`
	Workspace string `json:"workspace"` // null decodes as ""
	TimeoutMs *int64 `json:"timeout_ms"`
}

// timeout is the scenario client's timeout_ms as Options.Timeout.
func (c clientArgs) timeout() time.Duration {
	if c.TimeoutMs == nil {
		return 0
	}
	return time.Duration(*c.TimeoutMs) * time.Millisecond
}

func decode[T any](args map[string]json.RawMessage, key string) T {
	var v T
	raw, ok := args[key]
	if !ok {
		panic(fmt.Sprintf("scenario args have no %q", key))
	}
	if err := json.Unmarshal(raw, &v); err != nil {
		panic(fmt.Sprintf("scenario arg %q: %v", key, err))
	}
	return v
}

func facade(args map[string]json.RawMessage) *Client {
	c := decode[clientArgs](args, "client")
	client, err := New(Options{APIKey: c.APIKey, ServerURL: c.BaseURL, Workspace: c.Workspace, Timeout: c.timeout()})
	if err != nil {
		panic(fmt.Sprintf("scenario client: %v", err))
	}
	return client
}

func managerFacade(args map[string]json.RawMessage) *ManagerClient {
	c := decode[clientArgs](args, "client")
	return NewManager(ManagerOptions{ServerURL: c.BaseURL, Token: c.Token, Timeout: c.timeout()})
}

func wrap[T any](v T, err error) (any, error) { return v, err }

// compatJSON calls a compat function that takes the scenario's json argument.
func compatJSON[R any](fn func(context.Context, string, ...baml_go.CallOption) (R, error)) operation {
	return func(ctx context.Context, a map[string]json.RawMessage) (any, error) {
		return wrap(Do(fn(ctx, decode[string](a, "json"))))
	}
}

func bootstrapRequest(raw map[string]json.RawMessage) baml_sdk.PlatformCommandBootstrapRequest {
	request := decode[map[string]json.RawMessage](raw, "request")
	if decode[string](request, "role") == "receiver" {
		return baml_sdk.NewPlatformCommandBootstrapRequestOtherOrPlatformCommandBootstrapRequestReceiverOrPlatformCommandBootstrapRequestSenderFromPlatformCommandBootstrapRequestReceiver(
			decode[baml_sdk.PlatformCommandBootstrapRequestReceiver](raw, "request"))
	}
	return baml_sdk.NewPlatformCommandBootstrapRequestOtherOrPlatformCommandBootstrapRequestReceiverOrPlatformCommandBootstrapRequestSenderFromPlatformCommandBootstrapRequestSender(
		decode[baml_sdk.PlatformCommandBootstrapRequestSender](raw, "request"))
}

var calls = map[string]operation{
	"platform.createSetupLink": func(ctx context.Context, a map[string]json.RawMessage) (any, error) {
		return wrap(facade(a).SetupLinks.Create(ctx, decode[baml_sdk.PlatformCreateSetupLinkRequest](a, "request")))
	},
	"platform.getProjectTemplateUrls": func(ctx context.Context, a map[string]json.RawMessage) (any, error) {
		return wrap(facade(a).Projects.GetTemplateURLs(ctx, decode[string](a, "idOrName")))
	},
	"platform.bootstrapCommands": func(ctx context.Context, a map[string]json.RawMessage) (any, error) {
		return wrap(facade(a).Commands.Bootstrap(ctx, bootstrapRequest(a)))
	},
	"platform.getManager": func(ctx context.Context, a map[string]json.RawMessage) (any, error) {
		return wrap(facade(a).Managers.Get(ctx, decode[string](a, "id")))
	},
	"manager.getCommandStatus": func(ctx context.Context, a map[string]json.RawMessage) (any, error) {
		return wrap(managerFacade(a).Commands.GetCommandStatus(ctx, decode[string](a, "command_id")))
	},
	"setup.cloudformationLaunchUrl": func(ctx context.Context, a map[string]json.RawMessage) (any, error) {
		return wrap(CloudFormationLaunchURL(ctx, decode[CloudFormationLaunch](a, "input")))
	},
	"setup.launchUrlForProject": func(ctx context.Context, a map[string]json.RawMessage) (any, error) {
		return wrap(facade(a).Setup.LaunchURLForProject(ctx, decode[string](a, "project"), decode[string](a, "region"),
			decode[string](a, "stackName"), decode[map[string]string](a, "parameters")))
	},
	"compat.acceptKeySetup": func(ctx context.Context, a map[string]json.RawMessage) (any, error) {
		return wrap(Do(baml_sdk.CompatAcceptKeySetup(ctx, facade(a).Platform, decode[baml_sdk.CompatKeySetupAction](a, "request"))))
	},
	"compat.decodeProjectPage":        compatJSON(baml_sdk.CompatDecodeProjectPage),
	"compat.decodeMaterialization":    compatJSON(baml_sdk.CompatDecodeMaterialization),
	"compat.storedSecretPresence":     compatJSON(baml_sdk.CompatStoredSecretPresence),
	"compat.reencodeConfig":           compatJSON(baml_sdk.CompatReencodeConfig),
	"compat.roundtripStackByPlatform": compatJSON(baml_sdk.CompatRoundtripStackByPlatform),
	"compat.roundtripEvent":           compatJSON(baml_sdk.CompatRoundtripEvent),
	"compat.roleScope":                compatJSON(baml_sdk.CompatRoleScope),
	"compat.decodeRemoteAccess":       compatJSON(baml_sdk.CompatDecodeRemoteAccess),
}

// callTimeout bounds one scenario call, so a stuck call fails its scenario
// instead of hanging the suite.
const callTimeout = time.Minute

type verdict struct {
	OK         bool            `json:"ok"`
	Problems   []string        `json:"problems"`
	Normalized json.RawMessage `json:"normalized"`
}

func TestMain(m *testing.M) {
	if err := checkIsolated(os.Getenv); err != nil {
		fmt.Fprintln(os.Stderr, "alien tests:", err)
		os.Exit(2)
	}
	// Scenarios and tests spell out every client option; the developer's
	// defaults must not fill in the ones they leave empty.
	for _, name := range []string{"ALIEN_API_KEY", "ALIEN_WORKSPACE"} {
		if err := os.Unsetenv(name); err != nil {
			fmt.Fprintln(os.Stderr, "alien tests:", err)
			os.Exit(2)
		}
	}
	code := m.Run()
	if err := Shutdown(); err != nil {
		fmt.Fprintln(os.Stderr, "Shutdown:", err)
		code = 1
	}
	os.Exit(code)
}

func TestConformance(t *testing.T) {
	scenarios, err := loadScenarios()
	if err != nil {
		t.Fatal(err)
	}
	if len(scenarios) == 0 {
		t.Fatal("no scenarios found")
	}
	s, err := startStub()
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(s.stop)

	for _, sc := range scenarios {
		t.Run(sc.ID, func(t *testing.T) {
			var loaded struct {
				Call call `json:"call"`
			}
			if err := s.post("/__load", map[string]json.RawMessage{"scenario": sc.Raw}, &loaded); err != nil {
				t.Fatal(err)
			}
			ctx, cancel := context.WithTimeout(t.Context(), callTimeout)
			defer cancel()
			var value any
			var err error
			if loaded.Call.Op != "" {
				value, err = runOperation(ctx, loaded.Call.Op, loaded.Call.Args)
			} else {
				run, ok := calls[loaded.Call.Fn]
				if !ok {
					t.Fatalf("no Go binding for scenario fn %s", loaded.Call.Fn)
				}
				value, err = run(ctx, loaded.Call.Args)
			}
			outcome := map[string]any{}
			if err != nil {
				var alienErr *AlienError
				if !errors.As(err, &alienErr) {
					t.Fatalf("call returned a non-AlienError: %v", err)
				}
				outcome["error"] = map[string]any{"code": alienErr.Code, "retryable": alienErr.Retryable, "http_status": alienErr.HTTPStatus, "message": alienErr.Message, "context": alienErr.Context}
			} else {
				outcome["ok"] = value
			}
			var v verdict
			if err := s.post("/__result", outcome, &v); err != nil {
				t.Fatal(err)
			}
			if out := os.Getenv("CONFORMANCE_OUT"); out != "" {
				dir := filepath.Join(out, "go")
				if err := os.MkdirAll(dir, 0o755); err != nil {
					t.Fatal(err)
				}
				if err := os.WriteFile(filepath.Join(dir, sc.ID+".json"), append(v.Normalized, '\n'), 0o644); err != nil {
					t.Fatal(err)
				}
				if sc.CaptureRaw {
					if err := os.WriteFile(filepath.Join(dir, sc.ID+".txt"), []byte(fmt.Sprint(value)), 0o644); err != nil {
						t.Fatal(err)
					}
				}
			}
			if !v.OK || len(v.Problems) > 0 {
				t.Fatalf("scenario failed: %v", v.Problems)
			}
		})
	}
}

// checkIsolated refuses to run with the developer's BAML state. The runtime
// keeps its state in BAML_HOME (default ~/.baml) and, without
// BAML_BRIDGE_PATH, may download its library into it. scripts/verify.sh sets
// all of this; conformance/README.md shows how to run the tests by hand.
func checkIsolated(getenv func(string) string) error {
	home := getenv("BAML_HOME")
	if home == "" {
		return errors.New("BAML_HOME is not set: point it at an empty directory, never ~/.baml (conformance/README.md#running)")
	}
	if user, err := os.UserHomeDir(); err == nil && filepath.Clean(home) == filepath.Join(user, ".baml") {
		return errors.New("BAML_HOME is ~/.baml: point it at an empty directory (conformance/README.md#running)")
	}
	if getenv("BAML_BRIDGE_PATH") == "" && getenv("BAML_BRIDGE_DISABLE_DOWNLOAD") != "true" {
		return errors.New("set BAML_BRIDGE_PATH to the runtime library, or BAML_BRIDGE_DISABLE_DOWNLOAD=true (docs/runtime.md)")
	}
	return nil
}
