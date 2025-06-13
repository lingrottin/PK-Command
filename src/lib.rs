use std::cell::{Cell, RefCell};
use std::fmt;
use std::time::Instant;
mod util;

#[derive(PartialEq, Eq, Clone, Copy)]
enum Operation {
    SendVariable,    // SENDV
    RequireVariable, // REQUV
    Invoke,          // INVOK
    GetVersion,      // PKVER
    Start,           // START
    EndTransmission, // ENDTR
    Acknowledge,     // ACKNO
    Query,           // QUERY
    Return,          // RTURN
    Empty,           // EMPTY
    Data,            // SDATA
    Await,           // AWAIT
    Error,           // ERROR
}

impl Operation {
    fn to_name(&self) -> &'static str {
        match self {
            Operation::SendVariable => "SENDV",
            Operation::RequireVariable => "REQUV",
            Operation::Invoke => "INVOK",
            Operation::GetVersion => "PKVER",
            Operation::Start => "START",
            Operation::EndTransmission => "ENDTR",
            Operation::Acknowledge => "ACKNO",
            Operation::Query => "QUERY",
            Operation::Return => "RTURN",
            Operation::Empty => "EMPTY",
            Operation::Data => "SDATA",
            Operation::Await => "AWAIT",
            Operation::Error => "ERROR",
        }
    }

    fn from_name(name: &str) -> Option<Operation> {
        match name {
            "SENDV" => Some(Operation::SendVariable),
            "REQUV" => Some(Operation::RequireVariable),
            "INVOK" => Some(Operation::Invoke),
            "PKVER" => Some(Operation::GetVersion),
            "START" => Some(Operation::Start),
            "ENDTR" => Some(Operation::EndTransmission),
            "ACKNO" => Some(Operation::Acknowledge),
            "QUERY" => Some(Operation::Query),
            "RTURN" => Some(Operation::Return),
            "EMPTY" => Some(Operation::Empty),
            "SDATA" => Some(Operation::Data),
            "AWAIT" => Some(Operation::Await),
            "ERROR" => Some(Operation::Error),
            _ => None,
        }
    }
}

#[derive(Clone)]
pub struct Command {
    msg_id: u16,
    operation: Operation,
    object: Option<String>,
    data: Option<String>,
}

impl Command {
    fn parse(msg: &str) -> Result<Command, &'static str> {
        // 1. 检查最小长度
        if msg.len() < 7 {
            return Err("Invalid length: message is too short.");
        }

        // 2. 解析 MSG ID (使用 .get() 更安全)
        let msg_id_str = msg.get(0..2).ok_or("Failed to slice MSG ID")?;

        // 3. 特殊处理 ERROR 指令
        if msg_id_str == "  " {
            // 检查 `ERROR ERROR` 或 `ACKNO ERROR` 结构
            if (msg.get(2..7) != Some("ACKNO")
                || msg.get(7..8) != Some(" ")
                || msg.get(8..13) != Some("ERROR"))
                || (msg.get(2..7) != Some("ERROR")
                    || msg.get(7..8) != Some(" ")
                    || msg.get(8..13) != Some("ERROR"))
            {
                return Err("Invalid ERROR command format.");
            }

            let data = if msg.len() > 14 {
                // 检查数据前的空格
                if msg.get(13..14) != Some(" ") {
                    return Err("Missing space before data in ERROR command.");
                }
                Some(String::from(msg.get(14..).unwrap()))
            } else if msg.len() == 13 {
                None
            } else {
                return Err("Invalid length for ERROR command.");
            };

            return Ok(Command {
                msg_id: 0, // ERROR 指令的 ID 通常不使用，可以设为 0
                operation: if msg.get(2..7) == Some("ACKNO") {
                    Operation::Acknowledge
                } else {
                    Operation::Error
                },
                object: Some("ERROR".to_string()), // 根据协议，对象是 "ERROR"
                data,
            });
        }

        // 4. 处理常规指令
        let msg_id = util::msg_id::to_u16(msg_id_str).map_err(|_| "Invalid MSG ID format.")?;

        let op_name = msg.get(2..7).ok_or("Failed to slice operation name.")?;
        let operation = Operation::from_name(op_name).ok_or("Unrecognized operation name.")?;

        // 5. 根据长度和分隔符判断 object 和 data
        let (object, data) = match msg.len() {
            // 只有 MSG ID 和 OP NAME
            7 => (None, None),

            // 包含 OBJECT
            13 => {
                if msg.get(7..8) != Some(" ") {
                    return Err("Missing space after operation name.");
                }
                let obj_str = msg.get(8..13).ok_or("Failed to slice object.")?;
                (Some(String::from(obj_str)), None)
            }

            // 包含 OBJECT 和 DATA
            len if len > 14 => {
                if msg.get(7..8) != Some(" ") || msg.get(13..14) != Some(" ") {
                    return Err("Missing space separator for object or data.");
                }
                let obj_str = msg.get(8..13).ok_or("Failed to slice object.")?;
                let data_str = msg.get(14..).unwrap(); // unwrap is safe due to length check
                (Some(String::from(obj_str)), Some(String::from(data_str)))
            }

            // 其他所有长度都是无效的
            _ => return Err("Invalid message length."),
        };

        Ok(Command {
            msg_id,
            operation,
            object,
            data,
        })
    }
}

impl fmt::Display for Command {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // from_u16 应该返回 Result<String, _>
        // 这里我们假设它返回 Result<String, &str>
        let id = match self.operation {
            Operation::Error => String::from("  "),
            // ERROR 指令的 ACKNO 的 id 也固定是两个空格
            Operation::Acknowledge => {
                if self.object == Some(String::from("ERROR")) {
                    String::from("  ")
                } else {
                    util::msg_id::from_u16(self.msg_id).map_err(|_| fmt::Error)?
                }
            }
            _ => util::msg_id::from_u16(self.msg_id).map_err(|_| fmt::Error)?, // 将自定义错误转换为 fmt::Error
        };

        let op = self.operation.to_name();

        // write! 宏是实现 Display 的标准方式，它类似于 format!
        // 但将结果写入 Formatter
        write!(f, "{}{}", id, op)?;

        if let Some(obj) = &self.object {
            write!(f, " {}", obj)?;
            if let Some(data) = &self.data {
                write!(f, " {}", data)?;
            }
        }

        Ok(())
    }
}

// 指示当前收发指令方的特定状态
#[derive(PartialEq, Eq, Clone, Copy)]
enum Status {
    Idle,           // 空闲状态（不在链内）
    Active,         // 等待对方的指令
    AwaitingAck,    // 等待 ACK
    AwaitingErrAck, // 等待 ACK（发送 ERROR 后）
    ReceivingData,  // 正在接收数据
    SendingData,    // 正在发送数据
}

// 指示当前“链”的状态（传输阶段）
#[derive(PartialEq, Eq, Clone, Copy)]
enum Stage {
    Idle,
    Started,               // 已发出/收到 START 指令
    RootOperationAssigned, // 已发出/收到根指令,等待发送参数
    SendingParameter,      // 正在传输参数
    ParameterSent,         // 已传输第一个“ENDTR”，等待 QUERY
    SendingResponse,       // 正在传输返回值
}

impl Stage {
    fn from_int(int: u8) -> Stage {
        match int {
            0 => Self::Idle,
            1 => Self::Started,
            2 => Self::RootOperationAssigned,
            3 => Self::SendingParameter,
            4 => Self::ParameterSent,
            5 => Self::SendingResponse,
            _ => panic!("Out-of-bound integer equivalent for Stage"),
        }
    }

    fn to_int(&self) -> u8 {
        match self {
            Self::Idle => 0,
            Self::Started => 1,
            Self::RootOperationAssigned => 2,
            Self::SendingParameter => 3,
            Self::ParameterSent => 4,
            Self::SendingResponse => 5,
        }
    }
    fn increment(&mut self) -> () {
        *self = *self + 1_u8;
    }
}

impl std::ops::Add<u8> for Stage {
    type Output = Self;
    fn add(self, other: u8) -> Stage {
        let int = self.to_int();
        let ret = int + other;
        if int + other >= 6 {
            return Self::Idle;
        }
        Self::from_int(ret)
    }
}

pub struct PkCommandConfig {
    ack_timeout: u64,
    inter_command_timeout: u64,
    await_interval: u64,
}

#[derive(PartialEq, Eq, Clone, Copy)]
enum Role {
    Host,   // 调用方（不一定是主机）
    Device, // 接收方（不一定是设备）
    Idle,   // （空闲期没有角色）
}

pub struct PkCommand {
    stage: Cell<Stage>,
    status: Cell<Status>,
    role: Cell<Role>,
    last_sent_command: RefCell<Command>,
    last_sent_msg_id: Cell<u16>,
    data_buffer: Vec<u8>,
    root_operation: Cell<Operation>,
    root_object: RefCell<String>,
    command_buffer: RefCell<Command>, // 收到的指令
    command_processed: Cell<bool>,    // “收到的命令”是否已在上次 poll 处理完毕
    last_command_time: Cell<Instant>, // 上次收到/发出命令的时间
    config: PkCommandConfig,
}

impl PkCommand {
    pub fn incoming_command(&self, command: String) -> Result<(), &'static str> {
        match Command::parse(&*command) {
            Ok(parsed_command) => {
                self.command_buffer.replace(parsed_command);
                self.command_processed.set(false);
                self.last_command_time.replace(Instant::now());
                Ok(())
            }
            Err(e) => Err(e),
        }
    }

    pub fn poll(&self) -> Option<Command> {
        let send = move |command: Command| -> Option<Command> {
            self.last_command_time.set(Instant::now());
            self.last_sent_msg_id.set(command.msg_id);
            self.last_sent_command.replace(command.clone());
            Some(command)
        };
        let ack = move |msg_id: u16, operation: String| -> Option<Command> {
            self.last_command_time.set(Instant::now());
            Some(Command {
                msg_id: msg_id,
                operation: Operation::Acknowledge,
                object: Some(operation),
                data: None,
            })
        };
        let err = |msg: &'static str| -> Option<Command> {
            let command = Command {
                msg_id: 0,
                operation: Operation::Error,
                object: Some(String::from("ERROR")),
                data: Some(String::from(msg)),
            };
            self.status.set(Status::AwaitingErrAck);
            send(command)
        };
        // 首先检查是否有新的指令进入 command buffer
        match self.command_processed.get() {
            // 如果没有新的指令则检查超时
            true => {
                // Idle 则忽略当前 poll
                if self.status == Status::Idle.into() {
                    return None;
                }

                // 获取当前时间来比较超时
                let elapsed_ms = self.last_command_time.get().elapsed().as_millis();
                match self.status.get() {
                    Status::AwaitingAck | Status::AwaitingErrAck => {
                        // 等待 ACK 时则检查 ACK 超时来确认是否重传
                        if elapsed_ms >= self.config.ack_timeout.into() {
                            return Some(self.last_sent_command.borrow().clone());
                        }
                    }
                    _ => {
                        // 不考虑 Idle 因为上面已经检查过了,没有等待 ACK 时则检查指令间超时
                        if elapsed_ms >= self.config.inter_command_timeout.into() {
                            return err("Operation timed out");
                        }
                    }
                }
            }
            // 缓冲区内有新的指令
            false => {
                self.command_processed.set(true);
                let recv = self.command_buffer.borrow();
                if self.status.get() == Status::AwaitingErrAck {
                    // 首先处理不被 Stage 描述的特殊情况
                    if recv.operation == Operation::Error {
                        self.root_operation.set(Operation::Empty);
                        self.stage.set(Stage::Idle);
                        self.status.set(Status::Idle);
                        return None;
                    } else {
                    }
                }
                match self.stage.get() {
                    Stage::Idle => {
                        // 在 Idle 状态下只能收到 START
                        if recv.operation != Operation::Start {
                            return err("not in a chain");
                        }
                        self.stage.set(Stage::Started);
                        self.status.set(Status::Active);
                        return ack(recv.msg_id, recv.operation.to_name().to_string());
                    }

                    _ => {}
                }
            }
        }
        None
    }
}
