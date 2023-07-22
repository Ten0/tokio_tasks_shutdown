//! For advanced usage of [`on_shutdown_or`](crate::TasksHandle::on_shutdown_or)

use crate::should_shutdown::{OnShutdownFuture, ShouldShutdown};

use {futures::prelude::*, std::task::Poll};

pin_project_lite::pin_project! {
	/// Returned by [`on_shutdown_or`](crate::TasksHandle::on_shutdown_or)
	///
	/// This `Future` resolves to [`ShouldShutdownOr<F::Output>`](ShouldShutdownOr).
	#[must_use = "futures do nothing unless polled"]
	pub struct OnShutdownOr<'a, F> {
		should_shutdown: &'a ShouldShutdown,
		#[pin]
		fut: F,
		#[pin]
		wait_for_cancellation_future: Option<OnShutdownFuture<'a>>,
	}
}

impl<'a, F: Future> OnShutdownOr<'a, F> {
	pub(crate) fn new(should_shutdown: &'a ShouldShutdown, fut: F) -> Self {
		Self {
			should_shutdown,
			fut,
			wait_for_cancellation_future: None,
		}
	}
}

/// Returned by [`on_shutdown_or`](crate::TasksHandle::on_shutdown_or) (once `await`ed)
pub enum ShouldShutdownOr<T> {
	ShouldShutdown,
	ShouldNotShutdown(T),
}

impl<'a, F: Future> Future for OnShutdownOr<'a, F> {
	type Output = ShouldShutdownOr<F::Output>;

	fn poll(self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> Poll<Self::Output> {
		if self.should_shutdown.should_shutdown() {
			Poll::Ready(ShouldShutdownOr::ShouldShutdown)
		} else {
			let mut p = self.project();
			match p.fut.poll(cx) {
				Poll::Ready(v) => Poll::Ready(ShouldShutdownOr::ShouldNotShutdown(v)),
				Poll::Pending => {
					if p.wait_for_cancellation_future.is_none() {
						p.wait_for_cancellation_future
							.set(Some(p.should_shutdown.on_shutdown()));
					}
					match p.wait_for_cancellation_future.as_pin_mut().unwrap().poll(cx) {
						Poll::Ready(()) => Poll::Ready(ShouldShutdownOr::ShouldShutdown),
						Poll::Pending => Poll::Pending,
					}
				}
			}
		}
	}
}
