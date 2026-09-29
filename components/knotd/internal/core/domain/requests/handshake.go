package requests

import (
	"github.com/kand1ss/knot-ops/components/knotd/internal/core/domain"
	"github.com/kand1ss/knot-ops/components/knotd/internal/core/values"
)

type ManifestSyncState int

const (
	ManifestSyncStateInSync = iota
	ManifestSyncStateOutOfSync
	ManifestSyncStateUnregistered
)

type HandshakeRequest struct {
	WorkspaceID values.WorkspaceId
	Manifest    domain.WorkspaceManifest
}

type HandshakeResponse struct {
	State ManifestSyncState
}
