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
// definition. Field-terminated with \x00, same canonicalization discipline
// as before — no field-boundary ambiguity from naive concatenation.
//
// Deliberately does NOT include svc.Name: the name is the *key* callers use
// to look up and compare this hash against a previous manifest's hash for
// the same service. Folding the name into the hash would be redundant with
// that key, not additive — the map key already answers "which service",
// this hash answers "did its content change".
func ServiceHash(svc domain.ServiceSpec) (Hash, error) {
	h := sha256.New()

	writeString := func(s string) error {
		if _, err := io.WriteString(h, s); err != nil {
			return fmt.Errorf("hash write failed: %w", err)
		}
		return nil
	}

	if err := writeString(fmt.Sprintf("command=%s\x00", svc.Command)); err != nil {
		return [32]byte{}, fmt.Errorf("failed hashing command: %w", err)
	}

	if err := writeString(fmt.Sprintf("directory=%s\x00", svc.Directory)); err != nil {
		return [32]byte{}, fmt.Errorf("failed hashing directory: %w", err)
	}

	depends := append([]values.ServiceName(nil), svc.Depends...)
	sort.Slice(depends, func(i, j int) bool { return depends[i] < depends[j] })
	for _, dep := range depends {
		if err := writeString(fmt.Sprintf("depends=%s\x00", dep)); err != nil {
			return [32]byte{}, fmt.Errorf("failed hashing depend %q: %w", dep, err)
		}
	}

	envKeys := make([]string, 0, len(svc.Env))
	for k := range svc.Env {
		envKeys = append(envKeys, k)
	}
	sort.Strings(envKeys)
	for _, k := range envKeys {
		if err := writeString(fmt.Sprintf("env.%s=%s\x00", k, svc.Env[k])); err != nil {
			return [32]byte{}, fmt.Errorf("failed hashing env key %q: %w", k, err)
		}
	}

	var out [32]byte
	copy(out[:], h.Sum(nil))
	return out, nil
}
