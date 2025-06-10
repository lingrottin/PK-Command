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
struct Command {
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
            // 检查 `ERROR ERROR` 结构
            if msg.get(2..7) != Some("ERROR")
                || msg.get(7..8) != Some(" ")
                || msg.get(8..13) != Some("ERROR")
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
                operation: Operation::Error,
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

#[derive(PartialEq, Eq, Clone, Copy)]
enum Status {
    Idle,           // 空闲状态（不在链内）
    Active,         // 已经开始链，等待对方的指令
    AwaitingAck,    // 等待 ACK
    AwaitingErrAck, // 等待 ACK（发送 ERROR 后）
    ReceivingData,  // 正在接收数据
    SendingData,    // 正在发送数据
    AwaitingQuery,  // 作为从机正在等待主机的 QUERY 指令
    AwaitingParam,  // 作为从机正在等待主机的 SDATA/EMPTY 指令
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
        match self.command_processed.get() {
            true => {
                if self.status == Status::Idle.into() {
                    return None;
                }
                let elapsed = self.last_command_time.get().elapsed();
                match self.status.get() {
                    Status::AwaitingAck => {
                        if elapsed.as_millis() >= self.config.ack_timeout.into() {
                            return Some(self.last_sent_command.borrow().clone());
                        }
                    }
                    _ => {
                        // 不考虑 Idle 因为上面已经检查过了
                        if elapsed.as_millis() >= self.config.inter_command_timeout.into() {
                            let command = Command {
                                msg_id: 0,
                                operation: Operation::Error,
                                object: Some(String::from("ERROR")),
                                data: Some(String::from("Operation timed out")),
                            };
                            self.status.set(Status::AwaitingErrAck);
                            self.last_sent_command.replace(command.clone());
                            self.last_sent_msg_id
                                .set(util::msg_id::increment(self.last_sent_msg_id.get()));
                            self.last_command_time.replace(Instant::now());
                            return Some(command);
                        }
                    }
                }
            }
            false => {}
        }
        None
    }
}
