package hashing

import (
	"crypto/sha256"
	"fmt"
	"io"
	"sort"

	"github.com/kand1ss/knot-ops/components/knotd/internal/domain"
	"github.com/kand1ss/knot-ops/components/knotd/internal/values"
)

// ServiceHash produces a deterministic content hash for a single service
// definition using length-prefixed key-value pairs to prevent field-boundary
// hash collision ambiguities.
//
// Deliberately does NOT include svc.Name: the name is the *key* callers use
// to look up and compare this hash against a previous manifest's hash for
// the same service.
func ServiceHash(svc domain.ServiceSpec) (Hash, error) {
	h := sha256.New()

	writeString := func(s string) error {
		if _, err := io.WriteString(h, s); err != nil {
			return fmt.Errorf("hash write failed: %w", err)
		}
		return nil
	}

	writeField := func(key, val string) error {
		return writeString(fmt.Sprintf("%d:%s:%d:%s", len(key), key, len(val), val))
	}

	if err := writeField("command", svc.Command); err != nil {
		return [32]byte{}, fmt.Errorf("failed hashing command: %w", err)
	}

	if err := writeField("directory", svc.Directory); err != nil {
		return [32]byte{}, fmt.Errorf("failed hashing directory: %w", err)
	}

	depends := append([]values.ServiceName(nil), svc.Depends...)
	sort.Slice(depends, func(i, j int) bool { return depends[i] < depends[j] })
	for _, dep := range depends {
		if err := writeField("depends", string(dep)); err != nil {
			return [32]byte{}, fmt.Errorf("failed hashing depend %q: %w", dep, err)
		}
	}

	envKeys := make([]string, 0, len(svc.Env))
	for k := range svc.Env {
		envKeys = append(envKeys, k)
	}
	sort.Strings(envKeys)
	for _, k := range envKeys {
		if err := writeField("env:"+k, svc.Env[k]); err != nil {
			return [32]byte{}, fmt.Errorf("failed hashing env key %q: %w", k, err)
		}
	}

	var out [32]byte
	copy(out[:], h.Sum(nil))
	return out, nil
}
