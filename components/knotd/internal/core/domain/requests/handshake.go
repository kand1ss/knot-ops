package requests

import "github.com/kand1ss/knot-ops/components/knotd/internal/core/domain"

type DriftReason int

const (
	DriftReasonNone = iota
	DriftReasonNewWorkspace
	DriftReasonConfigChanged
)

type HandshakeRequest struct {
	Manifest domain.WorkspaceManifest
	Metadata domain.WorkspaceMetadata
}

type HandshakeResponse struct {
	WorkspaceState domain.WorkspaceState
	DriftReason    DriftReason
}
