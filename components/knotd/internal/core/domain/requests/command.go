package requests

type CancelCommandRequest struct {
	CommandId string
	Reason    string
}

type CancelCommandResponse struct {
	Cancelled bool
}
