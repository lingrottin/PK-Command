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
