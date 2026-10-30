package domain

import (
	"fmt"

	"github.com/kand1ss/knot-ops/components/knotd/internal/core/values"
)

type ErrServiceNotInManifest struct {
	Service values.ServiceName
}

func (e ErrServiceNotInManifest) Error() string {
	return fmt.Sprintf("service %q not exists in manifest", e.Service)
}

type WorkspaceManifest struct {
	services map[values.ServiceName]ServiceSpec
}

func NewWorkspaceManifest(services ...ServiceSpec) WorkspaceManifest {
	manifest := WorkspaceManifest{
		services: make(map[values.ServiceName]ServiceSpec, len(services)),
	}

	for _, service := range services {
		manifest.Append(service)
	}

	return manifest
}

func (w *WorkspaceManifest) Append(service ServiceSpec) {
	if w.services == nil {
		w.services = make(map[values.ServiceName]ServiceSpec)
	}

	w.services[service.Name] = service
}

func (w *WorkspaceManifest) Get(name values.ServiceName) (ServiceSpec, error) {
	for _, service := range w.services {
		if service.Name == name {
			return service, nil
		}
	}
	return ServiceSpec{}, ErrServiceNotInManifest{Service: name}
}

func (w *WorkspaceManifest) SpecWaves(waves [][]values.ServiceName) ([][]ServiceSpec, error) {
	out := make([][]ServiceSpec, 0, len(waves))
	for _, wave := range waves {
		specs := make([]ServiceSpec, 0, len(wave))
		for _, n := range wave {
			spec, err := w.Get(n)
			if err != nil {
				return nil, ErrServiceNotInManifest{Service: n}
			}
			specs = append(specs, spec)
		}
		out = append(out, specs)
	}
	return out, nil
}

func (w *WorkspaceManifest) Services() []ServiceSpec {
	values := make([]ServiceSpec, 0, len(w.services))
	for _, v := range w.services {
		values = append(values, v)
	}
	return values
}

func (w *WorkspaceManifest) ServiceNames() []values.ServiceName {
	names := make([]values.ServiceName, 0, len(w.services))
	for _, v := range w.services {
		names = append(names, v.Name)
	}
	return names
}
