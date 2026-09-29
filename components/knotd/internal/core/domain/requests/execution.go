package requests

type CancelExecutionRequest struct {
	ExecutionID string
}

type CancelExecutionResponse struct {
	Cancelled bool
	Code      CancelExecutionCode
}

type CancelExecutionCode int

const (
	CancelExecutionCodeSuccess = iota
	CancelExecutionCodeNotFound
	CancelExecutionCodeAlreadyCompleted
	CancelExecutionCodeAlreadyCancelling
)
