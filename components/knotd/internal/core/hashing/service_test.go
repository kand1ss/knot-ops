package hashing

import (
	"testing"

	"github.com/kand1ss/knot-ops/components/knotd/internal/core/domain"
	"github.com/kand1ss/knot-ops/components/knotd/internal/core/values"
)

// Helper function to create a base ServiceSpec for testing.
func baseService(name values.ServiceName) domain.ServiceSpec {
	return domain.ServiceSpec{
		Name:      name,
		Command:   "go run main.go",
		Directory: "/app",
		Depends:   []values.ServiceName{"db", "redis"},
		Env: map[string]string{
			"PORT": "8080",
			"ENV":  "production",
		},
	}
}

// Helper function to perform a deep copy of a ServiceSpec.
func copyService(src domain.ServiceSpec) domain.ServiceSpec {
	dst := src
	dst.Depends = append([]values.ServiceName(nil), src.Depends...)
	dst.Env = make(map[string]string, len(src.Env))
	for k, v := range src.Env {
		dst.Env[k] = v
	}
	return dst
}

// 1. Consistency: Repeated calls on the same object or identical objects return the same hash.
func TestServiceHash_Consistency(t *testing.T) {
	svc1 := baseService("api")
	svc2 := copyService(svc1)

	hash1, _ := ServiceHash(svc1)
	hash2, _ := ServiceHash(svc1) // Repeated call on the exact same instance
	hash3, _ := ServiceHash(svc2) // Call on an identical instance

	if hash1 != hash2 {
		t.Errorf("repeated call returned a different hash: %x != %x", hash1, hash2)
	}
	if hash1 != hash3 {
		t.Errorf("identical services returned different hashes: %x != %x", hash1, hash3)
	}
}

// 2. Field mutation: Changing any field must result in a different hash.
func TestServiceHash_FieldMutation(t *testing.T) {
	base := baseService("api")
	baseHash, _ := ServiceHash(base)

	tests := []struct {
		name   string
		modify func(s *domain.ServiceSpec)
	}{
		{
			name: "Command changed",
			modify: func(s *domain.ServiceSpec) {
				s.Command = "go run server.go"
			},
		},
		{
			name: "Directory changed",
			modify: func(s *domain.ServiceSpec) {
				s.Directory = "/opt/app"
			},
		},
		{
			name: "Depends added",
			modify: func(s *domain.ServiceSpec) {
				s.Depends = append(s.Depends, values.ServiceName("rabbitmq"))
			},
		},
		{
			name: "Depends modified",
			modify: func(s *domain.ServiceSpec) {
				s.Depends[0] = "postgres"
			},
		},
		{
			name: "Env key added",
			modify: func(s *domain.ServiceSpec) {
				s.Env["DEBUG"] = "true"
			},
		},
		{
			name: "Env value changed",
			modify: func(s *domain.ServiceSpec) {
				s.Env["PORT"] = "9090"
			},
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			modified := copyService(base)
			tt.modify(&modified)

			newHash, _ := ServiceHash(modified)
			if newHash == baseHash {
				t.Errorf("modifying field (%s) did not change service hash", tt.name)
			}
		})
	}
}

func TestServiceHash_NoCollisions(t *testing.T) {
	svcA := domain.ServiceSpec{
		Command:   "run\x00directory=/tmp",
		Directory: "/srv",
	}

	svcB := domain.ServiceSpec{
		Command:   "run",
		Directory: "/tmp\x00directory=/srv",
	}

	hashA, errA := ServiceHash(svcA)
	if errA != nil {
		t.Fatalf("failed to calculate hash for A: %v", errA)
	}

	hashB, errB := ServiceHash(svcB)
	if errB != nil {
		t.Fatalf("failed to calculate hash for B: %v", errB)
	}

	if hashA == hashB {
		t.Fatalf("hash collision detected: distinct services produced identical hash %x", hashA)
	}
}

func TestServiceHash_EnvNoCollisions(t *testing.T) {
	tests := []struct {
		name string
		svcA domain.ServiceSpec
		svcB domain.ServiceSpec
	}{
		{
			name: "injection of zero byte and fake env key into value",
			svcA: domain.ServiceSpec{
				Env: map[string]string{
					"FOO": "bar\x00env.BAZ=qux",
				},
			},
			svcB: domain.ServiceSpec{
				Env: map[string]string{
					"FOO": "bar",
					"BAZ": "qux",
				},
			},
		},
		{
			name: "attempt to mimic length-prefix format inside value",
			svcA: domain.ServiceSpec{
				Env: map[string]string{
					"FOO": "bar:7:env:BAZ:3:qux",
				},
			},
			svcB: domain.ServiceSpec{
				Env: map[string]string{
					"FOO": "bar",
					"BAZ": "qux",
				},
			},
		},
		{
			name: "overlapping key-value boundaries with colon delimiter",
			svcA: domain.ServiceSpec{
				Env: map[string]string{
					"A:B": "C",
				},
			},
			svcB: domain.ServiceSpec{
				Env: map[string]string{
					"A": "B:C",
				},
			},
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			hashA, errA := ServiceHash(tt.svcA)
			if errA != nil {
				t.Fatalf("unexpected error hashing svcA: %v", errA)
			}

			hashB, errB := ServiceHash(tt.svcB)
			if errB != nil {
				t.Fatalf("unexpected error hashing svcB: %v", errB)
			}

			if hashA == hashB {
				t.Errorf("collision detected in %q: distinct services produced identical hash %x", tt.name, hashA)
			}
		})
	}
}
