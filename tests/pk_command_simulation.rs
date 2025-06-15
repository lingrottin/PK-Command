#[cfg(test)]
mod pk_command_integration_tests {
    use std::sync::mpsc::channel;
    use std::thread;
    use std::time::Duration;

    use pk_command::{PkCommand, PkCommandConfig, types::Operation as PkOperation};
    use pk_command::{PkMHashmapWrapper, PkPollable, PkVHashmapWrapper};

    const VARIA: &[u8] = b"variable value";
    const LONGV: &[u8] =b"(this is a very long string)Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat. Duis aute irure dolor in reprehenderit in voluptate velit esse cillum dolore eu fugiat nulla pariatur. Excepteur sint occaecat cupidatat non proident, sunt in culpa qui officia deserunt mollit anim id est laborum.";

    fn threads_simulation(
        operation: PkOperation,
        object: Option<String>,
        data: Option<Vec<u8>>,
        then: Box<dyn Fn(Vec<u8>) -> ()>,
    ) {
        let (host_tx, device_rx) = channel::<Vec<u8>>(); // Host -> Device
        let (device_tx, host_rx) = channel::<Vec<u8>>(); // Device -> Host

        let host_thread = thread::Builder::new()
            .name("HostThread".to_string())
            .spawn(move || {
                println!("[Host] Thread started");

                // In our test case, Host literally has no method or variable
                let var_accessor = PkVHashmapWrapper::new(vec![]);
                let method_accessor = PkMHashmapWrapper::new(vec![]);
                let host_pkc =
                    PkCommand::new(PkCommandConfig::default(64), var_accessor, method_accessor);

                host_pkc
                    .perform(operation, object.clone(), data)
                    .expect(&format!("Host failed to perform {:?}", operation));
                println!("[Host] Performed {:?} for {:?}", operation, object);

                let mut data: Vec<u8> = b"failed".into();
                for i in 0..10000 {
                    // 限制循环次数以防死锁/无限循环
                    if let Some(cmd_to_send) = host_pkc.poll() {
                        println!("[Host] Sending (iter {}): {}", i, cmd_to_send);
                        if host_tx.send(cmd_to_send.to_bytes()).is_err() {
                            break;
                        } // Device thread might have panicked
                    }

                    match host_rx.try_recv() {
                        Ok(received_bytes) => {
                            println!(
                                "[Host] Received {} bytes (iter {})",
                                received_bytes.len(),
                                i
                            );
                            if let Err(e) = host_pkc.incoming_command(received_bytes) {
                                println!("[Host] Error processing incoming command: {}", e);
                                break;
                            }
                        }
                        Err(std::sync::mpsc::TryRecvError::Empty) => { /* No message */ }
                        Err(_) => {
                            break;
                        } // Channel disconnected
                    }

                    if host_pkc.is_complete() {
                        println!("[Host] Transaction complete (iter {}).", i);
                        if let Some(ret_data) = host_pkc.get_return_data() {
                            println!(
                                "[Host] Got return data: {:?}",
                                String::from_utf8_lossy(&ret_data)
                            );
                            data = ret_data;
                        } else {
                            println!("[Host] No return data, but transaction complete.");
                            // 对于某些操作，没有返回数据也是成功的，取决于测试场景
                        }
                        break;
                    }
                    thread::sleep(Duration::from_millis(10)); // 给 Device 线程运行的机会
                }
                println!("[Host] Thread finished.",);
                data // 返回结果给主测试线程
            })
            .unwrap();

        // --- 从机线程 ---
        let device_thread = thread::Builder::new()
            .name("DeviceThread".to_string())
            .spawn(move || {
                println!("[Device] Thread started");
                let variable_listener = move |name: &'static str| {
                    return move |_: Vec<u8>| {
                        println!("[Variable Accessor] {} is changed", name);
                    };
                };
                let var_accessor = PkVHashmapWrapper::new(vec![
                    (
                        String::from("VARIA"),
                        Some(VARIA.to_vec()),
                        Box::new(variable_listener("VARIA")),
                    ),
                    (
                        String::from("LONGV"),
                        Some(LONGV.to_vec()),
                        Box::new(variable_listener("LONGV")),
                    ),
                ]);
                let method_accessor = PkMHashmapWrapper::new(vec![
                    (
                        String::from("ECHOO"),
                        Box::new(|param| {
                            PkPollable::execute(|resolve| {
                                resolve(param.unwrap_or(b"empty".to_vec()))
                            })
                        }),
                    ),
                    (
                        String::from("DEVID"),
                        Box::new(|_| {
                            PkPollable::execute(|resolve| resolve(b"device_123".to_vec()))
                        }),
                    ),
                    (
                        String::from("LONGO"),
                        Box::new(|_| {
                            PkPollable::execute(|resolve| {
                                thread::sleep(Duration::from_secs(2));
                                resolve(b"long_op_done".to_vec())
                            })
                        }),
                    ),
                ]);
                let device_pkc =
                    PkCommand::new(PkCommandConfig::default(64), var_accessor, method_accessor);

                for i in 0..10000 {
                    match device_rx.try_recv() {
                        Ok(received_bytes) => {
                            println!(
                                "[Device] Received {} bytes (iter {})",
                                received_bytes.len(),
                                i
                            );
                            if let Err(e) = device_pkc.incoming_command(received_bytes) {
                                println!("[Device] Error processing incoming command: {}", e);
                                break;
                            }
                        }
                        Err(std::sync::mpsc::TryRecvError::Empty) => { /* No message */ }
                        Err(_) => {
                            break;
                        } // Channel disconnected
                    }

                    if let Some(cmd_to_send) = device_pkc.poll() {
                        println!("[Device] Sending (iter {}): {}", i, cmd_to_send);
                        if device_tx.send(cmd_to_send.to_bytes()).is_err() {
                            break;
                        } // Host thread might have panicked
                    }

                    // 从机通常是被动方，它的 is_complete() 只是表示它完成了当前事务的它的部分
                    // 主机的 is_complete() 才是整个事务的结束标志
                    if device_pkc.is_complete() {
                        println!("[Device] Became idle and complete (iter {}).", i);
                        // Device might become idle before host fully processes the last ACK.
                        // The loop should continue to allow host to finish.
                    }
                    thread::sleep(Duration::from_millis(10));
                }
                println!("[Device] Thread finished");
            })
            .unwrap();

        let host_result = host_thread.join().expect("Host thread panicked");
        device_thread.join().expect("Device thread panicked");

        then(host_result);
    }

    #[test]
    fn test_requv_simulation() -> () {
        threads_simulation(
            PkOperation::RequireVariable,
            Some("VARIA".to_string()),
            None,
            Box::from(|data| assert_eq!(data, VARIA.to_vec(),)),
        );
    }

    #[test]
    fn test_long_requv_simulation() -> () {
        threads_simulation(
            PkOperation::RequireVariable,
            Some("LONGV".to_string()),
            None,
            Box::from(|data| assert_eq!(data, LONGV.to_vec(),)),
        );
    }

    #[test]
    fn test_sendv_simulation() -> () {
        threads_simulation(
            PkOperation::SendVariable,
            Some("VARIA".to_string()),
            Some(b"new value".to_vec()),
            Box::from(|data| {
                // SENDV 成功时，Host 收到的是 RTURN EMPTY 的 ACK，没有实际数据返回
                assert_eq!(data, b"failed"); // Host thread returns "failed" if no data is explicitly set
            }),
        );
    }

    #[test]
    fn test_long_sendv_simulation() -> () {
        threads_simulation(
            PkOperation::SendVariable,
            Some("LONGV".to_string()),
            Some(LONGV.to_vec()),
            Box::from(|data| {
                // SENDV 成功时，Host 收到的是 RTURN EMPTY 的 ACK，没有实际数据返回
                assert_eq!(data, b"failed"); // Host thread returns "failed" if no data is explicitly set
            }),
        );
    }

    #[test]
    fn test_invok_echo_simulation() -> () {
        threads_simulation(
            PkOperation::Invoke,
            Some("ECHOO".to_string()),
            Some(b"echo this back".to_vec()),
            Box::from(|data| assert_eq!(data, b"echo this back",)),
        );
    }

    #[test]
    fn test_invok_long_echo_simulation() -> () {
        threads_simulation(
            PkOperation::Invoke,
            Some("ECHOO".to_string()),
            Some(LONGV.to_vec()),
            Box::from(|data| assert_eq!(data, LONGV.to_vec(),)),
        );
    }

    #[test]
    fn test_invok_deviceid_simulation() -> () {
        threads_simulation(
            PkOperation::Invoke,
            Some("DEVID".to_string()),
            None, // No parameter for DEVID
            Box::from(|data| assert_eq!(data, b"device_123",)),
        );
    }

    #[test]
    fn test_invok_longop_simulation() -> () {
        threads_simulation(
            PkOperation::Invoke,
            Some(String::from("LONGO")),
            None,
            Box::from(|data| assert_eq!(data, b"long_op_done")),
        );
    }
}
