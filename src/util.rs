#[cfg(feature = "std")]
use std::sync::{Arc, RwLock};
#[cfg(feature = "std")]
use std::{cell::RefCell, pin::Pin};

pub mod msg_id {
    //! Module for handling PK Command Message IDs.
    //!
    //! ## Overview
    //! Message ID is a 2-character string used to uniquely identify commands and support
    //! the ACK mechanism and retransmission detection in the PK Command protocol.
    //!
    //! ## Base-94 Encoding Scheme
    //! - **Character Set**: Printable ASCII characters from `0x21` (`!`) to `0x7E` (`~`)
    //! - **Total Characters**: 94 characters
    //! - **Encoding Formula**: `ID = (c1 - 0x21) * 94 + (c2 - 0x21)`
    //!   - `c1` and `c2` are the two characters of the MSG ID string
    //!
    //! ## ID Range and Rollover
    //! - **Minimum Value**: 0 (represented as `!!`)
    //! - **Maximum Value**: 8835 (represented as `~~`)
    //! - **Rollover Mechanism**: When ID reaches 8835, the next increment returns 0
    //!
    //! ## Scope and Lifetime
    //! - IDs are **cumulative throughout the entire session**
    //! - IDs are **NOT reset** by `ENDTR` commands
    //! - Used for:
    //!   1. Command tracking: Each command is uniquely identified by its MSG ID
    //!   2. ACK validation: Receiver must return the same MSG ID in ACKNO response
    //!   3. Retransmission detection: Receiving the same MSG ID indicates a retransmitted packet
    //!
    //! ## Special Case: ERROR Command
    //! - ERROR command's MSG ID is fixed as two space characters (`0x20 0x20`)
    //! - Its acknowledgement (`ACKNO ERROR`) also has MSG ID fixed as two spaces

    #[cfg(not(feature = "std"))]
    use alloc::{format, string::String};

    const BASE: u16 = 94;
    const OFFSET: u8 = b'!';
    const MAX_ID: u16 = BASE * BASE - 1;

    /// Converts a 2-character string ID into its u16 integer representation.
    ///
    /// The input string must consist of two characters within the printable ASCII
    /// range (0x21 to 0x7E). Uses Base-94 encoding:
    /// `ID = (c1 - 0x21) * 94 + (c2 - 0x21)`
    ///
    /// # Examples
    /// ```
    /// use pk_command::msg_id;
    /// assert_eq!(msg_id::to_u16("!!"), Ok(0)); // minimum
    /// assert_eq!(msg_id::to_u16("!\""), Ok(1));
    /// assert_eq!(msg_id::to_u16("\"!"), Ok(94));
    /// assert_eq!(msg_id::to_u16("~~"), Ok(8835)); // maximum
    ///
    /// assert!(msg_id::to_u16("!").is_err()); // invalid length
    /// ```
    pub fn to_u16(id_str: &str) -> Result<u16, &'static str> {
        if id_str.len() != 2 {
            // This is an internal utility, so a simple error message is fine.
            // For a public API, more descriptive errors might be preferred.
            // However, given its use within the PK Command protocol, this is likely sufficient.
            // The primary validation for msg_id format happens during command parsing.
            // This function assumes the input string *should* be a valid 2-char ID.
            return Err("Input string must be exactly 2 characters long.");
        }

        let bytes = id_str.as_bytes();
        let c1 = bytes[0];
        let c2 = bytes[1];

        if !((b'!'..=b'~').contains(&c1) && (b'!'..=b'~').contains(&c2)) {
            return Err("Input string contains invalid characters.");
        }

        let val1 = (c1 - OFFSET) as u16;
        let val2 = (c2 - OFFSET) as u16;

        Ok(val1 * BASE + val2)
    }

    /// Converts a u16 integer ID back into its 2-character string representation.
    ///
    /// The ID must be within the valid range (0 to 8835, inclusive).
    /// Uses the inverse of Base-94 encoding:
    /// `c1 = (id / 94) + 0x21`, `c2 = (id % 94) + 0x21`
    ///
    /// # Arguments
    /// * `id`: The u16 integer ID to convert (0-8835).
    ///
    /// # Returns
    /// A `Result` containing the 2-character string ID, or an error message if the ID is out of range.
    ///
    /// # Examples
    /// ```
    /// use pk_command::msg_id;
    /// assert_eq!(msg_id::from_u16(0), Ok("!!".to_string()));
    /// assert_eq!(msg_id::from_u16(1), Ok("!\"".to_string()));
    /// assert_eq!(msg_id::from_u16(94), Ok("\"!".to_string()));
    /// assert_eq!(msg_id::from_u16(8835), Ok("~~".to_string()));
    ///
    /// assert!(msg_id::from_u16(8836).is_err()); // out of range
    /// ```
    pub fn from_u16(id: u16) -> Result<String, &'static str> {
        if id > MAX_ID {
            return Err("Input number is out of the valid range (0-8835).");
        }

        let val1 = id / BASE;
        let val2 = id % BASE;

        let c1 = (val1 as u8 + OFFSET) as char;
        let c2 = (val2 as u8 + OFFSET) as char;

        Ok(format!("{}{}", c1, c2))
    }

    /// Increments a message ID, handling rollover.
    ///
    /// When the ID reaches its maximum value (8835), it rolls over to 0.
    /// This ensures IDs cycle through the entire range [0, 8835] in a continuous session.
    ///
    /// Implemented as: `(id + 1) % 8836`
    ///
    /// # Arguments
    /// * `id`: The current u16 message ID.
    ///
    /// # Returns
    /// The next message ID in the sequence.
    ///
    /// # Examples
    /// ```
    /// use pk_command::msg_id;
    /// assert_eq!(msg_id::increment(0), 1);
    /// assert_eq!(msg_id::increment(100), 101);
    /// assert_eq!(msg_id::increment(8835), 0); // rollover
    /// ```
    pub fn increment(id: u16) -> u16 {
        (id + 1) % (MAX_ID + 1)
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        #[cfg(not(feature = "std"))]
        use alloc::string::ToString;

        #[test]
        fn test_msg_id_to_u16_valid() {
            assert_eq!(to_u16("!!"), Ok(0));
            assert_eq!(to_u16("!\""), Ok(1));
            assert_eq!(to_u16("\"!"), Ok(BASE));
            assert_eq!(to_u16("~~"), Ok(MAX_ID));
        }

        #[test]
        fn test_msg_id_to_u16_invalid_length() {
            assert!(to_u16("!").is_err());
            assert!(to_u16("!!!").is_err());
        }

        #[test]
        fn test_msg_id_to_u16_invalid_chars() {
            assert!(to_u16(" !").is_err()); // Space is not allowed
        }

        #[test]
        fn test_msg_id_from_u16_valid() {
            assert_eq!(from_u16(0), Ok("!!".to_string()));
            assert_eq!(from_u16(1), Ok("!\"".to_string()));
            assert_eq!(from_u16(BASE), Ok("\"!".to_string()));
            assert_eq!(from_u16(MAX_ID), Ok("~~".to_string()));
        }

        #[test]
        fn test_msg_id_from_u16_out_of_range() {
            assert!(from_u16(MAX_ID + 1).is_err());
        }

        #[test]
        fn test_msg_id_increment() {
            assert_eq!(increment(0), 1);
            assert_eq!(increment(MAX_ID), 0); // Rollover
            assert_eq!(increment(100), 101);
        }
    }
}

pub mod async_adapters {
    #[cfg(all(feature = "std", feature = "tokio-runtime"))]
    pub mod tokio {
        //! Tokio runtime adapters.
        //!
        //! This module provides a [`Pollable`](crate::Pollable) wrapper that runs a `Future`
        //! on the Tokio runtime and exposes its completion through the poll-based PK Command
        //! interface.
        //!
        //! # Example
        //! ```no_run
        //! use pk_command::{PkHashmapMethod, tokio_adapter::TokioFuturePollable};
        //!
        //! // use current_thread flavor just for simplifying the dependencies,
        //! // use whatever you want in your actual application
        //! #[tokio::main(flavor = "current_thread")]
        //! async fn main() {
        //!     tokio::spawn(async {
        //!         let method_accessor = PkHashmapMethod::new(vec![(
        //!             String::from("ECHOO"),
        //!             Box::new(move |param: Option<Vec<u8>>| {
        //!                 TokioFuturePollable::from_future(async move {
        //!                     // Note the `async` here     ^^^^^
        //!                     Ok(param) // Echo back the input
        //!                 })
        //!             }),
        //!         )]);
        //!         // let pk=....;
        //!         loop {
        //!             // now you can call pk.poll() as usual.
        //!         }
        //!     });
        //!
        //!     // The async tasks you registered above, when called by the other side,
        //!     // will run in the tokio runtime.
        //! }
        //! ```
        use std::future::Future;
        use std::pin::Pin;
        use std::sync::{Arc, RwLock};

        /// A `Pollable` adapter that spawns a `Future` onto the Tokio runtime and
        /// exposes its completion through the `Pollable` interface.
        ///
        ///
        ///
        /// # Example with [`PkHashmapMethod`](crate::PkHashmapMethod)
        /// ```
        /// use pk_command::{PkCommand, PkHashmapMethod, tokio_adapter::TokioFuturePollable};
        /// let method_impl = Box::new(move |param: Option<Vec<u8>>| {
        ///     TokioFuturePollable::from_future(async move {
        ///         // do_something_with(param);
        ///         Ok(Some(b"async result".to_vec()))
        ///     })
        /// });
        /// let methods = PkHashmapMethod::new(vec![("ASYNC".to_string(), method_impl)]);
        /// ```
        #[allow(clippy::type_complexity)]
        pub struct TokioFuturePollable {
            state: Arc<RwLock<Option<Result<Option<Vec<u8>>, String>>>>,
        }

        impl TokioFuturePollable {
            /// Spawn a future onto the Tokio runtime and return a `Pollable` that
            /// becomes ready when the future completes.
            ///
            /// The provided `Future` must output `Result<Option<Vec<u8>>, String>`.
            pub fn from_future<F>(fut: F) -> Pin<Box<dyn crate::Pollable>>
            where
                F: Future<Output = Result<Option<Vec<u8>>, String>> + Send + 'static,
            {
                let state = Arc::new(RwLock::new(None));
                let state_cloned = state.clone();
                tokio::spawn(async move {
                    let res = fut.await;
                    *state_cloned.write().unwrap() = Some(res);
                });
                Box::pin(TokioFuturePollable { state })
            }
        }

        #[cfg(all(feature = "std", feature = "tokio-runtime"))] // for documentation
        impl crate::Pollable for TokioFuturePollable {
            fn poll(&self) -> std::task::Poll<Result<Option<Vec<u8>>, String>> {
                match self.state.read().unwrap().as_ref() {
                    Some(r) => std::task::Poll::Ready(r.clone()),
                    None => std::task::Poll::Pending,
                }
            }
        }
    }

    #[cfg(all(feature = "std", feature = "smol-runtime"))]
    pub mod smol {
        //! Smol runtime adapters.
        //!
        //! This module provides a [`Pollable`](crate::Pollable) wrapper that runs a `Future`
        //! on the smol executor and exposes its completion through the poll-based PK Command
        //! interface.
        //!
        //! # Example
        //! ```no_run
        //! use pk_command::{PkHashmapMethod, smol_adapter::SmolFuturePollable};
        //!
        //! let method_accessor = PkHashmapMethod::new(vec![(
        //!     String::from("ECHOO"),
        //!     Box::new(move |param: Option<Vec<u8>>| {
        //!         SmolFuturePollable::from_future(async move {
        //!             // Note the `async` here    ^^^^^
        //!             Ok(param) // Echo back the input as the result
        //!         })
        //!     }),
        //! )]);
        //!
        //! // When using smol, you need to run the code within a smol executor context.
        //! // The async tasks you registered above, when called by the other side,
        //! // will run in the smol executor.
        //! ```
        use std::future::Future;
        use std::pin::Pin;
        use std::sync::{Arc, RwLock};

        /// A `Pollable` adapter that spawns a `Future` onto the smol executor and
        /// exposes its completion through the `Pollable` interface.
        ///
        ///
        #[allow(clippy::type_complexity)]
        pub struct SmolFuturePollable {
            state: Arc<RwLock<Option<Result<Option<Vec<u8>>, String>>>>,
        }

        impl SmolFuturePollable {
            /// Spawn a future onto the smol runtime and return a `Pollable` that
            /// becomes ready when the future completes.
            pub fn from_future<F>(fut: F) -> Pin<Box<dyn crate::Pollable>>
            where
                F: Future<Output = Result<Option<Vec<u8>>, String>> + Send + 'static,
            {
                let state = Arc::new(RwLock::new(None));
                let state_cloned = state.clone();
                // smol::spawn returns a Task which can be detached; detach so it runs in background
                smol::spawn(async move {
                    let res = fut.await;
                    *state_cloned.write().unwrap() = Some(res);
                })
                .detach();
                Box::pin(SmolFuturePollable { state })
            }
        }

        #[cfg(all(feature = "std", feature = "smol-runtime"))]
        impl crate::Pollable for SmolFuturePollable {
            fn poll(&self) -> std::task::Poll<Result<Option<Vec<u8>>, String>> {
                match self.state.read().unwrap().as_ref() {
                    Some(r) => std::task::Poll::Ready(r.clone()),
                    None => std::task::Poll::Pending,
                }
            }
        }
    }
    #[cfg(feature = "embassy-runtime")]
    pub mod embassy {
        //! Embassy runtime adapters.
        //!
        //! This module provides [`Pollable`](crate::Pollable) and [`PkMethodAccessor`](crate::PkMethodAccessor)
        //! helpers that integrate PK Command with the [Embassy](https://embassy.dev/) async runtime.
        //!
        //! See more at the [`embassy_method_accessor!`](crate::embassy_method_accessor) macro.

        // Typically `std` is not available here
        extern crate alloc;
        use alloc::boxed::Box;
        use alloc::string::String;
        use alloc::sync::Arc;
        use alloc::vec::Vec;
        use embassy_sync::once_lock::OnceLock;

        /// A [`Pollable`](crate::Pollable) backed by an Embassy [`OnceLock`].
        ///
        ///
        ///
        /// This is typically created by the [`embassy_method_accessor!`](crate::embassy_method_accessor) macro and
        /// becomes ready when the async task resolves.
        ///
        /// # Example
        /// ```no_run
        /// extern crate alloc;
        /// use alloc::sync::Arc;
        /// use core::task::Poll;
        /// use embassy_sync::once_lock::OnceLock;
        /// use pk_command::embassy_adapter::EmbassyPollable;
        ///
        /// let lock = Arc::new(OnceLock::new());
        /// let pollable = EmbassyPollable(lock);
        /// let _ = pollable; // pass to PK Command state machine
        /// ```
        pub struct EmbassyPollable(pub Arc<OnceLock<Vec<u8>>>);
        impl crate::Pollable for EmbassyPollable {
            fn poll(&self) -> core::task::Poll<Result<Option<Vec<u8>>, String>> {
                match self.0.try_get() {
                    Some(data) => core::task::Poll::Ready(Ok(Some(data.clone()))),
                    None => core::task::Poll::Pending,
                }
            }
        }

        /// Callback type used by Embassy tasks to resolve a method call.
        ///
        ///
        ///
        /// The callback should be invoked **exactly once** with the method's return data. Usually
        /// right after your task is finished.
        ///
        /// # Panics
        ///
        /// Usually you would get this function as the second parameter of the task functions
        /// (i.e. the ones marked with [`#[embassy_executor::task]`](embassy_executor::task))
        /// that you brought into the [`embassy_method_accessor!`](crate::embassy_method_accessor).
        ///
        /// Inside, that function calls [`OnceLock::init()`](embassy_sync::once_lock::OnceLock::init),
        /// right followed by an [`.unwrap()`](core::result::Result::unwrap). So this function panics
        /// **when it is called multiple times**. This usually indicates a tragic logic failure.
        ///
        /// # Example
        /// ```no_run
        /// use embassy_time::Timer;
        /// use pk_command::embassy_adapter::TaskCallback;
        ///
        /// #[embassy_executor::task]
        /// async fn async_echo(param: Vec<u8>, callback: TaskCallback) {
        ///     // You get the callback function^^^^^^^^ here.
        ///
        ///     Timer::after_millis(10).await;
        ///     callback(param);
        /// }
        /// ```
        pub type TaskCallback = Box<dyn Fn(Vec<u8>) + Send>;

        /// Helper macro for creating a [`PkMethodAccessor`](crate::PkMethodAccessor)
        /// backed by Embassy tasks.
        ///
        ///
        ///
        /// # What This Macro Does
        ///
        /// It provides a struct definition and implements the [`PkMethodAccessor`](crate::PkMethodAccessor)
        /// trait for it. The macro accepts several name-function pairs (5-character string literals paired
        /// with async functions) and internally expands them into a simple mapping implementation based on
        /// `match` statements. When invoked by [`PkCommand`](crate::PkCommand), it constructs an [`EmbassyPollable`]
        ///  struct and spawns the provided async task in the Embassy executor.
        ///
        /// # Why a Macro is Needed
        ///
        /// In embedded scenarios, memory and performance are typically constrained, making it wasteful to
        /// maintain a [`HashMap`](std::collections::HashMap) resident in memory. Moreover, for async tasks
        /// in the Embassy environment, the functions generated by [`#[task]`](embassy_executor::task) return
        /// a [`SpawnToken<S>`](embassy_executor::SpawnToken) (where `S` is an opaque type that differs for
        /// each generated task — we only know it implements the [`Sized`] trait). If we followed a `HashMap`-like
        /// approach and used wrappers like `Box` to force them into the same variable, it would introduce unnecessary
        /// and relatively expensive runtime overhead and code complexity.
        ///
        /// However, with a macro, we can simplify the complex hash matching logic into Rust's built-in pattern
        /// matching and hide the differences in [`SpawnToken<S>`](embassy_executor::SpawnToken) generic parameters through
        /// different execution paths (from the compiler's perspective). This not only significantly reduces runtime overhead
        /// but also seamlessly integrates the Embassy environment into PK Command.
        ///
        /// # How to Use This Macro
        ///
        /// You can invoke this macro like:
        ///
        /// ```no_run
        /// # #[macro_use] extern crate pk_command;
        /// # use pk_command::embassy_adapter::TaskCallback;
        /// # use embassy_time::Timer;
        /// #
        /// # #[embassy_executor::task]
        /// # async fn async_task_1(param:Vec<u8>, callback:TaskCallback)
        /// # {
        /// #    // do_something_with(param);
        /// #    callback(param);
        /// # }
        /// # #[embassy_executor::task]
        /// # async fn async_task_2(param:Vec<u8>, callback:TaskCallback)
        /// # {
        /// #    // do_something_with(param);
        /// #    callback(param);
        /// # }
        /// # extern crate alloc;
        /// embassy_method_accessor!(
        ///     MyMethodAccessor,
        ///     ("TASK1", async_task_1),
        ///     ("TASK2", async_task_2)
        /// );
        /// ```
        ///
        /// The macro accepts parameters consisting of an identifier and several tuples (at least one). Specifically:
        ///
        /// - Identifier: The name of the [`PkMethodAccessor`](crate::PkMethodAccessor) struct to be generated.
        /// - Tuples:
        ///   - First element: A 5-character string literal indicating the method name.
        ///   - Second element: A function marked with the [`#[embassy_executor::task]`](embassy_executor::task) macro. This function must have the following form:
        ///
        /// ```no_run
        /// # use pk_command::embassy_adapter::TaskCallback;
        /// #[embassy_executor::task]
        /// async fn async_task (param: Vec<u8>, callback: TaskCallback)
        /// # {}
        /// ```
        ///
        /// See [`TaskCallback`] for more details.
        ///
        /// <div class="warning">
        ///
        /// The macro does not perform compile-time checks for this, but you should still note:
        /// method names must be 5-character strings containing only ASCII characters. If this constraint
        /// is not met, the code will still compile, but your registered methods may not be callable by
        /// PkCommand, which is a serious logical error.
        ///
        /// </div>
        ///
        /// # Complete Example
        ///
        /// ```no_run
        /// # #[macro_use] extern crate pk_command;
        /// use embassy_executor::Spawner;
        /// use embassy_time::Timer;
        /// use pk_command::embassy_adapter::TaskCallback;
        /// use pk_command::{EmbassyInstant, PkCommand, PkCommandConfig, PkHashmapVariable};
        ///
        /// #[embassy_executor::task]
        /// async fn async_task(param: Vec<u8>, callback: TaskCallback) {
        ///     // do_something_with(param);
        ///     callback(param);
        /// }
        ///
        /// // This is required for the macro to work.
        /// extern crate alloc;
        /// pk_command::embassy_method_accessor!(MyMethodAccessor, ("TASK1", async_task));
        ///
        /// #[embassy_executor::task]
        /// async fn pk_command(pk: PkCommand<PkHashmapVariable, MyMethodAccessor, EmbassyInstant>) {
        ///     loop {
        ///         let cmd = pk.poll();
        ///         if let Some(cmd) = cmd {
        ///             // send to the transport layer..
        ///         }
        ///         Timer::after_millis(10).await;
        ///     }
        /// }
        ///
        /// #[embassy_executor::main]
        /// async fn main(spawner: Spawner) {
        ///     let send_spawner = spawner.make_send();
        ///     let ma = MyMethodAccessor::new(send_spawner);
        ///     // Here you can use `ma` as the MethodAccessor of PkCommand
        ///     let pk = PkCommand::<_, _, EmbassyInstant>::new(
        ///         PkCommandConfig::default(64),
        ///         PkHashmapVariable::new(vec![]),
        ///         ma,
        ///     );
        ///     spawner.spawn(pk_command(pk)).unwrap();
        /// }
        /// ```
        #[cfg_attr(docsrs, doc(cfg(feature = "embassy-runtime")))]
        #[macro_export]
        macro_rules! embassy_method_accessor {
            (
                $struct_name: ident,
                $(
                    (
                        $method_name: literal,
                        $function: path
                    )
                )
                , +
            ) => {
                #[derive(Clone, Copy)]
                struct $struct_name
                {
                    spawner: ::embassy_executor::SendSpawner,
                }

                impl ::pk_command::PkMethodAccessor for $struct_name
                {
                    fn call(
                        &self,
                        key: ::alloc::string::String,
                        param: ::alloc::vec::Vec<u8>
                    ) -> ::core::result::Result<::core::pin::Pin<::alloc::boxed::Box<dyn ::pk_command::Pollable>>, ::alloc::string::String>
                    {
                        let lock=::alloc::sync::Arc::new(::embassy_sync::once_lock::OnceLock::new());
                        let lock_clone=lock.clone();
                        let pollable=::alloc::boxed::Box::pin(::pk_command::embassy_adapter::EmbassyPollable(lock_clone));
                        let lock_clone_clone=lock.clone();
                        let callback = ::alloc::boxed::Box::new(move |data: Vec<u8>| {
                            lock_clone_clone.init(data).unwrap();
                        });
                        match key.as_str() {
                            $(
                                $method_name => {
                                    let token=$function(param, callback);
                                    self.spawner.spawn(token)
                                        .map_err(|x| x.to_string())?;
                                    Ok(pollable)
                                },
                            )*
                            _ => {
                                let mut err_msg = ::alloc::string::String::from("No method named ");
                                err_msg.push_str(&key);
                                err_msg.push_str(" found");
                                Err(err_msg)
                            }
                        }
                    }
                }
                impl $struct_name
                {
                    fn new(spawner: ::embassy_executor::SendSpawner) -> Self
                    {
                        Self {spawner}
                    }
                }
            };
        }
    }
}

/// Type alias for a variable change listener function.
/// This function takes a `Vec<u8>` as input and returns nothing.
/// It is used in `PkHashmapVariable`, and is called whenever a variable is updated.
///
///
#[cfg(feature = "std")]
pub type VariableChangeListener = Box<dyn Fn(Vec<u8>)>;

/// A wrapper for `std::collections::HashMap` that implements the `PkVariableAccessor` trait.
///
///
///
/// This implementation provides internal mutability and a listener mechanism for variable updates.
///
/// **Note**: This is only available when the `std` feature is enabled.
#[cfg(feature = "std")]
pub struct PkHashmapVariable {
    hashmap: std::collections::HashMap<String, (RefCell<Vec<u8>>, VariableChangeListener)>,
}

#[cfg(feature = "std")]
impl crate::PkVariableAccessor for PkHashmapVariable {
    fn get(&self, key: String) -> Option<Vec<u8>> {
        self.hashmap.get(&key).map(|v| v.0.borrow().clone())
    }
    fn set(&self, key: String, value: Vec<u8>) -> Result<(), String> {
        if self.hashmap.contains_key(&key) {
            let v = self.hashmap.get(&key).unwrap();
            v.0.replace(value.clone());
            v.1(value);
            Ok(())
        } else {
            Err(String::from("Key not found"))
        }
    }
}
#[cfg(feature = "std")]
impl PkHashmapVariable {
    /// Creates a new `PkHashmapVariable` instance.
    ///
    /// # Arguments
    /// * `init_vec`: A vector of tuples, where each tuple contains:
    ///     - `String`: The variable key.
    ///     - `Option<Vec<u8>>`: The initial value of the variable. Defaults to an empty `Vec<u8>` if `None`.
    ///     - `VariableChangeListener`: A listener function called when the variable is set.
    ///
    /// **IMPORTANT**: The listener is executed synchronously in the same thread as `PkCommand::poll()`.
    /// If the listener performs heavy computation, it may block the protocol state machine.
    pub fn new(init_vec: Vec<(String, Option<Vec<u8>>, VariableChangeListener)>) -> Self {
        let mut hashmap = std::collections::HashMap::new();
        for i in init_vec.into_iter() {
            let (key, value, listener) = i;
            hashmap.insert(key, (RefCell::new(value.unwrap_or_default()), listener));
        }
        PkHashmapVariable { hashmap }
    }
}

/// Type alias for a method implementation function.
/// This function takes an optional [`Vec<u8>`] as input and returns a pinned [`Pollable`](crate::Pollable).
/// It is used in `PkHashmapMethod` to define method behaviors.
///
///
#[cfg(feature = "std")]
pub type MethodImplementation = Box<dyn Fn(Option<Vec<u8>>) -> Pin<Box<dyn crate::Pollable>>>;

/// A wrapper for `std::collections::HashMap` that implements the [`PkMethodAccessor`](crate::PkMethodAccessor) trait.
///
///
///
/// **Note**: This is only available when the `std` feature is enabled.
///
/// # Note for Method Implementation type
///
/// The type `Box<dyn Fn(Option<Vec<u8>>) -> Pin<Box<dyn Pollable>>>` (as appeared in the second field of the tuple that [`new()`](PkHashmapMethod::new) accepts) is a boxed closure
/// that takes an optional [`Vec<u8>`] as input, and returns a pinned [`Pollable`](crate::Pollable).
/// This allows method implementations to perform background work and return a [`Pollable`](crate::Pollable) that becomes ready when the work is complete.
///
/// The library provides some helper types for the [`Pollable`](crate::Pollable) trait:
/// - Sync, based on threads: [`PkPromise`].
/// - Async with [Tokio](https://tokio.rs/): [`TokioFuturePollable`](crate::tokio_adapter::TokioFuturePollable).
/// - Async with [Smol](https://github.com/smol-rs/smol): [`SmolFuturePollable`](crate::smol_adapter::SmolFuturePollable).
///
/// And simply boxing them should work.
///
/// # Example with [`PkPromise`]
/// ```
/// use pk_command::{PkHashmapMethod, PkPromise};
/// let methods = PkHashmapMethod::new(vec![(
///     String::from("LONGT"),
///     Box::new(|param| {
///         PkPromise::execute(|resolve| {
///             // do_something_with(param);
///             resolve(b"task complete".to_vec());
///         })
///     }),
/// )]);
/// ```
#[cfg(feature = "std")]
pub struct PkHashmapMethod {
    hashmap: std::collections::HashMap<String, MethodImplementation>,
}

#[cfg(feature = "std")]
impl crate::PkMethodAccessor for PkHashmapMethod {
    fn call(&self, key: String, param: Vec<u8>) -> Result<Pin<Box<dyn crate::Pollable>>, String> {
        if self.hashmap.contains_key(&key) {
            let f = self.hashmap.get(&key).unwrap();
            Ok(f(Some(param)))
        } else {
            Err(String::from("Method not found"))
        }
    }
}

#[cfg(feature = "std")]
impl PkHashmapMethod {
    /// Creates a new `PkHashmapMethod` instance.
    ///
    /// # Arguments
    /// * `init_vec`: A vector of tuples containing method keys and their corresponding implementation closures.
    pub fn new(init_vec: Vec<(String, MethodImplementation)>) -> Self {
        let mut hashmap = std::collections::HashMap::new();
        for i in init_vec.into_iter() {
            let (key, method) = i;
            hashmap.insert(key, method);
        }
        PkHashmapMethod { hashmap }
    }
}

/// A simple implementation of `Pollable` that executes tasks in a background thread.
///
///
///
/// This is similar to a JavaScript Promise. It allows offloading work to another thread
/// and polling for its completion within the PK protocol's `poll()` cycle.
///
/// **Note**: This is only available when the `std` feature is enabled.
#[derive(Clone)]
#[cfg(feature = "std")]
pub struct PkPromise {
    return_value: Arc<RwLock<Option<Vec<u8>>>>,
}
#[cfg(feature = "std")]
impl PkPromise {
    /// Executes a closure in a new thread and returns a `Pollable` handle.
    ///
    /// The provided closure `function` receives a `resolve` callback. Call `resolve(data)`
    /// when the task is complete to make the data available.
    ///
    /// # Arguments
    /// * `function`: A closure that performs the task and calls the provided `resolve` callback.
    ///
    /// # Example
    /// ```
    /// use pk_command::PkPromise;
    /// let promise = PkPromise::execute(|resolve| {
    ///     // Do expensive work...
    ///     resolve(b"done".to_vec());
    /// });
    /// ```
    ///
    /// # Example with PK Command
    /// ```
    /// use pk_command::{PkCommand, PkCommandConfig, PkHashmapMethod, PkHashmapVariable, PkPromise};
    /// let vars = PkHashmapVariable::new(vec![]);
    /// let methods = PkHashmapMethod::new(vec![(
    ///     "LONGT".to_string(),
    ///     Box::new(|param| {
    ///         PkPromise::execute(|resolve| {
    ///             // do_something_with(param);
    ///             resolve(b"task complete".to_vec());
    ///         })
    ///     }),
    /// )]);
    /// let config = PkCommandConfig::default(64);
    /// let pk = PkCommand::<_, _, std::time::Instant>::new(config, vars, methods);
    /// // main loop...
    /// ```
    #[cfg(feature = "std")]
    pub fn execute<T>(function: T) -> Pin<Box<Self>>
    where
        T: FnOnce(Box<dyn FnOnce(Vec<u8>) + Send + 'static>) + Send + 'static,
    {
        let return_value_arc = Arc::new(RwLock::new(None));
        let return_value_clone = return_value_arc.clone();
        std::thread::spawn(move || {
            let resolve: Box<dyn FnOnce(Vec<u8>) + Send + 'static> =
                Box::new(move |ret: Vec<u8>| {
                    // This resolve function is called by the user's function
                    *return_value_clone.write().unwrap() = Some(ret);
                });
            function(Box::new(resolve));
        });
        Box::pin(PkPromise {
            return_value: return_value_arc,
        })
    }
}
#[cfg(feature = "std")]
impl crate::Pollable for PkPromise {
    fn poll(&self) -> std::task::Poll<Result<Option<Vec<u8>>, String>> {
        let read_guard = self.return_value.read().unwrap();
        match read_guard.as_ref() {
            Some(data) => std::task::Poll::Ready(Ok(Some(data.clone()))),
            None => std::task::Poll::Pending,
        }
    }
}
