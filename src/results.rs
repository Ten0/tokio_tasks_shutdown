pub(crate) struct TaskResult<E> {
	pub(crate) name: String,
	pub(crate) result: Result<(), SystemErrorKind<E>>,
}

#[derive(thiserror::Error)]
#[error("System {system_name} errored: {kind}")]
pub struct SystemError<E> {
	pub(crate) system_name: String,
	pub(crate) kind: SystemErrorKind<E>,
}

#[derive(thiserror::Error)]
pub(crate) enum SystemErrorKind<E> {
	#[error("User error: {0}")]
	UserError(E),
	#[error("Tokio join error: {0}")]
	TokioJoinError(tokio::task::JoinError),
}

#[derive(thiserror::Error, Debug)]
#[error("At least one system errored")]
pub struct AtLeastOneSystemErrored {
	pub(crate) _private: (),
}

#[derive(thiserror::Error, Debug)]
#[error("Task {system_name} was not spawned because we were already stopping")]
pub struct SystemsAreStopping<SystemType> {
	pub system_name: String,
	pub system_that_failed_to_start: SystemType,
	pub(crate) _private: (),
}
