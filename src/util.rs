use std::{cell::RefCell, pin::Pin};

pub mod msg_id {
    const BASE: u16 = 94;
    const OFFSET: u8 = b'!';
    const MAX_ID: u16 = BASE * BASE - 1;

    pub fn to_u16(id_str: &str) -> Result<u16, &'static str> {
        if id_str.len() != 2 {
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

    pub fn increment(id: u16) -> u16 {
        (id + 1) % (MAX_ID + 1)
    }
}

/// A wrapper for std::Hashmap which inplements PkVariableAccessor
pub struct PkVHashmapWrapper {
    // 这是一个实现了 PkVariableAccessor trait 的 Hashmap 包装器，
    // 基于 std 的 Hashmap 和 RefCell 类型，实现内部可变性和变量更改时的监听
    hashmap: std::collections::HashMap<String, (RefCell<Vec<u8>>, Box<dyn Fn(Vec<u8>) -> ()>)>,
}
impl crate::PkVariableAccessor for PkVHashmapWrapper {
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
impl PkVHashmapWrapper {
    /// Creates a new PkHashmapWrapper instance. Where: init_vec: (key, value, listener)
    pub fn new(init_vec: Vec<(String, Option<Vec<u8>>, Box<dyn Fn(Vec<u8>) -> ()>)>) -> Self {
        let mut hashmap = std::collections::HashMap::new();
        for i in init_vec.into_iter() {
            let (key, value, listener) = i;
            hashmap.insert(key, (RefCell::new(value.unwrap_or_default()), listener));
        }
        PkVHashmapWrapper { hashmap }
    }
}

/// A wrapper for std::Hashmap which inplements PkMethodAccessor
pub struct PkMHashmapWrapper {
    // 这是一个实现了 PkMethodAccessor trait 的 Hashmap 包装器，
    hashmap: std::collections::HashMap<
        String,
        Box<
            dyn Fn(
                Option<Vec<u8>>,
            ) -> Pin<Box<dyn Future<Output = Result<Option<Vec<u8>>, String>>>>,
        >,
    >,
}

impl crate::PkMethodAccessor for PkMHashmapWrapper {
    fn call(
        &self,
        key: String,
        param: Vec<u8>,
    ) -> Result<Pin<Box<dyn Future<Output = Result<Option<Vec<u8>>, String>>>>, String> {
        if self.hashmap.contains_key(&key) {
            let f = self.hashmap.get(&key).unwrap();
            Ok(f(Some(param)))
        } else {
            Err(String::from("Method not found"))
        }
    }
}

impl PkMHashmapWrapper {
    pub fn new(
        init_vec: Vec<(
            String,
            Box<
                dyn Fn(
                    Option<Vec<u8>>,
                )
                    -> Pin<Box<dyn Future<Output = Result<Option<Vec<u8>>, String>>>>,
            >,
        )>,
    ) -> Self {
        let mut hashmap = std::collections::HashMap::new();
        for i in init_vec.into_iter() {
            let (key, method) = i;
            hashmap.insert(key, method);
        }
        PkMHashmapWrapper { hashmap }
    }
}
