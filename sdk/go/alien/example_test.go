package alien_test

import (
	"context"
	"errors"
	"fmt"
	"log"

	"example.com/alien-baml-sdk/go/alien"
	"example.com/alien-baml-sdk/go/baml_sdk"
)

// These examples are compiled but not run: they would call the Alien API.

// Operations are methods of the client's groups. Optional arguments are a
// params struct, or the request body when that is the only one.
func Example() {
	ctx := context.Background()
	c, err := alien.New(alien.Options{Workspace: "my-workspace"}) // the API key comes from ALIEN_API_KEY
	if err != nil {
		log.Print(err) // INVALID_REQUEST: no API key
		return
	}

	m, err := c.Managers.Get(ctx, "mgr_...")
	var alienErr *alien.AlienError
	switch {
	case errors.As(err, &alienErr) && alienErr.HTTPStatus != nil && *alienErr.HTTPStatus == 404:
		fmt.Println("no such manager")
	case err != nil:
		log.Print(err)
	default:
		fmt.Println(m.Id)
	}

	page, err := c.Deployments.List(ctx, &alien.ListDeploymentsParams{Project: alien.Ptr("my-project"), Status: []string{"running"}, Limit: alien.Ptr[int64](10)})
	if err != nil {
		log.Print(err)
		return
	}
	fmt.Println(len(page.Items), page.NextCursor != nil) // NextCursor is set when there are more pages

	token, err := c.DeploymentGroups.CreateToken(ctx, "dg_...", &baml_sdk.PlatformCreateDeploymentGroupTokenRequest{Description: alien.Ptr("CI")})
	if err != nil {
		log.Print(err)
		return
	}
	fmt.Println(token.DeploymentLink)
}

// A Manager client for the URL and token that bootstrapping returns.
func ExampleNewManager() {
	ctx := context.Background()
	c, err := alien.New(alien.Options{})
	if err != nil {
		log.Print(err)
		return
	}
	boot, err := c.Commands.Bootstrap(ctx,
		baml_sdk.NewPlatformCommandBootstrapRequestOtherOrPlatformCommandBootstrapRequestReceiverOrPlatformCommandBootstrapRequestSenderFromPlatformCommandBootstrapRequestSender(
			baml_sdk.PlatformCommandBootstrapRequestSender{DeploymentId: "dep_...", Role: "sender"}))
	if err != nil {
		log.Print(err)
		return
	}
	manager := alien.NewManager(alien.ManagerOptions{ServerURL: boot.ManagerUrl, Token: boot.Token})
	status, err := manager.Commands.GetCommandStatus(ctx, "cmd_...")
	if err != nil {
		log.Print(err)
		return
	}
	fmt.Println(status.State)
}

// Shut the runtime down before main returns, so the telemetry recording is
// sealed. log.Fatal and os.Exit skip deferred calls.
func ExampleShutdown() {
	defer func() {
		if err := alien.Shutdown(); err != nil {
			log.Print(err)
		}
	}()
	// ... SDK calls ...
}
