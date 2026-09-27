package domain

import "github.com/kand1ss/knot-ops/components/knotd/internal/core/values"

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

func (w *WorkspaceManifest) Get(name string) (ServiceSpec, bool) {
	for _, service := range w.services {
		if string(service.Name) == name {
			return service, true
		}
	}
	return ServiceSpec{}, false
}

func (w *WorkspaceManifest) Services() []ServiceSpec {
	values := make([]ServiceSpec, 0, len(w.services))
	for _, v := range w.services {
		values = append(values, v)
	}
	return values
}
