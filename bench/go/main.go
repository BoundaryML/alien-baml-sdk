// Benchmarks for the Go SDK (bench/run.py drives it and lists the modes).
// Every mode prints one line, then waits for stdin EOF so the driver can read RSS.
package main

import (
	"context"
	"encoding/json"
	"fmt"
	"io"
	"os"
	"strconv"
	"time"

	"example.com/alien-baml-sdk/go/alien"
	"example.com/alien-baml-sdk/go/baml_sdk"
)

const id = "mgr_0c29fq4a2yjb7kx3smwdgxlcmpqr"

var input = alien.CloudFormationLaunch{
	TemplateUrl: "https://example.com/setup.yaml", Region: "us-east-1", StackName: "alien-setup",
	Parameters: map[string]string{"ExternalId": "customer-0001"},
}

func must[T any](v T, err error) T {
	if err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
	return v
}

func atoi(s string) int { return must(strconv.Atoi(s)) }

// batches returns the per-call nanoseconds of call, one value per batch, after warmup calls.
func batches(call func() error, count, n, warmup int) []float64 {
	for i := 0; i < warmup; i++ {
		must(0, call())
	}
	perCall := make([]float64, 0, count)
	for b := 0; b < count; b++ {
		start := time.Now()
		for i := 0; i < n; i++ {
			must(0, call())
		}
		perCall = append(perCall, float64(time.Since(start).Nanoseconds())/float64(n))
	}
	return perCall
}

func done(line string) {
	fmt.Println(line)
	_, _ = io.ReadAll(os.Stdin)
}

func jsonLine(v any) string { return string(must(json.Marshal(v))) }

func main() {
	ctx := context.Background()
	args := os.Args[2:]
	switch os.Args[1] {
	case "import":
		// Go loads the runtime library on the first call; this is process start only.
		_ = must(alien.New(alien.Options{APIKey: "alien_bench_key", ServerURL: "http://127.0.0.1:9"}))
		done("ready")
	case "get":
		client := must(alien.New(alien.Options{APIKey: "alien_bench_key", ServerURL: args[0]}))
		done(must(client.Managers.Get(ctx, id)).Id)
	case "warm-get":
		client := must(alien.New(alien.Options{APIKey: "alien_bench_key", ServerURL: args[0]}))
		done(jsonLine(batches(func() error {
			_, err := client.Managers.Get(ctx, id)
			return err
		}, atoi(args[1]), atoi(args[2]), 200)))
	case "page":
		client := must(alien.New(alien.Options{APIKey: "alien_bench_key", ServerURL: args[0]}))
		size, warmup, n := atoi(args[1]), atoi(args[2]), atoi(args[3])
		limit := int64(size)
		ms := make([]float64, 0, n)
		for i := 0; i < warmup+n; i++ {
			start := time.Now()
			page := must(client.Managers.List(ctx, &alien.ListManagersParams{Limit: &limit}))
			if i >= warmup {
				ms = append(ms, float64(time.Since(start).Nanoseconds())/1e6)
			}
			last := page[len(page)-1]
			if len(page) != size || last.Metrics == nil || last.Metrics.CpuUsagePercent == nil || *last.Metrics.CpuUsagePercent != 12.25 {
				fmt.Fprintln(os.Stderr, "bad page")
				os.Exit(1)
			}
		}
		done(jsonLine(ms))
	case "warm-helper", "warm-noop":
		call := func() error { _, err := alien.CloudFormationLaunchURL(ctx, input); return err }
		if os.Args[1] == "warm-noop" {
			call = func() error { _, err := baml_sdk.CompatNewerRemoteSandbox(ctx); return err }
		}
		done(jsonLine(batches(call, atoi(args[0]), atoi(args[1]), 1000)))
	default:
		fmt.Fprintln(os.Stderr, "unknown mode", os.Args[1])
		os.Exit(2)
	}
}
