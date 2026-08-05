use saya_types::ConnectionError;

use super::errors;

pub(crate) fn token(value: &str) -> Result<String, ConnectionError> {
    let mut found = None;
    for pair in value.split('&') {
        let Some((key, value)) = pair.split_once('=') else {
            continue;
        };
        if decode(key)? == "token" {
            if found.is_some() {
                return Err(errors::auth());
            }
            found = Some(decode(value)?);
        }
    }
    found
        .filter(|value| !value.is_empty())
        .ok_or_else(errors::auth)
}

fn decode(value: &str) -> Result<String, ConnectionError> {
    let mut bytes = Vec::with_capacity(value.len());
    let input = value.as_bytes();
    let mut index = 0;
    while index < input.len() {
        match input[index] {
            b'+' => bytes.push(b' '),
            b'%' if index + 2 < input.len() => {
                let hex = std::str::from_utf8(&input[index + 1..index + 3])
                    .map_err(|_| errors::auth())?;
                bytes.push(u8::from_str_radix(hex, 16).map_err(|_| errors::auth())?);
                index += 2;
            }
            b'%' => return Err(errors::auth()),
            item => bytes.push(item),
        }
        index += 1;
    }
    String::from_utf8(bytes).map_err(|_| errors::auth())
}
