#![cfg(feature = "smol-runtime")]
#![cfg(test)]

use async_channel::{Receiver, Sender, unbounded};
use pk_command::smol_adapter::SmolFuturePollable;
use pk_command::{PkCommand, PkCommandConfig, PkHashmapMethod, PkHashmapVariable};
use std::time::Instant;

#[test]
fn test_smol_integration_simple_invoke() {
    smol::block_on(async {
        let (host_tx, device_rx): (Sender<Vec<u8>>, Receiver<Vec<u8>>) = unbounded();
        let (device_tx, host_rx) = unbounded();

        // Host side PK
        let var_accessor = PkHashmapVariable::new(vec![]);
        let method_accessor = PkHashmapMethod::new(vec![]);
        let host_pk = PkCommand::<_, _, Instant>::new(
            PkCommandConfig::default(64),
            var_accessor,
            method_accessor,
        );

        // Device: method ECHOO -> smol adapter
        let method_impl = Box::new(move |_param: Option<Vec<u8>>| {
            SmolFuturePollable::from_future(async move { Ok(Some(b"smol-async".to_vec())) })
        });
        let device_methods = PkHashmapMethod::new(vec![(String::from("ECHOO"), method_impl)]);
        let var_accessor_dev = PkHashmapVariable::new(vec![]);
        let device_pk = PkCommand::<_, _, Instant>::new(
            PkCommandConfig::default(64),
            var_accessor_dev,
            device_methods,
        );

        for _ in 0..1000 {
            // Host: poll and send
            if let Some(cmd) = host_pk.poll() {
                let _ = host_tx.send(cmd.to_bytes()).await;
            }
            // Device: receive
            while let Ok(bytes) = device_rx.try_recv() {
                let _ = device_pk.incoming_command(bytes);
            }
            if let Some(cmd) = device_pk.poll() {
                let _ = device_tx.send(cmd.to_bytes()).await;
            }
            while let Ok(bytes) = host_rx.try_recv() {
                let _ = host_pk.incoming_command(bytes);
            }
            if host_pk.is_complete() {
                break;
            }
            smol::Timer::after(std::time::Duration::from_millis(1)).await;
        }
    })
}
