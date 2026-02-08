#![cfg(all(feature = "embassy", feature = "embassy-runtime-test"))]
#![cfg(test)]

use embassy_executor::Executor;
use embassy_time::Timer;
use pk_command::embassy_adapter::TaskCallback;
use pk_command::embassy_method_accessor;
use pk_command::types::Operation as PkOperation;
use pk_command::{EmbassyInstant, PkCommand, PkCommandConfig, PkHashmapMethod, PkHashmapVariable};
use static_cell::StaticCell;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread;
use std::time::Duration;

#[embassy_executor::task]
async fn async_echo(param: Vec<u8>, callback: TaskCallback) {
    Timer::after_secs(1).await;
    callback(param);
}

#[embassy_executor::task]
async fn async_some(param: Vec<u8>, callback: TaskCallback) {
    Timer::after_secs(3).await;
    callback(param);
}

// this is required for the macro
extern crate alloc;

embassy_method_accessor!(
    TestMethodAccessor,
    ("ECHOO", async_echo),
    ("3SECS", async_some)
);

// embassy require Executor to be 'static, so we use StaticCell to hold it.
static EXECUTOR: StaticCell<Executor> = StaticCell::new();

#[embassy_executor::task]
async fn poll(rx: Receiver<Vec<u8>>, tx: Sender<Vec<u8>>, method_accessor: TestMethodAccessor) {
    let var_accessor = PkHashmapVariable::new(vec![]);
    let device_pkc = PkCommand::<_, _, EmbassyInstant>::new(
        PkCommandConfig::default(64),
        var_accessor,
        method_accessor,
    );
    loop {
        match rx.try_recv() {
            Ok(received_bytes) => {
                let _ = device_pkc.incoming_command(received_bytes);
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                panic!("device thread stopped");
            }
        }

        if let Some(cmd_to_send) = device_pkc.poll() {
            let _ = tx.send(cmd_to_send.to_bytes());
        }

        if device_pkc.is_complete() {
            // embassy_executor::Executor::run never returns,
            // so we use panic! to stop the device thread forcedly when command is complete.
            panic!("Command completed, stopping device thread");
        }
        Timer::after_millis(5).await;
    }
}

#[test]
fn test_embassy_invok_echo() {
    let (host_tx, device_rx) = channel::<Vec<u8>>();
    let (device_tx, host_rx) = channel::<Vec<u8>>();
    let input_data = b"embassy echo".to_vec();

    let host_thread = thread::spawn({
        let host_tx = host_tx.clone();
        let host_rx = host_rx;
        let input_data = input_data.clone();
        move || {
            let var_accessor = PkHashmapVariable::new(vec![]);
            let method_accessor = PkHashmapMethod::new(vec![]);
            let host_pkc = PkCommand::<_, _, std::time::Instant>::new(
                PkCommandConfig::default(64),
                var_accessor,
                method_accessor,
            );

            host_pkc
                .perform(
                    PkOperation::Invoke,
                    Some("ECHOO".to_string()),
                    Some(input_data.clone()),
                )
                .expect("Host failed to perform INVOK");

            let mut data: Vec<u8> = Vec::new();
            for _ in 0..10000 {
                if let Some(cmd_to_send) = host_pkc.poll() {
                    if host_tx.send(cmd_to_send.to_bytes()).is_err() {
                        break;
                    }
                }

                match host_rx.try_recv() {
                    Ok(received_bytes) => {
                        let _ = host_pkc.incoming_command(received_bytes);
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => {}
                    Err(_) => break,
                }

                if host_pkc.is_complete() {
                    if let Some(ret_data) = host_pkc.get_return_data() {
                        data = ret_data;
                    }
                    break;
                }
                thread::sleep(Duration::from_millis(10));
            }
            data
        }
    });

    let device_thread = thread::spawn(move || {
        let executor = EXECUTOR.init(Executor::new());
        executor.run(|spawner| {
            let spawner_send = spawner.make_send();
            let ma = TestMethodAccessor::new(spawner_send);
            spawner.spawn(poll(device_rx, device_tx, ma)).unwrap();
        });
    });

    let host_result = host_thread.join().expect("Host thread panicked");
    // Because we stopped the device thread via panic!, so we don't expect it to join successfully.
    let _ = device_thread.join();
    assert_eq!(host_result, input_data);
}
