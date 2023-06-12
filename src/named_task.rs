use {futures::prelude::*, tokio::task::JoinHandle};

use crate::results::*;

pin_project_lite::pin_project! {
	#[derive(Debug)]
	pub(crate) struct NamedTask<E> {
		pub(crate) name: Option<String>,
		#[pin]
		pub(crate) task: JoinHandle<Result<(), E>>,
	}
}

impl<E> NamedTask<E> {
	pub(crate) fn abort(&self) {
		self.task.abort()
	}

	pub(crate) fn name(&self) -> &str {
		self.name
			.as_deref()
			.expect("There until resolved, and should be consumed when resolved")
	}
}

impl<E> Future for NamedTask<E> {
	type Output = TaskResult<E>;

	fn poll(self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> std::task::Poll<Self::Output> {
		let this = self.project();
		match JoinHandle::poll(this.task, cx) {
			std::task::Poll::Ready(res) => std::task::Poll::Ready(TaskResult {
				name: this.name.take().expect("Shouldn't be polled after resolving"),
				result: match res {
					Ok(Ok(())) => Ok(()),
					Ok(Err(e)) => Err(TaskErrorKind::UserError(e)),
					Err(e) => Err(TaskErrorKind::TokioJoinError(e)),
				},
			}),
			std::task::Poll::Pending => std::task::Poll::Pending,
		}
	}
}
