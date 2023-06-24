use {anyhow::Error as InternalError, std::time::Duration, tokio::time::sleep};

use tokio_tasks_shutdown::*;

fn init_log() {
	match simple_logger::SimpleLogger::new().init() {
		Ok(()) => {}
		Err(e) => {
			let _: log::SetLoggerError = e;
		}
	}
}

#[tokio::test]
async fn no_tasks() {
	init_log();

	let start = std::time::Instant::now();
	let master = TasksBuilder::default().dont_catch_signals().build::<InternalError>();
	let handle = master.handle();
	tokio::task::spawn(async move {
		sleep(Duration::from_millis(500)).await;
		handle.start_shutdown();
	});

	master.join_all_with(|e| panic!("{e}")).await.unwrap();
	let elapsed = start.elapsed();
	assert!(elapsed > Duration::from_millis(450) && elapsed < Duration::from_millis(550));
}
