package process

import (
	"errors"
	"os/exec"
	"strings"
)

// mergeEnv overlays overrides on top of base, preserving base's ordering
// for keys not touched by overrides and appending new keys from overrides.
// De-duplication matters here: passing raw concatenated slices to exec.Cmd.Env
// leaves duplicate-key resolution to the child's libc, which is
// platform/implementation defined behavior we must not depend on.
func mergeEnv(base []string, overrides map[string]string) []string {
	if len(overrides) == 0 {
		return base
	}

	values := make(map[string]string, len(base)+len(overrides))
	order := make([]string, 0, len(base)+len(overrides))

	for _, kv := range base {
		k, v, ok := splitEnv(kv)
		if !ok {
			continue
		}
		if _, exists := values[k]; !exists {
			order = append(order, k)
		}
		values[k] = v
	}

	for k, v := range overrides {
		if _, exists := values[k]; !exists {
			order = append(order, k)
		}
		values[k] = v
	}

	result := make([]string, 0, len(order))
	for _, k := range order {
		result = append(result, k+"="+values[k])
	}
	return result
}

func splitEnv(kv string) (key, value string, ok bool) {
	before, after, isOk := strings.Cut(kv, "=")
	if !isOk {
		return "", "", false
	}
	return before, after, true
}

// extractExitCode distinguishes "exited with code N" from "we genuinely
// don't know" (killed by signal, wait() I/O failure). Fabricating a 0 or -1
// in the unknown case would silently corrupt restart-policy decisions
// downstream (v0.3), so unknown stays nil rather than a sentinel value.
func extractExitCode(err error) *int32 {
	if err == nil {
		zero := int32(0)
		return &zero
	}

	var exitErr *exec.ExitError
	if errors.As(err, &exitErr) {
		code := int32(exitErr.ExitCode())
		return &code
	}

	return nil
}
