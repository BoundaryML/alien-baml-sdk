package alien

import (
	"bufio"
	"bytes"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"sort"
	"strconv"
	"strings"
)

func repoRoot() string {
	_, file, _, _ := runtime.Caller(0)
	return filepath.Clean(filepath.Join(filepath.Dir(file), "..", "..", ".."))
}

type scenario struct {
	ID         string          `json:"id"`
	Phase      float64         `json:"phase"`
	CaptureRaw bool            `json:"capture_raw"`
	Raw        json.RawMessage `json:"-"`
}

// call is a named call (fn), bound by hand, or a generated operation (op),
// run by runOperation.
type call struct {
	Fn   string                     `json:"fn"`
	Op   string                     `json:"op"`
	Args map[string]json.RawMessage `json:"args"`
}

// loadScenarios returns scenarios with phase <= CONFORMANCE_PHASE (every phase when unset).
func loadScenarios() ([]scenario, error) {
	maxPhase := float64(1 << 30)
	if v := os.Getenv("CONFORMANCE_PHASE"); v != "" {
		parsed, err := strconv.ParseFloat(v, 64)
		if err != nil {
			return nil, fmt.Errorf("CONFORMANCE_PHASE: %w", err)
		}
		maxPhase = parsed
	}
	paths, err := filepath.Glob(filepath.Join(repoRoot(), "conformance", "scenarios", "*.json"))
	if err != nil {
		return nil, err
	}
	sort.Strings(paths)
	var out []scenario
	for _, path := range paths {
		raw, err := os.ReadFile(path)
		if err != nil {
			return nil, err
		}
		var s scenario
		if err := json.Unmarshal(raw, &s); err != nil {
			return nil, fmt.Errorf("%s: %w", path, err)
		}
		s.Raw = raw
		if s.Phase <= maxPhase {
			out = append(out, s)
		}
	}
	return out, nil
}

type stub struct {
	cmd  *exec.Cmd
	base string
}

func startStub() (*stub, error) {
	cmd := exec.Command("node", filepath.Join(repoRoot(), "conformance", "stub-server", "server.mjs"))
	cmd.Stderr = os.Stderr
	stdout, err := cmd.StdoutPipe()
	if err != nil {
		return nil, err
	}
	if err := cmd.Start(); err != nil {
		return nil, err
	}
	line, err := bufio.NewReader(stdout).ReadString('\n')
	if err != nil || !strings.HasPrefix(line, "READY ") {
		_ = cmd.Process.Kill()
		return nil, fmt.Errorf("stub server did not start: %q %v", line, err)
	}
	return &stub{cmd: cmd, base: strings.TrimSpace(strings.TrimPrefix(line, "READY "))}, nil
}

func (s *stub) post(path string, body any, out any) error {
	encoded, err := json.Marshal(body)
	if err != nil {
		return err
	}
	resp, err := http.Post(s.base+path, "application/json", bytes.NewReader(encoded))
	if err != nil {
		return err
	}
	defer resp.Body.Close()
	data, err := io.ReadAll(resp.Body)
	if err != nil {
		return err
	}
	if resp.StatusCode != 200 {
		return fmt.Errorf("stub %s: HTTP %d: %s", path, resp.StatusCode, data)
	}
	return json.Unmarshal(data, out)
}

func (s *stub) stop() {
	_ = s.cmd.Process.Kill()
	_ = s.cmd.Wait()
}
