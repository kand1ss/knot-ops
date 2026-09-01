package hashing

import (
	"crypto/sha256"
	"fmt"
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
func ServiceHash(svc domain.ServiceSpec) [32]byte {
	h := sha256.New()

	fmt.Fprintf(h, "command=%s\x00", svc.Command)
	fmt.Fprintf(h, "directory=%s\x00", svc.Directory)

	depends := append([]values.ServiceName(nil), svc.Depends...)
	sort.Slice(depends, func(i, j int) bool { return depends[i] < depends[j] })
	fmt.Fprintf(h, "depends=%s\x00", depends)

	envKeys := make([]string, 0, len(svc.Env))
	for k := range svc.Env {
		envKeys = append(envKeys, k)
	}
	sort.Strings(envKeys)
	for _, k := range envKeys {
		fmt.Fprintf(h, "env.%s=%s\x00", k, svc.Env[k])
	}

	var out [32]byte
	copy(out[:], h.Sum(nil))
	return out
}
