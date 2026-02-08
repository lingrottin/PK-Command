//!
//! This module defines the core data structures and types used in the PK Command protocol implementation.

#[cfg(not(feature = "std"))]
extern crate alloc;
#[cfg(not(feature = "std"))]
extern crate core as std;
#[cfg(not(feature = "std"))]
use alloc::{
    format,
    string::{String, ToString},
    vec::Vec,
};

use crate::util::msg_id;

/// Defines the set of operations supported by the PK Command protocol.
#[derive(PartialEq, Eq, Clone, Copy, Debug)]
pub enum Operation {
    /// To set a variable's value on the device.
    ///
    /// 5-character name: `SENDV`
    SendVariable,

    /// To get a variable's value from the device.
    ///
    /// 5-character name: `REQUV`
    RequireVariable,

    /// To invoke a method on the device.
    ///
    /// 5-character name: `INVOK`
    Invoke,

    /// To get the version of the PK Command processor on the device.
    ///
    /// 5-character name: `PKVER`
    GetVersion,

    /// To indicate the start of a transaction chain.
    ///
    /// This is used internally by the [`poll`](crate::PkCommand::poll) method to manage transaction stages
    /// and usually should not be used directly.
    ///
    /// 5-character name: `START`
    Start,

    /// To indicate the end of a transaction chain.
    ///
    /// This is used internally by the [`poll`](crate::PkCommand::poll) method to manage transaction stages
    /// and usually should not be used directly.
    ///
    /// 5-character name: `ENDTR`
    EndTransaction,

    /// To acknowledge the receipt of a command.
    ///
    /// This is used internally by the state machine to manage acknowledgment status and usually should not be used directly.
    ///
    /// 5-character name: `ACKNO`
    Acknowledge,

    /// To request the outbound data from the device.
    ///
    /// This is used internally by the state machine to manage transaction stages and usually should not be used directly.
    ///
    /// 5-character name: `QUERY`
    Query,

    /// To return the response data from the device to the host.
    ///
    /// This is used internally by the state machine to manage transaction stages and usually should not be used directly.
    ///
    /// 5-character name: `RTURN`
    Return,

    /// To indicate that the current transaction phase has no data.
    ///
    /// This is used internally by the state machine to manage transaction stages and usually should not be used directly.
    ///
    /// 5-character name: `EMPTY`
    Empty,

    /// To send a chunk of data.
    ///
    /// This is used internally by the state machine to manage transaction stages and usually should not be used directly.
    ///
    /// 5-character name: `SDATA`
    Data,

    /// To indicate that the device is still processing and the transaction should be kept alive.
    ///
    /// This is used internally by the state machine to manage transaction stages and usually should not be used directly.
    ///
    /// 5-character name: `AWAIT`
    Await,

    /// To indicate an error occurred during transaction processing.
    ///
    /// This is used internally by the state machine to manage error handling and usually should not be used directly.
    ///
    /// 5-character name: `ERROR`
    Error,
}

impl Operation {
    /// Returns the 5-character string representation of the operation.
    pub fn to_name(&self) -> &'static str {
        use Operation::*;
        match self {
            SendVariable => "SENDV",
            RequireVariable => "REQUV",
            Invoke => "INVOK",
            GetVersion => "PKVER",
            Start => "START",
            EndTransaction => "ENDTR",
            Acknowledge => "ACKNO",
            Query => "QUERY",
            Return => "RTURN",
            Empty => "EMPTY",
            Data => "SDATA",
            Await => "AWAIT",
            Error => "ERROR",
        }
    }

    /// Creates an `Operation` from its 5-character string representation.
    pub fn from_name(name: &str) -> Option<Operation> {
        use Operation::*;
        match name {
            "SENDV" => Some(SendVariable),
            "REQUV" => Some(RequireVariable),
            "INVOK" => Some(Invoke),
            "PKVER" => Some(GetVersion),
            "START" => Some(Start),
            "ENDTR" => Some(EndTransaction),
            "ACKNO" => Some(Acknowledge),
            "QUERY" => Some(Query),
            "RTURN" => Some(Return),
            "EMPTY" => Some(Empty),
            "SDATA" => Some(Data),
            "AWAIT" => Some(Await),
            "ERROR" => Some(Error),
            _ => None,
        }
    }

    /// Checks if the operation is a "root operation" that can initiate a transaction chain.
    pub fn is_root(&self) -> bool {
        matches!(
            self,
            Operation::SendVariable
                | Operation::RequireVariable
                | Operation::Invoke
                | Operation::GetVersion
        )
    }
}

/// Defines the role of a participant in a PK Command transaction.
///
/// This is used internally by the state machine to manage transaction flow and usually should not be used directly.
#[derive(PartialEq, Eq, Clone, Copy)]
pub enum Role {
    /// The initiator of the transaction.
    Host, // 调用方（不一定是主机）
    /// The receiver and executor of the transaction.
    Device, // 接收方（不一定是设备）
    /// Indicates that no transaction is active, and thus no specific role is assigned.
    Idle, // （空闲期没有角色）
}

/// Represents a parsed or to-be-sent PK Command.
///
/// A command consists of a 2-character base-94 `msg_id`, a 5-character `operation`,
/// an optional 5-character `object`, and optional variable-length `data`.
#[derive(PartialEq, Clone, Debug)]
pub struct Command {
    /// The numeric message ID (0-8835).
    pub msg_id: u16,
    /// The operation to be performed.
    pub operation: Operation,
    /// The target object of the operation (e.g., variable or method name).
    pub object: Option<String>,
    /// Optional payload data.
    pub data: Option<Vec<u8>>,
}

impl Command {
    /// Parses a byte slice into a [`Command`] struct.
    ///
    /// The input must follow the protocol format: `[ID][OP] [OBJ] [DATA]`.
    ///
    /// # Arguments
    /// * `msg_bytes`: The raw bytes received from the transport layer.
    ///
    /// # Returns
    /// A [`Result`] containing the parsed [`Command`] or an error message.
    ///
    /// # Errors
    /// Returns an error if the byte slice is not a valid PK Command. (For example, the length is too short, the MSG ID is invalid,
    /// or the operation name is unrecognized.)
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

    /// Serializes the [`Command`] into a [`Vec<u8>`] for transmission.
    ///
    /// This method ensures the output matches the fixed-length field requirements
    /// of the PK Command protocol.
    ///
    /// # Panics
    /// Panics if the `msg_id` is out of the valid 0-8835 range.
    /// This usually indicates a tragic programming error.
    ///
    /// # Examples
    /// ```
    /// use pk_command::types::{Command, Operation};
    /// let cmd = Command {
    ///     msg_id: 2,
    ///     operation: Operation::SendVariable,
    ///     object: Some("VARIA".to_string()),
    ///     data: Some(b"payload".to_vec()),
    /// };
    /// assert_eq!(cmd.to_bytes(), b"!#SENDV VARIA payload".to_vec());
    /// ```
    ///
    /// ```should_panic
    /// use pk_command::types::{Command, Operation};
    /// let cmd = Command {
    ///     msg_id: 9000, // Invalid MSG ID (greater than 8835)
    ///     operation: Operation::SendVariable,
    ///     object: Some("VARIA".to_string()),
    ///     data: Some(b"payload".to_vec()),
    /// };
    /// cmd.to_bytes(); // This should panic due to invalid MSG ID
    /// ```
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
            vec.push(b' ');
            vec.append(&mut self.data.clone().unwrap());
            vec
        }
    }
}

impl std::fmt::Display for Command {
    /// Formats the command for debugging or logging purposes.
    ///
    /// The output mimics the protocol format, but ensures non-printable data is handled gracefully.
    ///
    /// # Example
    /// ```
    /// use pk_command::types::{Command, Operation};
    /// let cmd = Command {
    ///     msg_id: 0,
    ///     operation: Operation::Error,
    ///     object: Some("ERROR".to_string()),
    ///     data: Some(b"Some error description".to_vec()),
    /// };
    /// assert_eq!(format!("{}", cmd), "  ERROR ERROR Some error description");
    ///
    /// let cmd_non_printable = Command {
    ///     msg_id: 1145,
    ///     operation: Operation::Data,
    ///     object: Some("QUERY".to_string()),
    ///     data: Some(vec![0xFF, 0x00, 0xAB]),
    /// };
    ///
    /// // "-2" is the base-94 encoding of 1145, and the data is non-printable,
    /// // so it should show as "<BINARY DATA>".
    /// assert_eq!(
    ///     format!("{}", cmd_non_printable),
    ///     "-2SDATA QUERY <BINARY DATA: 3 bytes>"
    /// );
    ///
    /// let cmd_utf8 = Command {
    ///     msg_id: 1145,
    ///     operation: Operation::Data,
    ///     object: Some("QUERY".to_string()),
    ///     data: Some("汉字🐼".as_bytes().to_vec()),
    /// };
    /// // The data is valid UTF-8, so it should be displayed as is.
    /// assert_eq!(format!("{}", cmd_utf8), "-2SDATA QUERY 汉字🐼");
    /// ```
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
                #[cfg(not(feature = "std"))]
                use alloc::str::from_utf8;
                #[cfg(feature = "std")]
                use std::str::from_utf8;

                if let Ok(data) = from_utf8(data_vec) {
                    write!(f, " {}", data)?;
                } else {
                    write!(f, " <BINARY DATA: {} bytes>", data_vec.len())?;
                };
            }
        }
        Ok(())
    }
}

/// Indicates the current acknowledgment status of the participant.
#[derive(PartialEq, Eq, Clone, Copy)]
pub enum Status {
    /// Normal state, not awaiting any acknowledgment.
    Other,
    /// Currently waiting for a standard `ACKNO` to be received.
    AwaitingAck,
    /// Currently waiting for an acknowledgment to an `ERROR` packet.
    AwaitingErrAck,
}

/// Defines the high-level stages of a PK Command transaction chain.
#[derive(PartialEq, Eq, Clone, Copy)]
pub enum Stage {
    /// No transaction is currently active.
    Idle,
    /// A `START` command has been sent or received.
    Started,
    /// The root operation (e.g., `SENDV`) has been established.
    RootOperationAssigned,
    /// Parameter data is currently being transferred via `SDATA` or `EMPTY`.
    SendingParameter,
    /// All parameters have been sent, and the transaction is awaiting a `QUERY` or processing.
    ParameterSent,
    /// The response/return data is currently being transferred.
    SendingResponse,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::msg_id;

    #[test]
    fn test_command_parse_valid_simple() {
        let bytes = b"!!START";
        let cmd = Command::parse(bytes).unwrap();
        assert_eq!(cmd.msg_id, 0);
        assert_eq!(cmd.operation, Operation::Start);
        assert!(cmd.object.is_none());
        assert!(cmd.data.is_none());
    }

    #[test]
    fn test_command_parse_valid_with_object() {
        let bytes = b"!\"SENDV VARIA";
        let cmd = Command::parse(bytes).unwrap();
        assert_eq!(cmd.msg_id, msg_id::to_u16("!\"").unwrap());
        assert_eq!(cmd.operation, Operation::SendVariable);
        assert_eq!(cmd.object, Some("VARIA".to_string()));
        assert!(cmd.data.is_none());
    }

    #[test]
    fn test_command_parse_valid_with_object_and_data() {
        let bytes = b"!#SENDV VARIA data_payload";
        let cmd = Command::parse(bytes).unwrap();
        assert_eq!(cmd.msg_id, msg_id::to_u16("!#").unwrap());
        assert_eq!(cmd.operation, Operation::SendVariable);
        assert_eq!(cmd.object, Some("VARIA".to_string()));
        assert_eq!(cmd.data, Some(b"data_payload".to_vec()));
    }

    #[test]
    fn test_command_parse_error_command() {
        let bytes = b"  ERROR ERROR Some error description";
        let cmd = Command::parse(bytes).unwrap();
        assert_eq!(cmd.msg_id, 0);
        assert_eq!(cmd.operation, Operation::Error);
        assert_eq!(cmd.object, Some("ERROR".to_string()));
        assert_eq!(cmd.data, Some(b"Some error description".to_vec()));
    }

    #[test]
    fn test_command_parse_ackno_error_command() {
        let bytes = b"  ACKNO ERROR";
        let cmd = Command::parse(bytes).unwrap();
        assert_eq!(cmd.msg_id, 0);
        assert_eq!(cmd.operation, Operation::Acknowledge);
        assert_eq!(cmd.object, Some("ERROR".to_string()));
        assert!(cmd.data.is_none());
    }

    #[test]
    fn test_command_parse_invalid_error_msg_id() {
        // space msg_id's are only allowed in ERROR and ACKNO ERROR commands
        let bytes = b"  START";
        let result = Command::parse(bytes);
        assert!(result.is_err());
    }

    #[test]
    fn test_command_parse_invalid_too_short() {
        // assert_eq!(
        //     Command::parse(b"!!STA"),
        //     Err("Invalid length: message is too short.")
        // );
        let result = Command::parse(b"!!STA");
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "Invalid length: message is too short.");
    }

    #[test]
    fn test_command_parse_invalid_msg_id() {
        // LF(0x0A) and CR(0x0D) is not in the charset
        let result = Command::parse(b"\n\rSTART");
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), "Invalid MSG ID format.");
    }

    #[test]
    fn test_command_to_bytes_simple() {
        let cmd = Command {
            msg_id: 0,
            operation: Operation::Start,
            object: None,
            data: None,
        };
        assert_eq!(cmd.to_bytes(), b"!!START".to_vec());
    }

    #[test]
    fn test_command_to_bytes_with_object_and_data() {
        let cmd = Command {
            msg_id: msg_id::to_u16("!#").unwrap(),
            operation: Operation::SendVariable,
            object: Some("VARIA".to_string()),
            data: Some(b"payload".to_vec()),
        };

        let mut expected = b"!#SENDV VARIA".to_vec();
        expected.push(b' ');
        expected.extend_from_slice(b"payload");
        assert_eq!(cmd.to_bytes(), expected);
    }

    #[test]
    fn test_command_to_bytes_error() {
        let cmd = Command {
            msg_id: 0, // msg_id is ignored for ERROR
            operation: Operation::Error,
            object: Some("ERROR".to_string()),
            data: Some(b"Test error".to_vec()),
        };
        let mut expected = b"  ERROR ERROR".to_vec();
        expected.push(b' ');
        expected.extend_from_slice(b"Test error");
        assert_eq!(cmd.to_bytes(), expected);
    }
}
