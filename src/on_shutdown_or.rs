use crate::ShouldStop;

use {
	futures::prelude::*,
	std::{pin::Pin, task::Poll},
	tokio_util::sync::WaitForCancellationFuture,
};

pin_project_lite::pin_project! {
	pub struct OnShutdownOr<'a, F> {
		should_stop: &'a ShouldStop,
		#[pin]
		fut: F,
		ct_cancelled: Option<Pin<Box<WaitForCancellationFuture<'a>>>>, // TODO figure out if we can remove that boxing
	}
}

impl<'a, F: Future> OnShutdownOr<'a, F> {
	pub(crate) fn new(should_stop: &'a ShouldStop, fut: F) -> Self {
		Self {
			should_stop,
			fut,
			ct_cancelled: None,
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
			let p = self.project();
			match p.fut.poll(cx) {
				Poll::Ready(v) => Poll::Ready(ShouldShutdownOr::ShouldNotShutdown(v)),
				Poll::Pending => {
					match p
						.ct_cancelled
						.get_or_insert_with(|| Box::pin(p.should_stop.cancellation_token.cancelled()))
						.as_mut()
						.poll(cx)
					{
						Poll::Ready(()) => Poll::Ready(ShouldShutdownOr::ShouldShutdown),
						Poll::Pending => Poll::Pending,
					}
				}
			}
		}
	}
}
