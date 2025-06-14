const PK_VERSION: &'static str = "0.5";

use std::cell::{Cell, RefCell};
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

mod types;
mod util;
use types::{Command, Operation, Role, Stage, Status};

pub use util::{PkMHashmapWrapper, PkVHashmapWrapper};

/// Trait defining how to access (get/set) variables by their string key.
///
/// This allows the `PkCommand` state machine to be generic over the actual variable storage.
pub trait PkVariableAccessor {
    /// Retrieves the value of a variable.
    ///
    /// # Arguments
    /// * `key`: The name of the variable to retrieve.
    ///
    /// # Returns
    /// `Some(Vec<u8>)` containing the variable's data if found, or `None` otherwise.
    fn get(&self, key: String) -> Option<Vec<u8>>;

    /// Sets the value of a variable.
    ///
    /// # Arguments
    /// * `key`: The name of the variable to set.
    /// * `value`: The new data for the variable.
    ///
    /// # Returns
    /// `Ok(())` if successful, or an `Err(String)` describing the error.
    fn set(&self, key: String, value: Vec<u8>) -> Result<(), String>;
}

/// Trait defining how to invoke methods by their string key.
///
/// This allows the `PkCommand` state machine to be generic over the actual method implementation.
pub trait PkMethodAccessor {
    /// Calls a method with the given parameters.
    ///
    /// # Arguments
    /// * `key`: The name of the method to call.
    /// * `param`: The parameters for the method, as a byte vector.
    ///
    /// # Returns
    /// A `Result` containing a pinned, boxed Future that will resolve to the method's output
    /// (`Result<Option<Vec<u8>>, String>`), or an `Err(String)` if the method call cannot be initiated.
    fn call(
        &self,
        key: String,
        param: Vec<u8>,
    ) -> Result<Pin<Box<dyn Future<Output = Result<Option<Vec<u8>>, String>>>>, String>;
}

/// Configuration for the `PkCommand` state machine.
pub struct PkCommandConfig {
    /// Timeout duration for waiting for an `ACKNO` command.
    ack_timeout: Duration,
    /// Timeout duration for waiting for the next command in a sequence when not waiting for an `ACKNO`.
    inter_command_timeout: Duration,
    /// Interval at which the Device should send `AWAIT` commands during long-running operations.
    await_interval: Duration,
    /// The maximum length of a single command packet, including headers and data.
    packet_limit: u64,
    /// The version string of the PK protocol interpreter.
    pk_version: &'static str,
}

/// The main state machine for handling the PK Command protocol.
///
/// It manages transaction states, command parsing, command generation,
/// acknowledgments, timeouts, and data slicing.
pub struct PkCommand<VA, MA>
where
    VA: PkVariableAccessor,
    MA: PkMethodAccessor,
{
    stage: Cell<Stage>,
    status: Cell<Status>,
    role: Cell<Role>,
    last_sent_command: RefCell<Command>,
    last_sent_msg_id: Cell<u16>,
    last_received_msg_id: Cell<u16>,
    data_param: RefCell<Vec<u8>>,
    data_return: RefCell<Vec<u8>>,
    sending_data_progress: Cell<u64>,
    root_operation: Cell<Operation>,
    root_object: RefCell<Option<String>>,
    command_buffer: RefCell<Command>,
    command_processed: Cell<bool>,
    last_command_time: Cell<Instant>,
    device_op_pending: Cell<bool>,
    device_await_deadline: Cell<Option<Instant>>,
    config: PkCommandConfig,
    variable_accessor: VA,
    method_accessor: MA,
    pending_future: RefCell<Option<Pin<Box<dyn Future<Output = Result<Option<Vec<u8>>, String>>>>>>,
}

impl<VA: PkVariableAccessor, MA: PkMethodAccessor> PkCommand<VA, MA> {
    /// Ingests a raw command received from the other party.
    ///
    /// The command bytes are parsed, and if successful, the parsed `Command`
    /// is stored in an internal buffer to be processed by the next call to `poll()`.
    ///
    /// # Arguments
    /// * `command_bytes`: A `Vec<u8>` containing the raw bytes of the received command.
    ///
    /// # Returns
    /// `Ok(())` if the command was successfully parsed and buffered.
    /// `Err(&'static str)` if parsing failed.
    pub fn incoming_command(&self, command_bytes: Vec<u8>) -> Result<(), &'static str> {
        match Command::parse(&command_bytes) {
            // Pass as slice
            Ok(parsed_command) => {
                self.command_buffer.replace(parsed_command);
                self.command_processed.set(false);
                self.last_command_time.replace(Instant::now());
                Ok(())
            }
            Err(e) => Err(e),
        }
    }

    /// Slices a chunk of data from either `data_param` (for Host sending)
    /// or `data_return` (for Device sending).
    ///
    /// The size of the chunk is determined by `config.packet_limit` minus protocol overhead.
    /// Updates `sending_data_progress`.
    ///
    /// # Arguments
    /// * `role`: The current role of this `PkCommand` instance, determining which buffer to use.
    ///
    /// # Returns
    /// `Ok((Vec<u8>, bool))` where the `Vec<u8>` is the data chunk and the `bool` is `true`
    /// if this is the last chunk of data.
    /// `Err(&'static str)` if there's no data to send or if the role is `Idle`.
    fn slice_data(&self, role: Role) -> Result<(Vec<u8>, bool), &'static str> {
        // 如果 Role 是 Device 则默认在发送返回值，反之亦然
        match role {
            Role::Device => {
                let data = self.data_return.borrow();
                if data.len() == 0 {
                    return Err("No return data to slice.");
                }
                let start = self.sending_data_progress.get() as usize;
                let end =
                    std::cmp::min(start + (self.config.packet_limit - 14) as usize, data.len());
                let is_last_packet = end == data.len();
                self.sending_data_progress.set(end as u64);
                Ok((data[start..end].to_vec(), is_last_packet))
            }
            Role::Host => {
                let data = self.data_param.borrow();
                if data.len() == 0 {
                    return Err("No parameter data to slice.");
                }
                let start = self.sending_data_progress.get() as usize;
                let end =
                    std::cmp::min(start + (self.config.packet_limit - 14) as usize, data.len());
                let is_last_packet = end == data.len();
                self.sending_data_progress.set(end as u64);
                Ok((data[start..end].to_vec(), is_last_packet))
            }
            Role::Idle => Err("Cannot slice data in Idle role."),
        }
    }

    /// Polls the state machine for actions.
    ///
    /// This method should be called periodically. It processes incoming commands
    /// from the internal buffer (filled by `incoming_command`), handles timeouts,
    /// manages retransmissions, and progresses through the transaction stages.
    ///
    /// If the state machine determines that a command needs to be sent to the other party,
    /// this method will return `Some(Command)`.
    ///
    /// # Returns
    /// `Some(Command)` if a command needs to be sent, or `None` otherwise.
    pub fn poll(&self) -> Option<Command> {
        let next_msg_id_for_send = || util::msg_id::increment(self.last_received_msg_id.get());
        let send = move |command: Command| -> Option<Command> {
            self.last_command_time.set(Instant::now());
            self.last_sent_msg_id.set(command.msg_id);
            self.last_sent_command.replace(command.clone());
            // 因为 ACK 的函数并没有嵌套调用这个，所以
            self.status.set(Status::AwaitingAck);
            Some(command)
        };
        let reset_transaction_state = || {
            self.stage.set(Stage::Idle);
            self.status.set(Status::Other);
            self.role.set(Role::Idle);
            // Clear other relevant fields like root_operation, data_param, data_return, device_op_pending etc.
            self.data_param.borrow_mut().clear();
            self.data_return.borrow_mut().clear();
            self.sending_data_progress.set(0);
            self.device_op_pending.set(false);
            self.device_await_deadline.set(None);
        };
        let ack = move |msg_id: u16, operation: Operation| -> Option<Command> {
            self.last_command_time.set(Instant::now());
            Some(Command {
                msg_id: msg_id,
                operation: Operation::Acknowledge,
                object: Some(operation.to_name().to_string()),
                data: None,
            })
        };
        let err = |msg: &'static str| -> Option<Command> {
            // 在收到 ERROR 或 ACKNO ERROR 后，状态数据清零
            // 这个逻辑在下面处理 所以这里就不写了
            self.status.set(Status::AwaitingErrAck);
            let command = Command {
                msg_id: 0,
                operation: Operation::Error,
                object: Some(String::from("ERROR")),
                data: Some(msg.as_bytes().to_vec()),
            };
            self.last_command_time.set(Instant::now());
            self.last_sent_msg_id.set(command.msg_id);
            self.last_sent_command.replace(command.clone());
            Some(command)
        };
        // 首先检查是否有新的指令进入 command buffer
        match self.command_processed.get() {
            // 如果没有新的指令则检查超时
            true => {
                // Idle 则忽略当前 poll
                if self.stage.get() == Stage::Idle {
                    return None;
                }
                if self.stage.get() == Stage::Started
                    && self.role.get() == Role::Host
                    && self.status.get() != Status::AwaitingAck
                {
                    return send(Command {
                        msg_id: next_msg_id_for_send(),
                        operation: Operation::Start,
                        object: None,
                        data: None,
                    });
                }
                // Poll pending future if Device is in ParameterSent stage and an INVOK is pending
                if self.stage.get() == Stage::ParameterSent
                    && self.role.get() == Role::Device
                    && self.device_op_pending.get()
                        & (self.status.get() == Status::Other
                            || (self.status.get() != Status::AwaitingAck
                                && self.status.get() != Status::AwaitingErrAck))
                {
                    let waker = futures_task::noop_waker_ref();
                    let mut cx = Context::from_waker(waker);

                    let mut future_store = self.pending_future.borrow_mut();
                    if let Some(pinned_future) = future_store.as_mut() {
                        match pinned_future.as_mut().poll(&mut cx) {
                            Poll::Ready(result) => {
                                future_store.take(); // Remove completed future
                                self.device_op_pending.set(false);
                                self.device_await_deadline.set(None);

                                match result {
                                    Ok(data_opt) => {
                                        self.data_return.replace(data_opt.unwrap_or_default());
                                        self.stage.set(Stage::SendingResponse);
                                        // status will be set by send() to AwaitingAck
                                        self.sending_data_progress.set(0);

                                        let rturn_object_name = if self
                                            .data_return
                                            .borrow()
                                            .is_empty()
                                        {
                                            Operation::Empty.to_name().to_string()
                                        } else {
                                            // For INVOK, RTURN object is the method name
                                            self.root_object
                                                .borrow()
                                                .as_ref()
                                                .cloned()
                                                .unwrap_or_else(|| {
                                                    // Fallback, though root_object should be set for INVOK
                                                    self.root_operation.get().to_name().to_string()
                                                })
                                        };
                                        // RTURN itself doesn't carry data in its DATA field.
                                        // Data is sent via subsequent SDATA commands if rturn_object_name is not EMPTY.
                                        return send(Command {
                                            msg_id: next_msg_id_for_send(),
                                            operation: Operation::Return,
                                            object: Some(rturn_object_name),
                                            data: None,
                                        });
                                    }
                                    Err(_) => {
                                        // Future returned an error. Terminate transaction.
                                        reset_transaction_state();
                                        // log::error!("INVOK operation failed: {}", e_str); // Consider logging
                                        return err("INVOK operation failed"); // Send generic PK error
                                    }
                                }
                            }
                            Poll::Pending => {
                                if Instant::now()
                                    > self.device_await_deadline.get().unwrap_or(Instant::now())
                                {
                                    return send(Command {
                                        msg_id: next_msg_id_for_send(),
                                        operation: Operation::Await,
                                        object: None,
                                        data: None,
                                    });
                                }
                            }
                        }
                    } else {
                        return err("No pending future to poll");
                    }
                }

                // 获取当前时间来比较超时
                let elapsed_ms = self.last_command_time.get().elapsed();
                match self.status.get() {
                    Status::AwaitingAck | Status::AwaitingErrAck => {
                        // 等待 ACK 时则检查 ACK 超时来确认是否重传
                        if elapsed_ms >= self.config.ack_timeout {
                            return Some(self.last_sent_command.borrow().clone());
                        }
                    }
                    _ => {
                        // 不考虑 Idle 因为上面已经检查过了,没有等待 ACK 时则检查指令间超时
                        if elapsed_ms >= self.config.inter_command_timeout {
                            return err("Operation timed out");
                        }
                    }
                }
            }
            // 缓冲区内有新的指令
            false => {
                self.command_processed.set(true);
                self.last_received_msg_id
                    .set(self.command_buffer.borrow().msg_id); // Store received msg_id
                let recv = self.command_buffer.borrow();
                // 首先处理 Error 这种不被 Stage 描述的特殊情况
                if recv.operation == Operation::Error {
                    reset_transaction_state();
                    return ack(0, Operation::Error);
                } else {
                    if self.status.get() == Status::AwaitingErrAck {
                        if recv.operation == Operation::Acknowledge
                            && recv.object == Some(String::from("ERROR"))
                        {
                            self.status.set(Status::Other);
                            self.root_operation.set(Operation::Empty);
                            self.stage.set(Stage::Idle);
                            self.role.set(Role::Idle);
                            return None;
                        } else {
                            return err("Should be ACKNO ERROR");
                        }
                    }
                }
                match self.stage.get() {
                    Stage::Idle => {
                        // 在 Idle 状态下只能收到 START，且自身为 Device
                        if recv.operation != Operation::Start {
                            return err("not in a chain");
                        }
                        self.role.set(Role::Device);
                        self.stage.set(Stage::Started);
                        self.status.set(Status::Other); // Awaiting root command from Host
                        return ack(recv.msg_id, recv.operation);
                    }
                    Stage::Started => {
                        // 在 Started 状态下，根据当前角色不同，预期的行为应该是
                        // - Host -> 接收到 ACK，指示当前的根操作
                        // - Device -> 接收到根操作，进行 ACK
                        match self.role.get() {
                            Role::Host => {
                                if recv.operation == Operation::Acknowledge {
                                    self.status.set(Status::Other);
                                    self.stage.set(Stage::RootOperationAssigned);
                                    return send(Command {
                                        msg_id: next_msg_id_for_send(),
                                        operation: self.root_operation.get(),
                                        object: self.root_object.borrow().clone(),
                                        data: None,
                                    });
                                }
                            }
                            Role::Device => {
                                if recv.operation.is_root() {
                                    self.root_operation.set(recv.operation);
                                    // Validate if object is present for ops that require it
                                    if (recv.operation == Operation::RequireVariable
                                        || recv.operation == Operation::SendVariable
                                        || recv.operation == Operation::Invoke)
                                        && recv.object.is_none()
                                    {
                                        reset_transaction_state();
                                        return err(
                                            "Operation requires an object but none was provided.",
                                        );
                                    }
                                    self.root_object.replace(recv.object.clone());
                                    self.stage.set(Stage::RootOperationAssigned);
                                    return ack(recv.msg_id, recv.operation);
                                } else {
                                    return err("not a root operation");
                                }
                            }
                            _ => {
                                // 考虑代码问题，因为 Stage 已经是 Started 了，Role 不可能是 Idle
                                panic!("Role cannot be Idle if Stage is Started")
                            }
                        }
                    }
                    Stage::RootOperationAssigned => {
                        /* Host -> 接收到 ACK，**开始**传输数据。也就是说参数的*第一段*或 EMPTY 指令
                          Device -> 接收到 EMPTY 或数据的第一段
                        */
                        match self.role.get() {
                            Role::Host => {
                                if recv.operation == Operation::Acknowledge {
                                    self.status.set(Status::Other);
                                    self.stage.set(Stage::SendingParameter);
                                    if self.data_param.borrow().len() == 0 {
                                        return send(Command {
                                            msg_id: next_msg_id_for_send(),
                                            operation: Operation::Empty,
                                            object: None,
                                            data: None,
                                        });
                                    } else {
                                        match self.slice_data(Role::Host) {
                                            Ok((data_chunk, _is_last)) => {
                                                return send(Command {
                                                    msg_id: next_msg_id_for_send(),
                                                    operation: Operation::Data,
                                                    object: Some(
                                                        self.root_operation
                                                            .get()
                                                            .to_name()
                                                            .to_string(),
                                                    ),
                                                    data: Some(data_chunk),
                                                });
                                            }
                                            Err(e) => {
                                                reset_transaction_state();
                                                return err(e);
                                            }
                                        }
                                    }
                                } else {
                                    return err("Should be ACKNO");
                                }
                            }
                            Role::Device => {
                                if recv.operation == Operation::Empty {
                                    self.stage.set(Stage::SendingParameter);
                                    return ack(recv.msg_id, recv.operation);
                                } else if recv.operation == Operation::Data {
                                    self.stage.set(Stage::SendingParameter);
                                    self.data_param.borrow_mut().append(&mut Vec::from(
                                        recv.data.as_ref().unwrap().clone(),
                                    ));
                                    return ack(recv.msg_id, recv.operation);
                                } else {
                                    return err("Should be EMPTY or DATA");
                                }
                            }
                            _ => {
                                // 同上
                                panic!("Role cannot be Idle if Stage is RootOperationAssigned")
                            }
                        }
                    }
                    Stage::SendingParameter => {
                        // 此阶段：
                        // - Host: 已发送第一个参数数据包（SDATA）或 EMPTY，并收到 ACKNO。
                        //         现在需要判断是继续发送 SDATA 还是发送 ENDTR。
                        // - Device: 已收到第一个参数数据包（SDATA）或 EMPTY，并发送了 ACKNO。
                        //           现在等待接收后续的 SDATA 或 ENDTR。
                        match self.role.get() {
                            Role::Host => {
                                // Host 必须是收到了 ACKNO
                                if recv.operation != Operation::Acknowledge {
                                    return err("Host expected ACKNO in SendingParameter stage");
                                }
                                self.status.set(Status::Other); // ACK received, status is clear before sending next command

                                // 检查是对哪个指令的 ACKNO
                                match self.last_sent_command.borrow().operation {
                                    Operation::Empty => {
                                        // 对 EMPTY 的 ACKNO，参数传输结束，发送 ENDTR
                                        self.stage.set(Stage::ParameterSent);
                                        return send(Command {
                                            msg_id: next_msg_id_for_send(),
                                            operation: Operation::EndTransaction,
                                            object: None,
                                            data: None,
                                        });
                                    }
                                    Operation::Data => {
                                        // 对 SDATA 的 ACKNO
                                        let param_data_len = self.data_param.borrow().len() as u64;
                                        if self.sending_data_progress.get() < param_data_len {
                                            // 还有参数数据需要发送
                                            let (data_chunk, _is_last) =
                                                match self.slice_data(Role::Host) {
                                                    Ok(d) => d,
                                                    Err(e) => {
                                                        reset_transaction_state();
                                                        return err(e);
                                                    }
                                                };
                                            self.status.set(Status::AwaitingAck);
                                            return send(Command {
                                                msg_id: next_msg_id_for_send(),
                                                operation: Operation::Data,
                                                object: Some(
                                                    self.root_operation.get().to_name().to_string(),
                                                ),
                                                data: Some(data_chunk),
                                            });
                                        } else {
                                            // 参数数据已全部发送完毕，发送 ENDTR
                                            self.stage.set(Stage::ParameterSent);
                                            return send(Command {
                                                msg_id: next_msg_id_for_send(),
                                                operation: Operation::EndTransaction,
                                                object: None,
                                                data: None,
                                            });
                                        }
                                    }
                                    _ => {
                                        return err(
                                            "Host received ACKNO for unexpected command in SendingParameter stage",
                                        );
                                    }
                                }
                            }
                            Role::Device => {
                                // Device 等待 SDATA 或 ENDTR
                                if recv.operation == Operation::Data {
                                    if let Some(ref data_vec) = recv.data {
                                        self.data_param.borrow_mut().extend_from_slice(data_vec);
                                    }
                                    return ack(recv.msg_id, recv.operation);
                                } else if recv.operation == Operation::EndTransaction {
                                    self.stage.set(Stage::ParameterSent);
                                    return ack(recv.msg_id, recv.operation);
                                } else {
                                    return err(
                                        "Device expected DATA or ENDTR in SendingParameter stage",
                                    );
                                }
                            }
                            Role::Idle => {
                                panic!("Role cannot be Idle if Stage is SendingParameter")
                            }
                        }
                    }
                    Stage::ParameterSent => {
                        /* Host -> 收到对 ENDTR 的 ACK，发送 QUERY。等待回传数据或 AWAKE 保活。
                        Device -> 收到 QUERY，执行逻辑，处理保活和/或回传数据。 */
                        match self.role.get() {
                            Role::Host => match recv.operation {
                                Operation::Acknowledge => {
                                    self.status.set(Status::Other); // ACK received
                                    if recv.object == Some(String::from("ENDTR")) {
                                        return send(Command {
                                            msg_id: util::msg_id::increment(recv.msg_id),
                                            operation: Operation::Query,
                                            object: None,
                                            data: None,
                                        });
                                    } else if recv.object == Some(String::from("QUERY")) {
                                        return None;
                                    } else {
                                        return err(
                                            "Host: Unexpected ACK object in ParameterSent stage",
                                        );
                                    }
                                }
                                Operation::Await => {
                                    return ack(recv.msg_id, recv.operation);
                                }
                                Operation::Return => {
                                    if recv.object == Some(String::from("EMPTY"))
                                        || recv.object
                                            == Some(self.root_operation.get().to_name().to_string())
                                    {
                                        self.stage.set(Stage::SendingResponse);
                                        return ack(recv.msg_id, recv.operation);
                                    }
                                }
                                _ => {
                                    return err("Should be ACKNO, AWAIT or RETURN");
                                }
                            },
                            Role::Device => {
                                if recv.operation == Operation::Query {
                                    // 开始执行逻辑，然后 ACK
                                    match self.root_operation.get() {
                                        Operation::GetVersion => {
                                            self.data_return.replace(
                                                self.config.pk_version.as_bytes().to_vec(),
                                            );
                                            self.stage.set(Stage::SendingResponse);
                                        }
                                        Operation::RequireVariable => {
                                            let key = match self
                                                .root_object
                                                .borrow()
                                                .as_ref()
                                                .cloned()
                                            {
                                                Some(k) => k,
                                                None => {
                                                    // This check should ideally be when root_op was received
                                                    reset_transaction_state();
                                                    return err(
                                                        "Internal: Missing object name for REQUV.",
                                                    );
                                                }
                                            };
                                            self.data_return.replace(
                                                self.variable_accessor.get(key).unwrap_or(vec![]),
                                            );
                                            self.stage.set(Stage::SendingResponse);
                                        }
                                        Operation::SendVariable => {
                                            let key = match self
                                                .root_object
                                                .borrow()
                                                .as_ref()
                                                .cloned()
                                            {
                                                Some(k) => k,
                                                None => {
                                                    // This check should ideally be when root_op was received
                                                    reset_transaction_state();
                                                    return err(
                                                        "Internal: Missing object name for SENDV.",
                                                    );
                                                }
                                            };
                                            self.data_return.replace(
                                                if let Err(e) = self
                                                    .variable_accessor
                                                    .set(key, self.data_param.borrow().clone())
                                                {
                                                    e.as_bytes().to_vec()
                                                } else {
                                                    vec![]
                                                },
                                            );
                                            self.stage.set(Stage::SendingResponse); // Note: SENDV error reporting via data_return
                                        }
                                        Operation::Invoke => {
                                            self.device_op_pending.set(true);
                                            self.device_await_deadline.set(Some(
                                                Instant::now() + self.config.await_interval,
                                            ));
                                            // The object for INVOK is self.root_object, not from QUERY (recv.object)
                                            let method_name = match self
                                                .root_object
                                                .borrow()
                                                .as_ref()
                                                .cloned()
                                            {
                                                Some(name) => name,
                                                None => {
                                                    reset_transaction_state();
                                                    return err(
                                                        "Internal: Missing method name for INVOK",
                                                    );
                                                }
                                            };
                                            match self
                                                .method_accessor
                                                .call(method_name, self.data_param.borrow().clone())
                                            {
                                                Ok(future) => {
                                                    self.pending_future.replace(Some(future));
                                                }
                                                Err(_) => {
                                                    reset_transaction_state();
                                                    // log::error!("Failed to create INVOK future: {}", e_str);
                                                    return err(
                                                        "Failed to initiate INVOK operation",
                                                    );
                                                }
                                            }
                                        }
                                        _ => {
                                            reset_transaction_state();
                                            return err("Not a root operation");
                                        }
                                    }
                                    return ack(recv.msg_id, recv.operation);
                                }
                            }
                            Role::Idle => {
                                panic!("Role cannot be Idle if Stage is ParameterSent")
                            }
                        }
                    }
                    Stage::SendingResponse => {
                        /* Host -> 收到对 RETURN 的 ACK，开始接收数据。
                        Device -> 收到对 QUERY 的 ACK，发送 RETURN。 */
                        match self.role.get() {
                            Role::Host => {
                                // Host 等待 SDATA 或 ENDTR
                                if recv.operation == Operation::Data {
                                    // Host receives SDATA from Device
                                    if let Some(ref data_vec) = recv.data {
                                        self.data_return.borrow_mut().extend_from_slice(data_vec);
                                    }
                                    return ack(recv.msg_id, recv.operation);
                                } else if recv.operation == Operation::EndTransaction {
                                    let endtr_ack = ack(recv.msg_id, recv.operation);
                                    // 收到 ENDTR，事务结束
                                    reset_transaction_state();
                                    return endtr_ack;
                                } else {
                                    return err(
                                        "Host expected DATA or ENDTR in SendingResponse stage",
                                    );
                                }
                            }
                            Role::Device => {
                                // Device 必须是收到了 ACKNO
                                if recv.operation != Operation::Acknowledge {
                                    return err("Device expected ACKNO in SendingResponse stage");
                                }
                                self.status.set(Status::Other); // ACK received, status is clear before sending next command

                                // 检查是对哪个指令的 ACKNO
                                match self.last_sent_command.borrow().operation {
                                    Operation::Return => {
                                        // 对 RETURN 的 ACKNO
                                        let return_data_len =
                                            self.data_return.borrow().len() as u64;
                                        if return_data_len == 0 {
                                            // 没有返回值，直接发送 ENDTR
                                            self.stage.set(Stage::Idle); // Transaction ends
                                            return send(Command {
                                                msg_id: next_msg_id_for_send(),
                                                operation: Operation::EndTransaction,
                                                object: None,
                                                data: None,
                                            });
                                        } else {
                                            // 有返回值
                                            let (data_chunk, _) =
                                                match self.slice_data(Role::Device) {
                                                    Ok(d) => d,
                                                    Err(e) => {
                                                        reset_transaction_state();
                                                        return err(e);
                                                    }
                                                };

                                            return send(Command {
                                                msg_id: next_msg_id_for_send(),
                                                operation: Operation::Data,
                                                object: Some(
                                                    self.root_operation.get().to_name().to_string(),
                                                ),
                                                data: Some(data_chunk),
                                            });
                                        }
                                    }
                                    Operation::Data => {
                                        if self.sending_data_progress.get()
                                            < self.data_return.borrow().len() as u64
                                        {
                                            let (data_chunk, _) =
                                                match self.slice_data(Role::Device) {
                                                    Ok(d) => d,
                                                    Err(e) => {
                                                        reset_transaction_state();
                                                        return err(e);
                                                    }
                                                };
                                            return send(Command {
                                                msg_id: next_msg_id_for_send(),
                                                operation: Operation::Data,
                                                object: Some(
                                                    self.root_operation.get().to_name().to_string(),
                                                ),
                                                data: Some(data_chunk),
                                            });
                                        } else {
                                            return send(Command {
                                                msg_id: next_msg_id_for_send(),
                                                operation: Operation::EndTransaction,
                                                object: None,
                                                data: None,
                                            });
                                        }
                                    }
                                    Operation::EndTransaction => {
                                        self.role.set(Role::Idle);
                                        self.stage.set(Stage::Idle);
                                        // reset_transaction_state();
                                        // 结束后不清理数据，考虑到外部可能手动获取，这里就不操心了
                                        return None;
                                    }
                                    Operation::Await => {
                                        // Device received ACKNO AWAIT
                                        // self.status is Other. Device continues pending op.
                                        return None;
                                    }
                                    _ => {
                                        return err(
                                            "Device received ACKNO for unexpected command in SendingResponse stage",
                                        );
                                    }
                                }
                            }
                            _ => {
                                panic!("Role cannot be Idle if Stage is SendingResponse")
                            }
                        }
                    }
                }
            }
        }
        None
    }

    /// Initiates a new root operation from the Host side.
    ///
    /// This method should only be called when the `PkCommand` instance is in an `Idle` state.
    /// It sets up the necessary internal state to begin a new transaction.
    /// The actual `START` command and subsequent root operation command will be generated
    /// by subsequent calls to `poll()`.
    ///
    /// # Arguments
    /// * `operation`: The root `Operation` to perform (e.g., `SENDV`, `REQUV`, `INVOK`, `PKVER`).
    /// * `object`: An optional `String` representing the object of the operation (e.g., variable name, method name).
    /// * `data`: Optional `Vec<u8>` containing parameter data for the operation (e.g., for `SENDV` or `INVOK`).
    ///
    /// # Returns
    /// `Ok(())` if the operation can be initiated, or `Err(&'static str)` if not (e.g., not idle, or not a root operation).
    pub fn perform(
        &self,
        operation: Operation,
        object: Option<String>,
        data: Option<Vec<u8>>,
    ) -> Result<(), &'static str> {
        if operation.is_root()
            && self.stage.get() == Stage::Idle
            && self.status.get() == Status::Other
            && self.role.get() == Role::Idle
        {
            self.root_operation.set(operation);
            self.root_object.replace(object);
            self.data_param.replace(data.unwrap_or(vec![]));
            self.role.set(Role::Host);
            self.stage.set(Stage::Started);
            self.status.set(Status::Other);
            Ok(())
        } else if !operation.is_root() {
            Err("Cannot initiate a non-root operation")
        } else {
            Err("Cannot initiate an operation when the transaction is in progress")
        }
    }

    fn reset_transaction_state(&self) -> () {
        self.stage.set(Stage::Idle);
        self.status.set(Status::Other);
        self.role.set(Role::Idle);
        // Clear other relevant fields like root_operation, data_param, data_return, device_op_pending etc.
        self.data_param.borrow_mut().clear();
        self.data_return.borrow_mut().clear();
        self.sending_data_progress.set(0);
        self.device_op_pending.set(false);
        self.device_await_deadline.set(None);
    }

    /// Checks if the transaction is complete (i.e., the state machine is in the `Idle` stage).
    ///
    /// # Returns
    /// `true` if the transaction is complete, `false` otherwise.
    pub fn is_complete(&self) -> bool {
        self.stage.get() == Stage::Idle
    }

    /// Retrieves the return data from the completed transaction.
    ///
    /// This method should only be called when `is_complete()` returns `true` and the
    /// instance is acting as the Host.
    ///
    /// # Returns
    /// `Some(Vec<u8>)` containing the return data if available, or `None` if there was no return data.
    pub fn get_return_data(&self) -> Option<Vec<u8>> {
        if self.stage.get() == Stage::Idle && self.role.get() == Role::Host {
            let data = self.data_return.borrow().clone();
            self.reset_transaction_state();
            if data.is_empty() {
                None
            } else {
                Some(data.clone())
            }
        } else {
            None // Not in a state to provide return data
        }
    }

    /// Waits for the transaction to complete and then executes a callback with the return data.
    ///
    /// This is a blocking or polling-based wait depending on how the surrounding code
    /// calls `poll()`. The callback is only executed once the state machine enters the `Idle` stage.
    ///
    /// # Arguments
    /// * `callback`: A closure that takes an `Option<Vec<u8>>` (the return data) and is executed upon completion.
    ///
    /// # Note
    /// This method assumes `poll()` is being called externally to drive the state machine.
    /// It does not block the current thread waiting for completion, but rather checks the state
    /// and executes the callback if complete. You must ensure `poll()` is called frequently
    /// for the transaction to progress.
    pub fn wait_for_complete_and<F>(&self, callback: F) -> ()
    where
        F: FnOnce(Option<Vec<u8>>) -> (),
    {
        // 这个函数也是轮询的，用来给 Host 方返回值（因为在上面的 perform 中并没有告诉 PK 该怎么处理返回值）
        if self.stage.get() == Stage::Idle {
            let data = self.data_return.borrow().clone();
            self.reset_transaction_state();
            callback(if data.len() == 0 { None } else { Some(data) })
        }
    }

    /// Creates a new `PkCommand` state machine instance.
    ///
    /// # Arguments
    /// * `config`: The `PkCommandConfig` to use.
    /// * `variable_accessor`: An implementation of `PkVariableAccessor` for variable operations.
    /// * `method_accessor`: An implementation of `PkMethodAccessor` for method invocation.
    ///
    pub fn new(config: PkCommandConfig, variable_accessor: VA, method_accessor: MA) -> Self {
        PkCommand {
            stage: Cell::new(Stage::Idle),
            status: Cell::new(Status::Other),
            role: Cell::new(Role::Idle),
            last_sent_command: RefCell::new(Command {
                msg_id: 0,
                operation: Operation::Empty,
                object: None,
                data: None,
            }),
            last_sent_msg_id: Cell::new(0),
            last_received_msg_id: Cell::new(0),
            data_param: RefCell::new(vec![]),
            data_return: RefCell::new(vec![]),
            sending_data_progress: Cell::new(0),
            root_operation: Cell::new(Operation::Empty),
            root_object: RefCell::new(None),
            command_buffer: RefCell::new(Command {
                msg_id: 0,
                operation: Operation::Empty,
                object: None,
                data: None,
            }),
            command_processed: Cell::new(true),
            last_command_time: Cell::new(Instant::now()),
            device_op_pending: Cell::new(false),
            device_await_deadline: Cell::new(None),
            config,
            variable_accessor,
            method_accessor,
            pending_future: RefCell::new(None),
        }
    }
}
impl PkCommandConfig {
    /// Creates a `PkCommandConfig` with default timeout values.
    ///
    /// # Arguments
    /// * `packet_limit`: The maximum packet size allowed by the transport layer.
    ///
    pub fn default(packet_limit: u64) -> Self {
        PkCommandConfig {
            ack_timeout: Duration::from_millis(100),
            inter_command_timeout: Duration::from_millis(500),
            await_interval: Duration::from_millis(300),
            packet_limit,
            pk_version: PK_VERSION,
        }
    }

    /// Creates a new `PkCommandConfig` with specified values.
    ///
    /// # Arguments
    /// * `ack_timeout`: ACK timeout in milliseconds.
    /// * `inter_command_timeout`: Inter-command timeout in milliseconds.
    /// * `await_interval`: AWAIT interval in milliseconds.
    /// * `packet_limit`: The maximum packet size allowed by the transport layer.
    ///
    pub fn new(
        ack_timeout: u64,
        inter_command_timeout: u64,
        await_interval: u64,
        packet_limit: u64,
    ) -> Self {
        PkCommandConfig {
            ack_timeout: Duration::from_millis(ack_timeout),
            inter_command_timeout: Duration::from_millis(inter_command_timeout),
            await_interval: Duration::from_millis(await_interval),
            packet_limit, // Default packet limit if not specified
            pk_version: PK_VERSION,
        }
    }
}
