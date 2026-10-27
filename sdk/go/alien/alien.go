// Package alien is the Alien SDK for Go: a thin facade over the generated BAML
// SDK (package baml_sdk). It builds clients and converts errors; every
// behavior lives in the BAML project (baml/baml_src). Option names follow
// Alien's TypeScript SDK.
//
// Every operation is a method, grouped as in Alien's TypeScript SDK
// (c.<Group>.<Method>). Required arguments are positional; optional ones are a
// trailing *<OperationID>Params, or the request body itself when that is the
// only optional argument. Failures are *AlienError:
//
//	c, err := alien.New(alien.Options{Workspace: "my-workspace"}) // API key from ALIEN_API_KEY
//	m, err := c.Managers.Get(ctx, "mgr_...")
//	page, err := c.Deployments.List(ctx, &alien.ListDeploymentsParams{Project: alien.Ptr("my-project"), Limit: alien.Ptr[int64](10)})
//
// Request and response types are in package baml_sdk (Platform<Name>,
// Manager<Name>). The methods are generated from it (operations.go).
//
// Package baml_sdk also exports the SDK's own helpers (Internal*) and test
// support (Compat*). They are not part of the SDK: BAML has no visibility
// modifiers (docs/gaps.md#b6).
//
// Call Shutdown before the process exits: Go never shuts the runtime down by
// itself, and the telemetry recording stays unsealed without it. With
// ALIEN_TELEMETRY=off it is not needed.
package alien

import (
	"cmp"
	"context"
	"fmt"
	"log/slog"
	"os"
	"time"

	baml_go "github.com/boundaryml/baml-go"

	"example.com/alien-baml-sdk/go/baml_sdk"
)

// CloudFormationLaunch is the input of CloudFormationLaunchURL. Every other
// request and response type is in package baml_sdk (Platform<Name>, Manager<Name>).
type CloudFormationLaunch = baml_sdk.SetupCloudFormationLaunch

// DefaultServerURL is the Platform API used when Options.ServerURL is empty.
const DefaultServerURL = "https://api.alien.dev"

// Options configures a Client. Printing it never shows the API key.
type Options struct {
	// APIKey is sent as a Bearer token. Empty means the ALIEN_API_KEY
	// environment variable, as in Alien's TypeScript SDK. New fails without
	// either.
	APIKey string
	// ServerURL defaults to DefaultServerURL.
	ServerURL string
	// Workspace is optional for workspace-scoped API keys. Empty means the
	// ALIEN_WORKSPACE environment variable, as in Alien's TypeScript SDK.
	Workspace string
	// Timeout limits each HTTP request, from connecting to reading the whole
	// response, rounded up to whole milliseconds (Alien's TypeScript SDK:
	// timeoutMs). Zero means BAML's default, 5 minutes; a negative Timeout
	// means no limit. A request that runs out fails with a COMMUNICATION_ERROR
	// *AlienError (retryable). The ctx deadline of a call still applies: the
	// earlier one ends the call.
	Timeout time.Duration
}

// Client is a Platform API client; create it with New. Its operations are
// the PlatformAPI groups: c.Managers.Get(ctx, id).
//
// Printing a Client never shows the API key, but printing c.Platform does:
// it is a generated type.
type Client struct {
	PlatformAPI
	// Platform is the generated client every method uses, as it is when the
	// method is called. Generated functions take it directly; wrap them in Do.
	Platform baml_sdk.PlatformPlatformClient
	// Setup holds the setup helpers that need the Platform API.
	Setup ProjectSetup
}

// New returns a Client. Without an API key it fails with an INVALID_REQUEST
// *AlienError; Alien's TypeScript SDK would send requests without one.
func New(options Options) (*Client, error) {
	apiKey := cmp.Or(options.APIKey, os.Getenv("ALIEN_API_KEY"))
	if apiKey == "" {
		return nil, &AlienError{Code: "INVALID_REQUEST", Message: "no API key: pass Options.APIKey or set ALIEN_API_KEY"}
	}
	c := &Client{Platform: baml_sdk.PlatformPlatformClient{
		BaseUrl:   cmp.Or(options.ServerURL, DefaultServerURL),
		ApiKey:    apiKey,
		Workspace: optional(cmp.Or(options.Workspace, os.Getenv("ALIEN_WORKSPACE"))),
		TimeoutMs: timeoutMs(options.Timeout),
	}}
	c.PlatformAPI = newPlatformAPI(c)
	c.Setup = ProjectSetup{client: c}
	return c, nil
}

// ManagerOptions configures a Manager API client. Printing it never shows the
// token.
type ManagerOptions struct {
	// ServerURL is the Manager's URL, for example ManagerUrl from
	// c.Commands.Bootstrap.
	ServerURL string
	// Token is sent as a Bearer token on every operation when set, for example
	// Token from c.Commands.Bootstrap (@alienplatform/commands names it
	// token).
	Token string
	// Timeout is Options.Timeout for Manager requests.
	Timeout time.Duration
}

// ManagerClient is a Manager API client; create it with NewManager. Its
// operations are the ManagerAPI groups: m.Commands.CreateCommand(ctx, req).
type ManagerClient struct {
	ManagerAPI
	// Generated is the generated client every method uses. (It is not named
	// Manager: m.Manager is the Manager API's manager group.)
	Generated baml_sdk.ManagerManagerClient
}

// NewManager returns a Manager API client, for example for the ManagerUrl and
// Token that c.Commands.Bootstrap returns.
func NewManager(options ManagerOptions) *ManagerClient {
	m := &ManagerClient{Generated: baml_sdk.ManagerManagerClient{
		BaseUrl:   options.ServerURL,
		Token:     optional(options.Token),
		TimeoutMs: timeoutMs(options.Timeout),
	}}
	m.ManagerAPI = newManagerAPI(m)
	return m
}

// optional is an optional string field of a generated client: nil when empty.
func optional(s string) *string {
	if s == "" {
		return nil
	}
	return &s
}

// timeoutMs is a client Timeout as the generated clients take it: nil for
// zero (BAML's default), -1 for a negative Timeout (no limit), else whole
// milliseconds, rounded up so that a sub-millisecond Timeout still limits.
func timeoutMs(timeout time.Duration) *int64 {
	switch {
	case timeout == 0:
		return nil
	case timeout < 0:
		return Ptr[int64](-1)
	}
	ms := timeout.Milliseconds()
	if time.Duration(ms)*time.Millisecond < timeout {
		ms++
	}
	return &ms
}

// Do passes a generated operation's result through and converts a thrown
// AlienError into *AlienError (docs/gaps.md#b5). Every method does this; call
// it yourself only around a generated function:
//
//	m, err := alien.Do(baml_sdk.PlatformManagersGet(ctx, c.Platform, id))
//
// Other errors pass through unchanged (see AlienError).
func Do[R any](result R, err error) (R, error) {
	return result, convert(err)
}

// Err is Do for operations that return only an error:
//
//	err := alien.Err(baml_sdk.PlatformApiKeysRevoke(ctx, c.Platform, keyID))
func Err(err error) error {
	return convert(err)
}

// Ptr returns a pointer to v, for optional arguments and optional fields of
// request bodies: &alien.ListDeploymentsParams{Limit: alien.Ptr[int64](10)},
// &baml_sdk.PlatformCreateDeploymentGroupTokenRequest{Description: alien.Ptr("CI")}.
// From Go 1.26, new("CI") does the same.
func Ptr[T any](v T) *T {
	return &v
}

// ProjectSetup holds the setup helpers that need the Platform API (Client.Setup).
type ProjectSetup struct {
	client *Client
}

// LaunchURLForProject fetches the project's AWS template and builds a
// quick-create link for it. A nil parameters map adds no parameters.
func (s ProjectSetup) LaunchURLForProject(ctx context.Context, project, region, stackName string, parameters map[string]string) (string, error) {
	return Do(baml_sdk.SetupLaunchUrlForProject(ctx, s.client.Platform, project, region, stackName, parameters))
}

// CloudFormationLaunchURL builds an AWS CloudFormation quick-create link from
// a template URL.
func CloudFormationLaunchURL(ctx context.Context, input CloudFormationLaunch) (string, error) {
	return Do(baml_sdk.SetupCloudformationLaunchUrl(ctx, input))
}

// Shutdown shuts the SDK's runtime down: it waits for calls in flight and
// seals the telemetry recording.
//
// Only telemetry needs it. Go has no exit hook that would do this, so unless
// telemetry is off (ALIEN_TELEMETRY=off), call it before the process exits,
// typically deferred in main (see the example); os.Exit and log.Fatal skip
// deferred calls, so call it before those too. Otherwise the recording is not
// sealed (docs/runtime.md). The error it returns can only be logged.
//
// Before the first SDK call it does nothing. After it, every SDK call fails
// with a runtime error that is not an *AlienError ("BAML returned an invalid
// zero call ID").
func Shutdown() error {
	return baml_go.Shutdown()
}

// Format prints o without its API key, for every verb (%v, %+v, %#v, %s, ...).
func (o Options) Format(f fmt.State, _ rune) {
	fmt.Fprintf(f, "alien.Options{APIKey:%s ServerURL:%q Workspace:%q Timeout:%v}", redact(o.APIKey), o.ServerURL, o.Workspace, o.Timeout)
}

// LogValue keeps the API key out of log/slog output, JSON handlers included.
func (o Options) LogValue() slog.Value { return slog.StringValue(fmt.Sprint(o)) }

// Format prints o without its token, for every verb.
func (o ManagerOptions) Format(f fmt.State, _ rune) {
	fmt.Fprintf(f, "alien.ManagerOptions{ServerURL:%q Token:%s Timeout:%v}", o.ServerURL, redact(o.Token), o.Timeout)
}

// LogValue keeps the token out of log/slog output, JSON handlers included.
func (o ManagerOptions) LogValue() slog.Value { return slog.StringValue(fmt.Sprint(o)) }

// Format prints c without its API key, for every verb.
func (c Client) Format(f fmt.State, _ rune) {
	workspace := ""
	if c.Platform.Workspace != nil {
		workspace = *c.Platform.Workspace
	}
	fmt.Fprintf(f, "alien.Client{APIKey:%s ServerURL:%q Workspace:%q}", redact(c.Platform.ApiKey), c.Platform.BaseUrl, workspace)
}

// LogValue keeps the API key out of log/slog output, JSON handlers included.
func (c Client) LogValue() slog.Value { return slog.StringValue(fmt.Sprint(c)) }

func redact(secret string) string {
	if secret == "" {
		return `""`
	}
	return "<redacted>"
}
