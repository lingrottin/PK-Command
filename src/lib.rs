//! # PK Command
//!
//! A lightweight, reliable data transfer protocol designed for constrained channels
//! (e.g., HID, Serial) between a **Host** and an **Embedded Device**.
//!
//! ## Key Features
//! - **Reliability**: Built-in ACK/Retransmission mechanism.
//! - **Efficiency**: Fixed-length headers and data slicing for small MTUs.
//! - **Flexibility**: Supports variable access (GET/SET) and remote method invocation (INVOK).
//! - **no_std Support**: Core logic is compatible with embedded systems without an OS. **Note that `alloc` is still used in no_std environments.**
//! - **Wait Mechanism**: Keep-alive `AWAIT` packets for long-running operations.
//!
//! ## Architecture
//! The protocol operates using **Transaction Chains**. A chain starts with a `START`
//! command and ends with `ENDTR`. Large payloads are automatically sliced into
//! `SDATA` packets.
//!
//! ## Command Format
//! Every command follows the fixed layout:
//!
//! `[MSG ID][OPERATION NAME] [OBJECT] [DATA]`.
//!
//! - `MSG ID`: A base-94 encoded unsigned integer (using ASCII characters from '!' to '~') that uniquely identifies the command within a transaction.
//! - `OPERATION NAME`: A 5-character ASCII string representing the command type. All the operations defined in the specification are defined in this library. (See [`types::Operation`] for details.)
//! - `OBJECT`: An optional 5-character ASCII string that provides additional context for the operation (e.g., variable name, method name).
//! - `DATA`: An optional binary payload that carries parameters or return values. It can be of arbitrary length and may contain any byte values.
//!
//! See [`Command`] for a structured representation of commands and utilities for parsing and serialization.
//!
//! Note that:
//! - `OBJECT` is either omitted, or **exactly 5 ASCII characters**.
//! - `DATA` (payload) is **arbitrary binary data**.
//! - The total length of a command is limited by the transportation layer's MTU (e.g., 64 bytes for HID). (See [`PkCommandConfig`].)
//!   But the protocol handles slicing and reassembly of large payloads automatically, so you can work with large data without worrying about the underlying transport constraints.
//!
//! ## Example
//! ```no_run
//! use pk_command::{PkCommand, PkCommandConfig, PkHashmapMethod, PkHashmapVariable};
//!
//! // 1. Setup configuration and accessors
//! let config = PkCommandConfig::default(64);
//! let vars = PkHashmapVariable::new(vec![]);
//! let methods = PkHashmapMethod::new(vec![]);
//!
//! // 2. Initialize the state machine
//! let pk = PkCommand::<_, _, std::time::Instant>::new(config, vars, methods);
//! # let transport=pk_command::doc_util::Transport::new();
//!
//! // 3. Basic loop driving the protocol
//! loop {
//!     // Handle received bytes from your transport (HID/Serial/etc.)
//!     if let Some(bytes) = transport.recv() {
//!         pk.incoming_command(bytes);
//!     }
//!
//!     // Process and get commands to send back
//!     if let Some(cmd) = pk.poll() {
//!         transport.send(cmd.to_bytes());
//!     }
//!
//!     if pk.is_complete() {
//!         break;
//!     }
//! }
//! ```
//!
//! # Feature flags
//! - `std`: Enables features that require the Rust standard library. (Mainly the convenient wrappers like [`PkPromise`], [`PkHashmapVariable`], [`PkHashmapMethod`]) **Enabled by default.**
//! - `embassy`: Enables integration with the [Embassy](https://embassy.dev/) async framework. Flags below are also enabled when this is active:
//!   - `embassy-time`: Enables the support for [embassy-time](https://crates.io/crates/embassy-time) crate, which provides timekeeping utilities for embedded environments.
//!   - `embassy-runtime`: Enables the support for [embassy-executor](https://crates.io/crates/embassy-executor) crate, which provides integration between the main state machine and Embassy async tasks.
//! - `tokio-runtime`: Enables integration with the [Tokio](https://tokio.rs/) async runtime. Provides [`tokio_adapter`] for running async operations within method implementations. Requires `std` feature.
//! - `smol-runtime`: Enables integration with the [Smol](https://github.com/smol-rs/smol) async executor. Provides [`smol_adapter`] for running async operations within method implementations. Requires `std` feature.

#![warn(missing_docs)]
#![cfg_attr(not(feature = "std"), no_std)]
#![cfg_attr(docsrs, feature(doc_cfg))]

const PK_VERSION: &str = env!("CARGO_PKG_VERSION");

// Compile-time guard: async runtime adapters require `std` feature.
#[cfg(all(
    any(feature = "tokio-runtime", feature = "smol-runtime"),
    not(feature = "std")
))]
compile_error!("Enabling 'tokio-runtime' or 'smol-runtime' requires the 'std' feature.");

#[cfg(not(feature = "std"))]
extern crate alloc;
#[cfg(not(feature = "std"))]
use alloc::{
    boxed::Box,
    string::{String, ToString},
    vec,
    vec::Vec,
};

#[cfg(not(feature = "std"))]
extern crate core as std;

// The items below (not gated behind the "std" feature)
// are re-exported by `std` crate from `core`,
// so just simply renaming `core` as `std` should work
use std::cell::{Cell, RefCell};
use std::ops::Add;
use std::pin::Pin;
use std::task::Poll;
use std::time::Duration;

/// Core data structures and types for PK Command.
pub mod types;
use types::{Command, Operation, Role, Stage, Status};

/// Utilities used in examples.
#[doc(hidden)]
#[cfg(feature = "doc")]
pub mod doc_util;

mod util;
#[cfg_attr(docsrs, doc(cfg(feature = "embassy-runtime")))]
#[cfg(feature = "embassy-runtime")]
pub use util::async_adapters::embassy as embassy_adapter;
#[cfg_attr(docsrs, doc(cfg(all(feature = "std", feature = "smol-runtime"))))]
#[cfg(all(feature = "std", feature = "smol-runtime"))]
pub use util::async_adapters::smol as smol_adapter;
#[cfg(all(feature = "std", feature = "tokio-runtime"))]
#[cfg_attr(docsrs, doc(cfg(all(feature = "std", feature = "tokio-runtime"))))]
pub use util::async_adapters::tokio as tokio_adapter;
#[cfg(feature = "std")]
#[cfg_attr(docsrs, doc(cfg(feature = "std")))]
pub use util::{PkHashmapMethod, PkHashmapVariable, PkPromise, msg_id};

/// Trait defining how to access (get/set) variables by their string key.
///
/// This allows the [`PkCommand`] state machine to be generic over the actual variable storage.
/// In `std` environments, a convenient implementation using [`std::collections::HashMap`] is provided as [`PkHashmapVariable`].
/// In `no_std` environments, you must provide your own implementation.
///
/// # Example
/// ```
/// use pk_command::PkVariableAccessor;
///
/// struct MyVariableStore;
/// impl PkVariableAccessor for MyVariableStore {
///     fn get(&self, key: String) -> Option<Vec<u8>> {
///         if key == "VERSION" {
///             Some(b"1.0.0".to_vec())
///         } else {
///             None
///         }
///     }
///     fn set(&self, key: String, value: Vec<u8>) -> Result<(), String> {
///         // Logic to store the value
///         Ok(())
///     }
/// }
/// ```
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

/// A handle for a long-running operation that can be polled for completion.
///
/// This is used primarily by the `INVOK` operation. Since PK Command is designed
/// for poll-based environments (like in embedded systems, or within an async runtime),
/// methods that take time to execute should return a [`Pollable`].
///
/// The state machine will call [`poll()`](crate::Pollable::poll) periodically and send `AWAIT` packets
/// to the host to keep the transaction alive as long as [`Poll::Pending`] is returned.
pub trait Pollable {
    /// Polls the operation for completion.
    ///
    /// # Returns
    /// - `Poll::Ready(Ok(Some(data)))`: Operation finished with result data.
    /// - `Poll::Ready(Ok(None))`: Operation finished successfully with no data.
    /// - `Poll::Ready(Err(e))`: Operation failed with an error message.
    /// - `Poll::Pending`: Operation is still in progress.
    fn poll(&self) -> std::task::Poll<Result<Option<Vec<u8>>, String>>;
}

/// Trait defining how to invoke methods by their string key.
///
/// This allows the [`PkCommand`] state machine to call arbitrary logic on the device.
/// In `std` environments, a convenient implementation using [`std::collections::HashMap`] is provided as [`PkHashmapMethod`].
/// In `no_std` environments, you must provide your own implementation.
///
/// # Example
/// ```
/// use pk_command::{PkMethodAccessor, Pollable};
/// use std::pin::Pin;
/// use std::task::Poll;
///
/// struct MyMethod;
/// impl Pollable for MyMethod {
///     fn poll(&self) -> Poll<Result<Option<Vec<u8>>, String>> {
///         Poll::Ready(Ok(Some(b"Hello from PK!".to_vec())))
///     }
/// }
///
/// struct MyMethodStore;
/// impl PkMethodAccessor for MyMethodStore {
///     fn call(&self, key: String, param: Vec<u8>) -> Result<Pin<Box<dyn Pollable>>, String> {
///         if key == "GREET" {
///             Ok(Box::pin(MyMethod))
///         } else {
///             Err("Method not found".to_string())
///         }
///     }
/// }
/// ```
pub trait PkMethodAccessor {
    /// Calls a method with the given parameters.
    ///
    /// # Arguments
    /// * `key`: The name of the method to call.
    /// * `param`: The parameters for the method, as a byte vector.
    ///
    /// # Returns
    /// A `Result` containing a pinned, boxed `Pollable` that will resolve to the method's output,
    /// or an `Err(String)` if the method call cannot be initiated.
    fn call(&self, key: String, param: Vec<u8>) -> Result<Pin<Box<dyn Pollable>>, String>;
}

/// Trait representing an instant in time.
///
/// This trait abstracts over the [`std::time::Instant`] to support `no_std` environments
/// where a custom timer implementation might be needed. In `std` environments, using [`std::time::Instant`] is recommended.
///
/// # Note
///
/// If you want to provide a custom implementation to be used in [`PkCommand`], besides implementing this trait,
/// you also need to ensure that `Add<Duration, Output = Instant>`, `PartialOrd`, and `Copy` are implemented for your type.
///
/// # Tips
/// If you are using Embassy on your embedded device, you can use the provided [`EmbassyInstant`] adapter, which wraps [`embassy_time::Instant`] and implements the necessary traits for compatibility with [`PkCommand`].
pub trait PkInstant
where
    Self: Sized,
{
    /// Returns the current instant.
    fn now() -> Self;
    /// Returns the duration elapsed since this instant.
    fn elapsed(&self) -> Duration;
}

#[cfg(feature = "std")]
impl PkInstant for std::time::Instant {
    fn now() -> Self {
        std::time::Instant::now()
    }

    fn elapsed(&self) -> Duration {
        std::time::Instant::elapsed(self)
    }
}

/// A [`PkInstant`] adapter for [`embassy_time::Instant`].
///
///
///
/// This type bridges the PK Command timing abstraction with the
/// [`embassy-time`](https://crates.io/crates/embassy-time) crate, enabling timeout handling in `no_std`/embedded
/// environments that use Embassy.
///
/// # Availability
/// Enabled when the `embassy` feature is active (it also enables `embassy-time`).
///
/// # Why this wrapper exists
/// [`embassy_time::Instant`] is tied to [`embassy_time::Duration`], while
/// PK Command uses [`core::time::Duration`]. These durations can be converted,
/// but their types are not directly compatible with the `PkCommand` signature,
/// so a newtype adapter keeps the API consistent without large generic changes.
///
/// # Notes
/// - Implements [`Add<Duration>`] with millisecond precision.
/// - [`elapsed()`](crate::EmbassyInstant::elapsed) guards against clock rollback by returning `0` if the
///   current instant is earlier than the stored instant.
///
/// # Example
///
/// ```
/// use pk_command::{
///     EmbassyInstant, PkCommand, PkCommandConfig, PkHashmapMethod, PkHashmapVariable,
/// };
///
/// // embassy_timer::Instant must be constructed within an Embassy context
/// // otherwise the code would not compile.
/// #[embassy_executor::task]
/// async fn example() {
///     let pk = PkCommand::<_, _, EmbassyInstant>::new(
///         PkCommandConfig::default(64),
///         PkHashmapVariable::new(vec![]),
///         PkHashmapMethod::new(vec![]),
///     );
/// }
/// ```
///
/// # See also
///
/// To use PK Command within an Embassy context, you might also want to have a
/// look at [`embassy_adapter`].
///
#[cfg(feature = "embassy-time")]
#[cfg_attr(docsrs, doc(cfg(feature = "embassy-time")))]
#[derive(Clone, Copy, PartialOrd, PartialEq, Eq, Ord)]
pub struct EmbassyInstant(embassy_time::Instant);

#[cfg(feature = "embassy-time")]
impl EmbassyInstant {
    fn into_inner(self) -> embassy_time::Instant {
        self.0
    }
}

#[cfg(feature = "embassy-time")]
impl Add<Duration> for EmbassyInstant {
    type Output = EmbassyInstant;

    fn add(self, rhs: Duration) -> Self::Output {
        EmbassyInstant(self.0 + embassy_time::Duration::from_millis(rhs.as_millis() as u64))
    }
}

#[cfg(feature = "embassy-time")]
impl From<embassy_time::Instant> for EmbassyInstant {
    fn from(inst: embassy_time::Instant) -> Self {
        EmbassyInstant(inst)
    }
}

#[cfg(feature = "embassy-time")]
impl PkInstant for EmbassyInstant {
    fn now() -> Self {
        embassy_time::Instant::now().into()
    }

    fn elapsed(&self) -> core::time::Duration {
        let now = embassy_time::Instant::now();
        if now >= self.into_inner() {
            (now - self.into_inner()).into()
        } else {
            // This case can happen if the system clock was adjusted backwards.
            core::time::Duration::from_secs(0)
        }
    }
}

/// Configuration for the [`PkCommand`] state machine.
///
/// Use this struct to define timeout durations and packet size limits according to your
/// transport layer's constraints (e.g., HID, Serial, etc.).
#[derive(Clone)]
pub struct PkCommandConfig {
    /// Timeout duration for waiting for an `ACKNO` command. Default is 100ms.
    ack_timeout: Duration,
    /// Timeout duration for waiting for the next command in a sequence. Default is 500ms.
    inter_command_timeout: Duration,
    /// Interval at which the Device sends `AWAIT` keep-alive commands. Default is 300ms.
    await_interval: Duration,
    /// The maximum length of a single command packet (in bytes), including headers.
    packet_limit: u64,
    /// The version string of the package.
    pk_version: &'static str,
}

impl PkCommandConfig {
    /// Creates a [`PkCommandConfig`] with default (as recommended in the specification file) timeout values.
    ///
    /// # Default timeouts
    /// - ACK timeout: 100ms
    /// - Inter command timeout: 500ms
    /// - `AWAIT` interval: 300ms
    ///
    /// # Arguments
    /// * `packet_limit`: The maximum packet size (MTU) of the underlying transport (e.g., 64 for HID).
    ///
    /// # Returns
    /// A [`PkCommandConfig`] instance with default timeouts and the specified packet limit.
    ///
    /// # Note
    /// This is **not** an implementation of [`Default`] trait because `packet_limit` must be specified.
    pub fn default(packet_limit: u64) -> Self {
        PkCommandConfig {
            ack_timeout: Duration::from_millis(100),
            inter_command_timeout: Duration::from_millis(500),
            await_interval: Duration::from_millis(300),
            packet_limit,
            pk_version: PK_VERSION,
        }
    }

    /// Creates a new [`PkCommandConfig`] with custom timing and packet limit.
    ///
    /// # Arguments
    /// * `ack_timeout`: Timeout for ACKs in milliseconds.
    /// * `inter_command_timeout`: Timeout between commands in milliseconds.
    /// * `await_interval`: Interval for sending `AWAIT` keep-alives in milliseconds.
    /// * `packet_limit`: Maximum length of a single packet in bytes.
    ///
    /// # Note
    /// To avoid undesirable behavior, you should ensure that the timeout values on both sides (Host and Device) are exactly the same.
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
            packet_limit,
            pk_version: PK_VERSION,
        }
    }
}

/// The main state machine for handling the PK Command protocol.
///
/// It manages the lifecycle of a transaction, including:
/// - Parsing incoming raw bytes into commands.
/// - Generating response commands (ACKs, data slices, etc.).
/// - Handling timeouts and retransmissions.
/// - Managing data slicing for large transfers.
///
/// This struct is generic over:
/// - `VA`: A [`PkVariableAccessor`] for variable storage.
/// - `MA`: A [`PkMethodAccessor`] for method invocation.
/// - `Instant`: A [`PkInstant`] for time tracking (allowing for `no_std` timer implementations). Typically [`std::time::Instant`] in `std` environments, or [`EmbassyInstant`] in Embassy environments.
///
/// # Usage Pattern
/// 1. Feed received data into [`incoming_command()`](crate::PkCommand::incoming_command).
/// 2. Regularly call [`poll()`](crate::PkCommand::poll) to progress the state machine and check for commands to send.
/// 3. If [`poll()`](crate::PkCommand::poll) returns `Some(Command)`, serialize it with [`to_bytes()`](crate::types::Command::to_bytes) and send it over your transport.
///
/// # Host vs Device
///
/// They are not actual device types, but rather roles in a transaction.
///
/// - **Host** is the one who calls [`perform()`](crate::PkCommand::perform) to initiate a transaction (e.g., `SENDV`, `INVOK`).
/// - **Device** is the one who reacts against the transaction and automatically responds to incoming root commands using the provided accessors.
///
/// # Example
/// ```no_run
/// use pk_command::{PkCommand, PkCommandConfig, PkHashmapMethod, PkHashmapVariable};
///
/// let config = PkCommandConfig::default(64);
/// let vars = PkHashmapVariable::new(vec![]);
/// let methods = PkHashmapMethod::new(vec![]);
/// let pk = PkCommand::<_, _, std::time::Instant>::new(config, vars, methods);
/// # let transport = pk_command::doc_util::Transport::new();
/// loop {
///     // 1. Receive data from transport...
///     if let Some(received_bytes) = transport.recv() {
///         pk.incoming_command(received_bytes);
///     }
///
///     // 2. Drive the state machine
///     if let Some(cmd) = pk.poll() {
///         let bytes = cmd.to_bytes();
///         transport.send(bytes);
///     }
///     std::thread::sleep(std::time::Duration::from_millis(10));
/// }
/// ```
pub struct PkCommand<VA, MA, Instant>
where
    VA: PkVariableAccessor,
    MA: PkMethodAccessor,
    Instant: PkInstant + Add<Duration, Output = Instant> + PartialOrd + Copy,
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
    pending_pollable: RefCell<Option<Pin<Box<dyn Pollable>>>>,
    device_should_return: Cell<bool>, // 设备是否“收到了 QUERY 但还没有返回值”
}

impl<
    VA: PkVariableAccessor,
    MA: PkMethodAccessor,
    Instant: PkInstant + Add<Duration, Output = Instant> + PartialOrd + Copy,
> PkCommand<VA, MA, Instant>
{
    /// Ingests a raw command received from the other party.
    ///
    /// This should be called whenever new bytes arrive on your transport layer. The
    /// state machine will parse the bytes and update its internal buffers for the
    /// next [`poll()`](crate::PkCommand::poll) cycle.
    ///
    /// # Arguments
    /// * `command_bytes`: The raw bytes of the received command.
    ///
    /// # Returns
    /// `Ok(())` if the command was successfully parsed and buffered.
    /// `Err(&'static str)` if parsing failed (e.g., invalid format, unknown operation).
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

    /// Slices a chunk of data from internal buffers for multipart transfer.
    ///
    /// This is an internal utility used during `SDATA` phases.
    fn slice_data(&self, role: Role) -> Result<(Vec<u8>, bool), &'static str> {
        // 如果 Role 是 Device 则默认在发送返回值，反之亦然
        match role {
            Role::Device => {
                let data = self.data_return.borrow();
                if data.is_empty() {
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
                if data.is_empty() {
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

    /// Polls the state machine for progress and pending actions.
    ///
    /// See [`PkCommand`] for more details.
    ///
    /// This method must be called frequently in your main loop. It handles:
    /// 1. **Processing**: Consuming commands received via `incoming_command`.
    /// 2. **Execution**: Running device-side logic (variable access, method polling).
    /// 3. **Protocol Flow**: Automatically generating ACKs, data slices, and ENDTRs.
    /// 4. **Reliability**: Handling timeouts and retransmitting lost packets.
    ///
    /// # Returns
    ///
    /// - `Some(Command)`: A command that needs to be sent to the peer.
    ///   Serialize it with [`to_bytes()`](crate::types::Command::to_bytes) and transmit it.
    /// - `None`: No action required at this time.
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
            self.data_param.replace(vec![]);
            self.data_return.replace(vec![]);
            self.sending_data_progress.set(0);
            self.device_op_pending.set(false);
            self.device_await_deadline.set(None);
            self.pending_pollable.replace(None); //确保清理
            self.device_should_return.set(false);
        };
        let ack = move |msg_id: u16, operation: Operation| -> Option<Command> {
            self.last_command_time.set(Instant::now());
            Some(Command {
                msg_id,
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
                // 当设备有挂起的 INVOK 操作并且处于响应阶段时，轮询 Pollable
                if self.role.get() == Role::Device
                    && self.device_op_pending.get()
                    && self.stage.get() == Stage::SendingResponse
                {
                    // 如果正在等待 AWAIT 的 ACK，则不轮询主 Pollable, ACK 超时机制处理 AWAIT 的重传
                    if self.status.get() == Status::AwaitingAck {
                        // Timeout for AWAIT's ACK will be handled by the generic timeout logic below.
                    } else if self.status.get() == Status::AwaitingErrAck {
                        // This state is unlikely if a device operation is pending normally.
                        // Consider if an error should be raised or state reset.
                    } else {
                        // Status::Other, ready to poll the main INVOK pollable
                        let mut pollable_store = self.pending_pollable.borrow_mut();

                        if let Some(pinned_pollable) = pollable_store.as_mut() {
                            match pinned_pollable.as_mut().poll() {
                                Poll::Ready(result) => {
                                    pollable_store.take(); // Remove completed pollable
                                    self.device_op_pending.set(false);
                                    self.device_await_deadline.set(None);

                                    match result {
                                        Ok(data_opt) => {
                                            self.data_return.replace(data_opt.unwrap_or_default());
                                            // Stage is already SendingResponse.
                                            self.sending_data_progress.set(0); // Reset for sending return data.

                                            let rturn_object_name =
                                                if self.data_return.borrow().is_empty() {
                                                    String::from("EMPTY")
                                                } else {
                                                    Operation::Invoke.to_name().to_string()
                                                };
                                            return send(Command {
                                                msg_id: next_msg_id_for_send(),
                                                operation: Operation::Return,
                                                object: Some(rturn_object_name),
                                                data: None,
                                            });
                                        }
                                        Err(_) => {
                                            reset_transaction_state();
                                            return err("INVOK operation failed");
                                        }
                                    }
                                }
                                Poll::Pending => {
                                    if Instant::now()
                                        >= self
                                            .device_await_deadline
                                            .get()
                                            .unwrap_or(Instant::now())
                                    {
                                        self.device_await_deadline
                                            .set(Some(Instant::now() + self.config.await_interval));
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
                            // device_op_pending is true, but no pollable.
                            reset_transaction_state();
                            return err("Internal: Device op pending but no pollable.");
                        }
                    }
                } // 结束 device_op_pending && Stage::SendingResponse 的处理
                if self.device_should_return.get() {
                    self.sending_data_progress.set(0); // 重置发送进度
                    self.device_should_return.set(false);
                    // 这时候的状态应该是收到了 QUERY，还没有发送返回值
                    match self.root_operation.get() {
                        Operation::GetVersion => {
                            return send(Command {
                                msg_id: next_msg_id_for_send(),
                                operation: Operation::Return,
                                object: Some(self.root_operation.get().to_name().to_string()),
                                data: None,
                            });
                        }
                        Operation::RequireVariable => {
                            if self.data_return.borrow().is_empty() {
                                return send(Command {
                                    msg_id: next_msg_id_for_send(),
                                    operation: Operation::Return,
                                    object: Some(String::from("EMPTY")),
                                    data: None,
                                });
                            }
                            return send(Command {
                                msg_id: next_msg_id_for_send(),
                                operation: Operation::Return,
                                object: Some(self.root_operation.get().to_name().to_string()),
                                data: None,
                            });
                        }
                        Operation::SendVariable => {
                            // SENDV doesn't return data in the RTURN command itself.
                            // The result of the set operation is implicitly acknowledged by the ENDTR ACK.
                            // If there was an error during set, it would be handled by the error path.
                            // We still send RTURN EMPTY to signal the end of the Device's processing phase.
                            self.data_return.replace(vec![]); // Ensure data_return is empty
                            return send(Command {
                                msg_id: next_msg_id_for_send(),
                                operation: Operation::Return,
                                object: Some(Operation::Empty.to_name().to_string()),
                                data: None,
                            });
                        }
                        Operation::Invoke => {
                            // 忽略，因为 Invoke 的返回在上面轮询 Pollable 时处理
                        }
                        _ => {
                            panic!("Not a root operation");
                        }
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
                        // 仅当不在 Idle 状态且没有挂起的设备操作时检查指令间超时
                        if self.stage.get() != Stage::Idle
                            && !self.device_op_pending.get()
                            && elapsed_ms >= self.config.inter_command_timeout
                        {
                            reset_transaction_state(); // 在发送错误前重置状态
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
                } else if self.status.get() == Status::AwaitingErrAck {
                    if recv.operation == Operation::Acknowledge
                        && Some(String::from("ERROR")) == recv.object
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
                                    if self.data_param.borrow().is_empty() {
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
                                    {
                                        // 缩小可变借用的作用域，确保归还
                                        self.data_param
                                            .borrow_mut()
                                            .append(&mut recv.data.as_ref().unwrap().clone());
                                    }
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
                                self.status.set(Status::Other);

                                // 将借用操作限制在最小作用域，以避免后续调用 send() 或 err() 时发生冲突
                                let last_sent_op;
                                {
                                    last_sent_op = self.last_sent_command.borrow().operation;
                                } // 不可变借用在此结束

                                match last_sent_op {
                                    Operation::Empty => {
                                        // 收到对 EMPTY 的 ACKNO，参数传输结束，发送 ENDTR
                                        self.stage.set(Stage::ParameterSent);
                                        return send(Command {
                                            msg_id: next_msg_id_for_send(),
                                            operation: Operation::EndTransaction,
                                            object: None,
                                            data: None,
                                        });
                                    }
                                    Operation::Data => {
                                        // 收到对 SDATA 的 ACKNO
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
                        /* Host -> 收到对 ENDTR 的 ACK，发送 QUERY。等待回传数据或 AWAIT 保活。
                        Device -> 收到 QUERY，执行逻辑，处理保活和/或回传数据。 */
                        match self.role.get() {
                            Role::Host => match recv.operation {
                                Operation::Acknowledge => {
                                    self.status.set(Status::Other); // ACK received
                                    if Some(String::from("ENDTR")) == recv.object {
                                        return send(Command {
                                            msg_id: util::msg_id::increment(recv.msg_id),
                                            operation: Operation::Query,
                                            object: None,
                                            data: None,
                                        });
                                    } else if Some(String::from("QUERY")) == recv.object {
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
                                    if Some(String::from("EMPTY")) == recv.object
                                        || Some(self.root_operation.get().to_name().to_string())
                                            == recv.object
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
                                                Ok(pollable) => {
                                                    self.pending_pollable.replace(Some(pollable));
                                                }
                                                Err(_) => {
                                                    reset_transaction_state();
                                                    // log::error!("Failed to create INVOK pollable: {}", e_str);
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
                                    self.stage.set(Stage::SendingResponse);
                                    self.device_should_return.set(true);
                                    return ack(recv.msg_id, recv.operation);
                                }
                            }
                            Role::Idle => {
                                panic!("Role cannot be Idle if Stage is ParameterSent")
                            }
                        }
                    }
                    Stage::SendingResponse => {
                        /* Host -> 接收数据。
                        Device -> 收到对 RTURN/SDATA 的 ACK，继续发送数据或终止 */
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
                                    self.stage.set(Stage::Idle);
                                    self.status.set(Status::Other); // After sending ACK, status is Other
                                    return endtr_ack;
                                } else {
                                    return err(
                                        "Host expected SDATA or ENDTR in SendingResponse stage",
                                    );
                                }
                            }
                            Role::Device => {
                                // Device 必须是收到了 ACKNO
                                if recv.operation != Operation::Acknowledge {
                                    return err("Device expected ACKNO in SendingResponse stage");
                                }
                                self.status.set(Status::Other);

                                // 将借用操作限制在最小作用域
                                let last_sent_op;
                                {
                                    last_sent_op = self.last_sent_command.borrow().operation;
                                } // 不可变借用在此结束

                                match last_sent_op {
                                    Operation::Return => {
                                        // 收到对 RETURN 的 ACKNO
                                        let return_data_len =
                                            self.data_return.borrow().len() as u64;
                                        if return_data_len == 0 {
                                            // 没有返回值，直接发送 ENDTR
                                            // self.stage.set(Stage::Idle); // Transaction ends
                                            // REMOVE: Do not set to Idle yet, wait for ENDTR's ACKNO
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
    /// This starts a new transaction chain. It can only be called when the state machine is `Idle`.
    /// The actual protocol exchange (beginning with a `START` packet) is driven by subsequent [`poll()`](crate::PkCommand::poll) calls.
    ///
    /// # Arguments
    /// * `operation`: The root operation to perform (`SENDV`, `REQUV`, `INVOK`, or `PKVER`).
    /// * `object`: The target name (e.g., variable name for `REQUV`, method name for `INVOK`).
    /// * `data`: Optional parameter data (e.g., the value to set for `SENDV`).
    ///
    /// # Returns
    /// - `Ok(())`: The transaction was successfully queued.
    /// - `Err(&'static str)`: The request was invalid (e.g., already in a transaction, not a root op).
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

    fn reset_transaction_state(&self) {
        self.stage.set(Stage::Idle);
        self.status.set(Status::Other);
        self.role.set(Role::Idle);
        // Clear other relevant fields like root_operation, data_param, data_return, device_op_pending etc.
        self.data_param.borrow_mut().clear();
        self.data_return.borrow_mut().clear();
        self.sending_data_progress.set(0);
        self.device_op_pending.set(false);
        self.device_await_deadline.set(None);
        self.pending_pollable.borrow_mut().take(); // Clear the pollable
    }

    /// Returns `true` if the state machine is currently [`Idle`](crate::types::Stage::Idle) (no active transaction).
    pub fn is_complete(&self) -> bool {
        self.stage.get() == Stage::Idle
    }

    /// Retrieves the return data from a finished transaction and resets the transaction state.
    ///
    /// This should be called by the Host after [`is_complete()`](crate::PkCommand::is_complete) returns `true` for a root
    /// operation that expects return data (e.g., `REQUV` or `INVOK`).
    ///
    /// # Returns
    /// - `Some(Vec<u8>)`: The returned payload.
    /// - `None`: If there was no data or the state machine is not in a completed host state.
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

    /// Checks for transaction completion and executes a callback with the resulting data.
    ///
    /// This is a convenience method for polling for completion on the Host side.
    /// If the transaction is complete, it calls the `callback` with the return data
    /// (if any) and returns `true`.
    ///
    /// # Note
    ///
    /// This function is also poll-based. You should also call it regularly (e.g., in your main loop), and when the transaction completes, it would call the provided function.
    ///
    /// # Returns
    /// `true` if the transaction was complete and the callback was executed.
    ///
    /// # Example
    /// ```no_run
    /// use pk_command::{PkCommand, PkCommandConfig, PkHashmapVariable, PkHashmapMethod};
    ///
    /// let config = PkCommandConfig::default(64);
    /// let vars = PkHashmapVariable::new(vec![]);
    /// let methods = PkHashmapMethod::new(vec![]);
    /// let pk = PkCommand::<_, _, std::time::Instant>::new(config, vars, methods);
    /// # let transport = pk_command::doc_util::Transport::new();
    ///
    /// loop {
    ///     // 1. Receive data from transport...
    ///     if let Some(received_data) = transport.recv() {
    ///         pk.incoming_command(received_data);
    ///     }
    ///
    ///     // 3. perform some operation
    ///     # let some_condition=true;
    ///     # let operation= pk_command::types::Operation::RequireVariable;
    ///     # let object=None;
    ///     # let data=None;
    ///     if some_condition && pk.is_complete() {
    ///         pk.perform(operation, object, data).unwrap();
    ///     }
    ///
    ///     // 4. poll
    ///     let cmd=pk.poll();
    ///
    ///     // 5. check for completion and handle return data
    ///     //    We can see that this function is poll-based as well, and should be called regularly. (typically
    ///     //    right after calling `poll()`) When the transaction completes, it would call the provided function
    ///     //    with the return data (if any).
    ///     let mut should_break=false;
    ///     pk.wait_for_complete_and(|data_opt| {
    ///         println!("Transaction complete! Return data: {:?}", data_opt);
    ///         should_break=true;
    ///     });
    ///
    ///     // 6. Send cmd back via transport...
    ///     if let Some(cmd_to_send) = cmd {
    ///         transport.send(cmd_to_send.to_bytes());
    ///     }
    ///
    ///     // 7. break if needed
    ///     if should_break {
    ///         break;
    ///     }
    /// }
    /// ```
    pub fn wait_for_complete_and<F>(&self, callback: F) -> bool
    where
        F: FnOnce(Option<Vec<u8>>),
    {
        // 这个函数也是轮询的，用来给 Host 方返回值（因为在上面的 perform 中并没有告诉 PK 该怎么处理返回值）
        if self.stage.get() == Stage::Idle {
            let data = self.data_return.borrow().clone();
            self.reset_transaction_state();
            callback(if data.is_empty() { None } else { Some(data) });
            true
        } else {
            false
        }
    }

    /// Creates a new [`PkCommand`] state machine.
    ///
    /// # Arguments
    /// * `config`: Configuration defining timeouts and packet limits.
    /// * `variable_accessor`: Provider for reading/writing variables.
    /// * `method_accessor`: Provider for invoking methods.
    ///
    /// # Note
    ///
    /// ## The Instant Type Parameter
    ///
    /// The `Instant` type parameter must implement [`PkInstant`], [`Copy`], [`PartialOrd`], and [`Add<Duration>`].
    /// We provide a default implementation for [`std::time::Instant`] in `std` environments,
    /// so that could be used directly.
    ///
    /// For `no_std` environments, users can implement their own [`PkInstant`] and use it here.
    ///
    /// ## The VA, MA Type Parameters
    ///
    /// In `std` environments, the library provides two convenient implementations for variable
    /// and method accessors: [`PkHashmapVariable`] and [`PkHashmapMethod`], which use
    /// [`HashMap`](std::collections::HashMap)s internally. You can use them directly or implement
    /// your own if you have different storage needs.
    ///
    /// In `no_std` environments, you must provide your own implementations of `VA`, `MA`,
    /// and `Instant`.
    ///
    /// ## For Embassy Users
    ///
    /// The library provides `no_std` utilities, based on the Embassy ecosystem,
    /// to help you integrate PK Command into a Embassy-based application.
    /// They are:
    ///
    /// - [`EmbassyInstant`] for `Instant`.
    /// - [`EmbassyPollable`](crate::embassy_adapter::EmbassyPollable) for [`Pollable`], and [`embassy_method_accessor!`](crate::embassy_method_accessor) for [`PkMethodAccessor`].
    ///
    /// Unfortunately still, you may need to provide your own implementation of [`PkVariableAccessor`].
    ///
    /// # Example
    /// ```
    /// use pk_command::{PkCommand, PkCommandConfig, PkHashmapVariable, PkHashmapMethod};
    ///
    /// // The third type parameter here is the PkInstant type. This can't be usually inferred so you must
    /// // specify it explicitly. If you are using std, just use std::time::Instant. If you are using no_std,
    /// //  implement your own PkInstant and specify it here.
    /// let pk = PkCommand::<_, _, std::time::Instant>::new(
    ///     PkCommandConfig::default(64),
    ///     PkHashmapVariable::new(vec![]),
    ///     PkHashmapMethod::new(vec![]),
    /// );
    /// ```
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
            pending_pollable: RefCell::new(None),
            device_should_return: Cell::new(false),
        }
    }
}
