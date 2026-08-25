//! Small JVM-compatible descriptor splitter used by DEX table construction.

pub(super) fn method_parts(descriptor: &str) -> Result<(Vec<String>, String), String> {
    let bytes = descriptor.as_bytes();
    if bytes.first() != Some(&b'(') {
        return Err("method descriptor does not start with `(`".to_owned());
    }
    let mut cursor = 1;
    let mut parameters = Vec::new();
    while bytes.get(cursor) != Some(&b')') {
        if cursor >= bytes.len() {
            return Err("method descriptor lacks `)`".to_owned());
        }
        let end = type_end(descriptor, cursor, false)?;
        parameters.push(descriptor[cursor..end].to_owned());
        cursor = end;
    }
    cursor += 1;
    let end = type_end(descriptor, cursor, true)?;
    if end != bytes.len() {
        return Err("method descriptor has trailing data".to_owned());
    }
    Ok((parameters, descriptor[cursor..end].to_owned()))
}

pub(super) fn field_type_valid(descriptor: &str) -> bool {
    type_end(descriptor, 0, false).is_ok_and(|end| end == descriptor.len())
}

pub(super) fn register_words(descriptor: &str) -> u16 {
    if matches!(descriptor.as_bytes().first(), Some(b'J' | b'D')) {
        2
    } else {
        1
    }
}

fn type_end(descriptor: &str, start: usize, allow_void: bool) -> Result<usize, String> {
    let bytes = descriptor.as_bytes();
    let Some(&head) = bytes.get(start) else {
        return Err("truncated type descriptor".to_owned());
    };
    match head {
        b'V' if allow_void => Ok(start + 1),
        b'B' | b'C' | b'D' | b'F' | b'I' | b'J' | b'S' | b'Z' => Ok(start + 1),
        b'L' => bytes[start + 1..]
            .iter()
            .position(|&byte| byte == b';')
            .filter(|&length| length != 0)
            .map(|length| start + length + 2)
            .ok_or_else(|| "unterminated or empty object descriptor".to_owned()),
        b'[' => {
            let mut component = start + 1;
            while bytes.get(component) == Some(&b'[') {
                component += 1;
            }
            type_end(descriptor, component, false)
        }
        _ => Err("invalid type descriptor tag".to_owned()),
    }
}
