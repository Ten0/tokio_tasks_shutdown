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

impl<E> SystemError<E> {
	pub fn system_name(&self) -> &str {
		&self.system_name
	}

	pub fn kind(&self) -> &SystemErrorKind<E> {
		&self.kind
	}

	pub fn into_kind(self) -> SystemErrorKind<E> {
		self.kind
	}
}

#[derive(thiserror::Error)]
pub enum SystemErrorKind<E> {
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
	pub(crate) system_name: String,
	pub(crate) system_that_failed_to_start: SystemType,
}

impl<SystemType> SystemsAreStopping<SystemType> {
	pub fn system_name(&self) -> &str {
		&self.system_name
	}

	pub fn into_system_that_failed_to_start(self) -> SystemType {
		self.system_that_failed_to_start
	}
}
