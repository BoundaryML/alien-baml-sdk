package alien

import (
	"context"

	"example.com/alien-baml-sdk/go/baml_sdk"
)

// Compile-time check of the Go calls in docs/api-shape.md ("Calling an
// operation"), written as they appear there: one required argument, only
// optional arguments, an optional body, no arguments, no response body, a
// Manager operation.
var _ = func(ctx context.Context, key, id, keyID, managerURL, token string, req baml_sdk.ManagerCreateCommandRequest) (err error) {
	c, err := New(Options{APIKey: key})
	m, err := c.Managers.Get(ctx, id)
	page, err := c.Deployments.List(ctx, &ListDeploymentsParams{Project: Ptr("my-project"), Limit: Ptr[int64](10)})
	t, err := c.DeploymentGroups.CreateToken(ctx, id, nil)
	me, err := c.Auth.Whoami(ctx)
	err = c.APIKeys.Revoke(ctx, keyID)
	cmd, err := NewManager(ManagerOptions{ServerURL: managerURL, Token: token}).Commands.CreateCommand(ctx, req)
	_, _, _, _, _ = m, page, t, me, cmd
	return err
}
