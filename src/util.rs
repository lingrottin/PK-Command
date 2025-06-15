use crate::Pollable;
use std::{
    cell::RefCell,
    pin::Pin,
    sync::{Arc, RwLock},
};

/// Module for handling PK Command Message IDs.
pub mod msg_id {
    const BASE: u16 = 94;
    const OFFSET: u8 = b'!';
    const MAX_ID: u16 = BASE * BASE - 1;

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

        if !(c1 >= b'!' && c1 <= b'~' && c2 >= b'!' && c2 <= b'~') {
            return Err("Input string contains invalid characters.");
        }

        let val1 = (c1 - OFFSET) as u16;
        let val2 = (c2 - OFFSET) as u16;

        Ok(val1 * BASE + val2)
    }

    /// Converts a u16 integer ID back into its 2-character string representation.
    ///
    /// The ID must be within the valid range (0 to 8835, inclusive).
    ///
    /// # Arguments
    /// * `id`: The u16 integer ID to convert.
    ///
    /// # Returns
    /// A `Result` containing the 2-character string ID, or an error message if the ID is out of range.
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
    ///
    /// # Arguments
    /// * `id`: The current u16 message ID.
    ///
    /// # Returns
    /// The next message ID.
    pub fn increment(id: u16) -> u16 {
        (id + 1) % (MAX_ID + 1)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

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

/// A wrapper for `std::collections::HashMap` that implements the `PkVariableAccessor` trait.
pub struct PkHashmapVariable {
    // 这是一个实现了 PkVariableAccessor trait 的 Hashmap 包装器，
    // 基于 std 的 Hashmap 和 RefCell 类型，实现内部可变性和变量更改时的监听
    hashmap: std::collections::HashMap<String, (RefCell<Vec<u8>>, Box<dyn Fn(Vec<u8>) -> ()>)>,
}
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
impl PkHashmapVariable {
    /// Creates a new `PkHashmapVariable` instance.
    ///
    /// # Arguments
    /// * `init_vec`: A vector of tuples, where each tuple contains:
    ///     - `String`: The variable key.
    ///     - `Option<Vec<u8>>`: The initial value of the variable. Defaults to an empty `Vec<u8>` if `None`.
    ///     - `Box<dyn Fn(Vec<u8>) -> ()>`: A listener function called when the variable is set.
    ///
    /// **IMPORTANT**: The listener passed in here is synchronously executed and may block the main thread (where `PkCommand::poll()` is executed).
    /// Use with caution or create a new thread if the listener is going to do something very costly.
    pub fn new(init_vec: Vec<(String, Option<Vec<u8>>, Box<dyn Fn(Vec<u8>) -> ()>)>) -> Self {
        let mut hashmap = std::collections::HashMap::new();
        for i in init_vec.into_iter() {
            let (key, value, listener) = i;
            hashmap.insert(key, (RefCell::new(value.unwrap_or_default()), listener));
        }
        PkHashmapVariable { hashmap }
    }
}

/// A wrapper for `std::collections::HashMap` that implements the `PkMethodAccessor` trait.
pub struct PkHashmapMethod {
    // 这是一个实现了 PkMethodAccessor trait 的 Hashmap 包装器，
    hashmap:
        std::collections::HashMap<String, Box<dyn Fn(Option<Vec<u8>>) -> Pin<Box<dyn Pollable>>>>,
}

impl crate::PkMethodAccessor for PkHashmapMethod {
    fn call(&self, key: String, param: Vec<u8>) -> Result<Pin<Box<dyn Pollable>>, String> {
        if self.hashmap.contains_key(&key) {
            let f = self.hashmap.get(&key).unwrap();
            Ok(f(Some(param)))
        } else {
            Err(String::from("Method not found"))
        }
    }
}

impl PkHashmapMethod {
    /// Creates a new `PkMHashmapWrapper` instance.
    ///
    /// # Arguments
    /// * `init_vec`: A vector of tuples, where each tuple contains:
    ///     - `String`: The method key (name).
    ///     - `Box<dyn Fn(Option<Vec<u8>>) -> Pin<Box<dyn Pollable>> `:
    ///       A function that takes optional parameters and returns a pinned, boxed `Pollable` representing the method call.
    pub fn new(
        init_vec: Vec<(
            String,
            Box<dyn Fn(Option<Vec<u8>>) -> Pin<Box<dyn Pollable>>>,
        )>,
    ) -> Self {
        let mut hashmap = std::collections::HashMap::new();
        for i in init_vec.into_iter() {
            let (key, method) = i;
            hashmap.insert(key, method);
        }
        PkHashmapMethod { hashmap }
    }
}

/// A simple implementation of `Pollable` based on multi-thread.
///
/// This is like `Promise` in JavaScript.
#[derive(Clone)]
pub struct PkPromise {
    return_value: Arc<RwLock<Option<Vec<u8>>>>,
}
impl PkPromise {
    /// Creates a new `PkPromise` and executes a function in a new thread.
    ///
    /// The provided function `function` will be executed in a separate thread.
    /// It receives a `resolve` closure as an argument. The user's function
    /// should call this `resolve` closure with the result data (`Vec<u8>`)
    /// when the asynchronous operation is complete.
    ///
    /// # Arguments
    /// * `function`: A closure that takes a `resolve` closure and performs the
    ///   asynchronous task. The `resolve` closure should be called with the
    ///   result data when the task finishes successfully.
    ///
    /// # Returns
    /// A new `PkPromise` instance that can be polled to check the status
    /// of the asynchronous operation.
    pub fn execute<T>(function: T) -> Pin<Box<Self>>
    where
        T: FnOnce(Box<dyn FnOnce(Vec<u8>) -> () + Send + 'static>) + Send + 'static,
    {
        let return_value_arc = Arc::new(RwLock::new(None));
        let return_value_clone = return_value_arc.clone();
        std::thread::spawn(move || {
            let resolve: Box<dyn FnOnce(Vec<u8>) -> () + Send + 'static> =
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
impl Pollable for PkPromise {
    fn poll(&self) -> std::task::Poll<Result<Option<Vec<u8>>, String>> {
        let read_guard = self.return_value.read().unwrap();
        match read_guard.as_ref() {
            Some(data) => std::task::Poll::Ready(Ok(Some(data.clone()))),
            None => std::task::Poll::Pending,
        }
    }
}
