use crate::ShouldStop;

use {futures::prelude::*, std::task::Poll, tokio_util::sync::WaitForCancellationFuture};

pin_project_lite::pin_project! {
	pub struct OnShutdownOr<'a, F> {
		should_stop: &'a ShouldStop,
		#[pin]
		fut: F,
		#[pin]
		wait_for_cancellation_future: Option<WaitForCancellationFuture<'a>>,
	}
}

impl<'a, F: Future> OnShutdownOr<'a, F> {
	pub(crate) fn new(should_stop: &'a ShouldStop, fut: F) -> Self {
		Self {
			should_stop,
			fut,
			wait_for_cancellation_future: None,
		}
	}
}

pub enum ShouldShutdownOr<T> {
	ShouldShutdown,
	ShouldNotShutdown(T),
}

impl<'a, F: Future> Future for OnShutdownOr<'a, F> {
	type Output = ShouldShutdownOr<F::Output>;

	fn poll(self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> Poll<Self::Output> {
		if self.should_stop.atomic.load(std::sync::atomic::Ordering::Relaxed) {
			Poll::Ready(ShouldShutdownOr::ShouldShutdown)
		} else {
			let mut p = self.project();
			match p.fut.poll(cx) {
				Poll::Ready(v) => Poll::Ready(ShouldShutdownOr::ShouldNotShutdown(v)),
				Poll::Pending => {
					if p.wait_for_cancellation_future.is_none() {
						p.wait_for_cancellation_future
							.set(Some(p.should_stop.cancellation_token.cancelled()));
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
