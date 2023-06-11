use {anyhow::Error as InternalError, log::info, std::time::Duration, tokio::time};

use async_systems_shutdown::*;

fn init_log() {
	match simple_logger::SimpleLogger::new().init() {
		Ok(()) => {}
		Err(e) => {
			let _: log::SetLoggerError = e;
		}
	}
}

#[tokio::test]
async fn simple() {
	init_log();

	let start = std::time::Instant::now();
	let master = SystemsMasterBuilder::default()
		.dont_catch_signals()
		.timeout(Duration::from_secs(2))
		.build::<InternalError>();
	let handle = master.handle();
	tokio::task::spawn(async move {
		time::sleep(Duration::from_millis(500)).await;
		handle.start_shutdown();
	});
	master.spawn("S1", |handle| async move {
		info!("Started task");
		time::sleep(Duration::from_secs(1)).await;
		info!("Slept");
		handle.on_shutdown().await;
		info!("Got shutdown msg");
		Ok(())
	});
	master.with_errors(|e| panic!("{e}")).await.unwrap();
	let elapsed = start.elapsed();
	assert!(elapsed < Duration::from_millis(1100) && elapsed > Duration::from_millis(900));
}
