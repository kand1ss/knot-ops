package domain

type WorkspaceManifest struct {
	services []ServiceSpec
}

func (w *WorkspaceManifest) Append(service ServiceSpec) {
	for _, existingService := range w.services {
		if existingService.Name == service.Name {
			return
		}
	}
	w.services = append(w.services, service)
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
	return w.services
}
