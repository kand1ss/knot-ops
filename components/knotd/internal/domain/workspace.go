package domain

type WorkspaceManifest struct {
	services []ServiceSpec
}

func (w *WorkspaceManifest) Append(service ServiceSpec) {
	w.services = append(w.services, service)
}

func (w *WorkspaceManifest) Get(name string) ServiceSpec {
	for _, service := range w.services {
		if string(service.Name) == name {
			return service
		}
	}
	return ServiceSpec{}
}

func (w *WorkspaceManifest) Services() []ServiceSpec {
	return w.services
}
