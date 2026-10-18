package task

type Event interface {
	IsServiceEvent()
}

type ServiceEvent struct {
	Service string
	TaskID  string
	Event   Event
}

func (e ServiceEvent) IsTaskEvent() {}

type ServiceError struct {
	Issue    string
	Context  string
	Solution string
}

type StartingEvent struct {
	Message string
}

func (e StartingEvent) IsServiceEvent() {}

type RunningEvent struct {
	Message string
}

func (e RunningEvent) IsServiceEvent() {}

type FailedEvent struct {
	Error ServiceError
}

func (t FailedEvent) IsServiceEvent() {}

type SkippedEvent struct {
	Message string
}

func (t SkippedEvent) IsServiceEvent() {}

type StoppingEvent struct {
	Message string
}

func (e StoppingEvent) IsServiceEvent() {}

type StoppedEvent struct {
	Message string
}

func (e StoppedEvent) IsServiceEvent() {}
