#![cfg(feature = "tokio-runtime-test")]
#![cfg(test)]

use pk_command::tokio_adapter::TokioFuturePollable;
use pk_command::{PkCommand, PkCommandConfig, PkHashmapMethod, PkHashmapVariable};
use std::time::Instant;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

#[tokio::test(flavor = "current_thread")]
async fn test_tokio_integration_simple_invoke() {
    let (host_tx, mut device_rx): (UnboundedSender<Vec<u8>>, UnboundedReceiver<Vec<u8>>) =
        unbounded_channel();
    let (device_tx, mut host_rx) = unbounded_channel();

    // Host side PK
    let var_accessor = PkHashmapVariable::new(vec![]);
    let method_accessor = PkHashmapMethod::new(vec![]);
    let host_pk = PkCommand::<_, _, Instant>::new(
        PkCommandConfig::default(64),
        var_accessor,
        method_accessor,
    );

    // Device side: method ECHO that uses TokioFuturePollable
    let method_impl = Box::new(move |_param: Option<Vec<u8>>| {
        TokioFuturePollable::from_future(async move {
            // Simulate async work
            Ok(Some(b"tokio-async".to_vec()))
        })
    });
    let device_methods = PkHashmapMethod::new(vec![(String::from("ECHOO"), method_impl)]);
    let var_accessor_dev = PkHashmapVariable::new(vec![]);
    let device_pk = PkCommand::<_, _, Instant>::new(
        PkCommandConfig::default(64),
        var_accessor_dev,
        device_methods,
    );

    // run a short loop: process messages exchanged over channels for both sides
    for _ in 0..1000 {
        // Host: poll and send
        if let Some(cmd) = host_pk.poll() {
            let _ = host_tx.send(cmd.to_bytes());
        }
        // Device: receive
        while let Ok(bytes) = device_rx.try_recv() {
            let _ = device_pk.incoming_command(bytes);
        }
        if let Some(cmd) = device_pk.poll() {
            let _ = device_tx.send(cmd.to_bytes());
        }
        // Host: receive
        while let Ok(bytes) = host_rx.try_recv() {
            let _ = host_pk.incoming_command(bytes);
        }
        // Early exit if host becomes idle
        if host_pk.is_complete() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(1)).await;
    }
}
