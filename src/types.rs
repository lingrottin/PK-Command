use crate::util::msg_id;

#[derive(PartialEq, Eq, Clone, Copy)]
pub enum Operation {
    SendVariable,    // SENDV
    RequireVariable, // REQUV
    Invoke,          // INVOK
    GetVersion,      // PKVER
    Start,           // START
    EndTransaction,  // ENDTR
    Acknowledge,     // ACKNO
    Query,           // QUERY
    Return,          // RTURN
    Empty,           // EMPTY
    Data,            // SDATA
    Await,           // AWAIT
    Error,           // ERROR
}

impl Operation {
    pub fn to_name(&self) -> &'static str {
        match self {
            Operation::SendVariable => "SENDV",
            Operation::RequireVariable => "REQUV",
            Operation::Invoke => "INVOK",
            Operation::GetVersion => "PKVER",
            Operation::Start => "START",
            Operation::EndTransaction => "ENDTR",
            Operation::Acknowledge => "ACKNO",
            Operation::Query => "QUERY",
            Operation::Return => "RTURN",
            Operation::Empty => "EMPTY",
            Operation::Data => "SDATA",
            Operation::Await => "AWAIT",
            Operation::Error => "ERROR",
        }
    }

    pub fn from_name(name: &str) -> Option<Operation> {
        match name {
            "SENDV" => Some(Operation::SendVariable),
            "REQUV" => Some(Operation::RequireVariable),
            "INVOK" => Some(Operation::Invoke),
            "PKVER" => Some(Operation::GetVersion),
            "START" => Some(Operation::Start),
            "ENDTR" => Some(Operation::EndTransaction),
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

    pub fn is_root(&self) -> bool {
        match self {
            Operation::SendVariable
            | Operation::RequireVariable
            | Operation::Invoke
            | Operation::GetVersion => true,
            _ => false,
        }
    }
}

#[derive(PartialEq, Eq, Clone, Copy)]
pub enum Role {
    Host,   // 调用方（不一定是主机）
    Device, // 接收方（不一定是设备）
    Idle,   // （空闲期没有角色）
}

#[derive(Clone)]
pub struct Command {
    pub msg_id: u16,
    pub operation: Operation,
    pub object: Option<String>,
    pub data: Option<Vec<u8>>,
}

impl Command {
    pub fn parse(msg_bytes: &[u8]) -> Result<Command, &'static str> {
        // 1. 检查最小长度
        if msg_bytes.len() < 7 {
            return Err("Invalid length: message is too short.");
        }

        // 2. 解析 MSG ID
        let msg_id_slice = msg_bytes.get(0..2).ok_or("Failed to slice MSG ID")?;

        // 3. 特殊处理 ERROR 指令
        if msg_id_slice == b"  " {
            // 检查 `ERROR ERROR` 或 `ACKNO ERROR` 结构
            let op_name_slice = msg_bytes.get(2..7);
            let space1_slice = msg_bytes.get(7..8);
            let object_slice = msg_bytes.get(8..13);

            let is_ackno_error = op_name_slice == Some(b"ACKNO")
                && space1_slice == Some(b" ")
                && object_slice == Some(b"ERROR");
            let is_error_error = op_name_slice == Some(b"ERROR")
                && space1_slice == Some(b" ")
                && object_slice == Some(b"ERROR");

            if !(is_ackno_error || is_error_error) {
                return Err("Invalid ERROR command format.");
            }

            let data = if msg_bytes.len() > 14 {
                // 检查数据前的空格
                if msg_bytes.get(13..14) != Some(b" ") {
                    return Err("Missing space before data in ERROR command.");
                }
                // unwrap is safe due to length check msg_bytes.len() > 14
                Some(msg_bytes.get(14..).unwrap().to_vec())
            } else if msg_bytes.len() == 13 {
                // Exactly "  OP_NAME OBJECT"
                None
            } else {
                return Err("Invalid length for ERROR command.");
            };

            return Ok(Command {
                msg_id: 0,
                operation: if op_name_slice == Some(b"ACKNO") {
                    Operation::Acknowledge
                } else {
                    Operation::Error
                },
                object: Some("ERROR".to_string()),
                data,
            });
        }

        // 4. 处理常规指令
        let msg_id_str =
            std::str::from_utf8(msg_id_slice).map_err(|_| "MSG ID is not valid UTF-8")?;
        let msg_id = msg_id::to_u16(msg_id_str).map_err(|_| "Invalid MSG ID format.")?;

        let op_name_slice = msg_bytes
            .get(2..7)
            .ok_or("Failed to slice operation name.")?;
        let op_name_str =
            std::str::from_utf8(op_name_slice).map_err(|_| "Operation name is not valid UTF-8")?;
        let operation = Operation::from_name(op_name_str).ok_or("Unrecognized operation name.")?;

        // 5. 根据长度和分隔符判断 object 和 data
        let (object, data) = match msg_bytes.len() {
            // 只有 MSG ID 和 OP NAME
            7 => (None, None),

            // 包含 OBJECT
            13 => {
                if msg_bytes.get(7..8) != Some(b" ") {
                    return Err("Missing space after operation name.");
                }
                let obj_slice = msg_bytes.get(8..13).ok_or("Failed to slice object.")?;
                let obj_str =
                    std::str::from_utf8(obj_slice).map_err(|_| "Object is not valid UTF-8")?;
                (Some(obj_str.to_string()), None)
            }

            // 包含 OBJECT 和 DATA
            len if len > 14 => {
                if msg_bytes.get(7..8) != Some(b" ") || msg_bytes.get(13..14) != Some(b" ") {
                    return Err("Missing space separator for object or data.");
                }
                let obj_slice = msg_bytes.get(8..13).ok_or("Failed to slice object.")?;
                let obj_str =
                    std::str::from_utf8(obj_slice).map_err(|_| "Object is not valid UTF-8")?;

                // unwrap is safe due to length check (len > 14)
                let data_slice = msg_bytes.get(14..).unwrap();
                (Some(obj_str.to_string()), Some(data_slice.to_vec()))
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

    pub fn to_bytes(&self) -> Vec<u8> {
        let id = match self.operation {
            Operation::Error => String::from("  "),
            // ERROR 指令的 ACKNO 的 id 也固定是两个空格
            Operation::Acknowledge => {
                if self.object == Some(String::from("ERROR")) {
                    String::from("  ")
                } else {
                    msg_id::from_u16(self.msg_id)
                        .map_err(|_| panic!("Invalid MSG ID"))
                        .unwrap()
                }
            }
            _ => msg_id::from_u16(self.msg_id)
                .map_err(|_| panic!("Invalid MSG ID"))
                .unwrap(),
        };
        if self.data.is_none() {
            if self.object.is_none() {
                format!("{}{}", id, self.operation.to_name())
                    .as_bytes()
                    .to_vec()
            } else {
                format!(
                    "{}{} {}",
                    id,
                    self.operation.to_name(),
                    self.object.clone().unwrap()
                )
                .as_bytes()
                .to_vec()
            }
        } else {
            let mut vec = format!(
                "{}{} {}",
                id,
                self.operation.to_name(),
                self.object.clone().unwrap()
            )
            .as_bytes()
            .to_vec();
            vec.push(20);
            vec.append(&mut self.data.clone().unwrap());
            vec
        }
    }
}

impl std::fmt::Display for Command {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let id = match self.operation {
            Operation::Error => String::from("  "),
            // ERROR 指令的 ACKNO 的 id 也固定是两个空格
            Operation::Acknowledge => {
                if self.object == Some(String::from("ERROR")) {
                    String::from("  ")
                } else {
                    msg_id::from_u16(self.msg_id).map_err(|_| std::fmt::Error)?
                }
            }
            _ => msg_id::from_u16(self.msg_id).map_err(|_| std::fmt::Error)?, // 将自定义错误转换为 fmt::Error
        };

        let op = self.operation.to_name();

        write!(f, "{}{}", id, op)?;

        if let Some(obj) = &self.object {
            write!(f, " {}", obj)?;
            if let Some(data_vec) = &self.data {
                // 对于 ERROR 命令, data 应该是 UTF-8 描述字符串.
                // 对于 SDATA 命令, data可能是任意二进制. String::from_utf8_lossy 在这里用于显示目的.
                // 如果需要严格的二进制到文本的转换 (例如 Base64), 应该在这里实现.
                let data_to_display = if self.operation == Operation::Error
                    || !data_vec.iter().any(|&b| b == 0 || b > 127)
                {
                    String::from_utf8_lossy(data_vec)
                } else {
                    String::from_utf8_lossy(data_vec) // 或者如: format!("<BINARY DATA: {} bytes>", data_vec.len())
                };
                write!(f, " {}", data_to_display)?;
            }
        }

        Ok(())
    }
}

// 指示当前收发指令方的特定状态
#[derive(PartialEq, Eq, Clone, Copy)]
pub enum Status {
    Other,          // 没有等待 ACK
    AwaitingAck,    // 等待 ACK
    AwaitingErrAck, // 等待 ACK（发送 ERROR 后）
}

// 指示当前“链”的状态（传输阶段）
#[derive(PartialEq, Eq, Clone, Copy)]
pub enum Stage {
    Idle,
    Started,               // 已发出/收到 START 指令
    RootOperationAssigned, // 已发出/收到根指令,等待发送参数
    SendingParameter,      // 正在传输参数
    ParameterSent,         // 已传输第一个“ENDTR”，等待 QUERY
    SendingResponse,       // 正在传输返回值
}
