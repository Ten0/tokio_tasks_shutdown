use {
	anyhow::{anyhow, Error as InternalError},
	std::time::Duration,
	tokio::time::sleep,
};

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

#[tokio::test]
async fn error_causes_stop() {
	init_log();

	let start = std::time::Instant::now();
	let master = TasksBuilder::default().dont_catch_signals().build::<InternalError>();

	std::env::set_var("RUST_BACKTRACE", "0"); // For the display impl test below

	master
		.spawn("erroring_task", |_| async move {
			sleep(Duration::from_millis(10)).await;
			Err(anyhow!("aaa"))
		})
		.unwrap();
	master
		.spawn("not_erroring_task", |h| async move {
			h.on_shutdown().await;
			sleep(Duration::from_millis(10)).await;
			Ok(())
		})
		.unwrap();

	let mut v = Vec::new();
	master.join_all_with(|e| v.push(format!("{e:?}"))).await.unwrap_err();
	let elapsed = start.elapsed();

	assert_eq!(v, ["TaskError { task_name: \"erroring_task\", kind: UserError(aaa) }"]);

	dbg!(elapsed);
	assert!(elapsed > Duration::from_millis(19) && elapsed < Duration::from_millis(25));
}
