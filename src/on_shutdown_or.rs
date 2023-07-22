//! For advanced usage of [`on_shutdown_or`](crate::TasksHandle::on_shutdown_or)

use {
	futures::prelude::*,
	std::{sync::atomic::AtomicBool, task::Poll},
	tokio_util::sync::{CancellationToken, WaitForCancellationFuture},
};

pin_project_lite::pin_project! {
	/// Returned by [`on_shutdown_or`](crate::TasksHandle::on_shutdown_or)
	///
	/// This `Future` resolves to [`ShouldShutdownOr<F::Output>`](ShouldShutdownOr).
	#[must_use = "futures do nothing unless polled"]
	pub struct OnShutdownOr<'a, F> {
		is_shutting_down: &'a AtomicBool,
		#[pin]
		fut: F,
		leaf_cancellation_token: &'a CancellationToken,
		#[pin]
		wait_for_cancellation_future: Option<WaitForCancellationFuture<'a>>,
	}
}

impl<'a, F: Future> OnShutdownOr<'a, F> {
	pub(crate) fn new(
		is_shutting_down: &'a AtomicBool,
		leaf_cancellation_token: &'a CancellationToken,
		fut: F,
	) -> Self {
		Self {
			is_shutting_down,
			fut,
			leaf_cancellation_token,
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
		if self.is_shutting_down.load(std::sync::atomic::Ordering::Relaxed) {
			Poll::Ready(ShouldShutdownOr::ShouldShutdown)
		} else {
			let mut p = self.project();
			match p.fut.poll(cx) {
				Poll::Ready(v) => Poll::Ready(ShouldShutdownOr::ShouldNotShutdown(v)),
				Poll::Pending => {
					if p.wait_for_cancellation_future.is_none() {
						p.wait_for_cancellation_future
							.set(Some(p.leaf_cancellation_token.cancelled()));
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
