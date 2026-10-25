// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A module's custom section, read from its bytes without compiling it.

/// An unsigned LEB128 of at most 32 bits at `at`, and where it ends.
fn u32_at(bytes: &[u8], mut at: usize) -> Option<(u32, usize)> {
    let mut value: u64 = 0;
    for shift in (0..35).step_by(7) {
        let byte = *bytes.get(at)?;
        at += 1;
        value |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return u32::try_from(value).ok().map(|v| (v, at));
        }
    }
    None
}

/// Why a module's bytes could not be read for a section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SectionError {
    /// Not a WebAssembly module (a component, or not wasm at all).
    NotAModule,
    /// The bytes end inside a section.
    Truncated,
}

/// The contents of the first custom section named `name`.
pub(crate) fn custom<'a>(bytes: &'a [u8], name: &str) -> Result<Option<&'a [u8]>, SectionError> {
    if bytes.get(..8) != Some(&b"\0asm\x01\0\0\0"[..]) {
        return Err(SectionError::NotAModule);
    }
    let mut at = 8;
    while at < bytes.len() {
        let id = bytes[at];
        let (size, body) = u32_at(bytes, at + 1).ok_or(SectionError::Truncated)?;
        let end = body
            .checked_add(size as usize)
            .filter(|end| *end <= bytes.len())
            .ok_or(SectionError::Truncated)?;
        if id == 0 {
            let (len, text) = u32_at(bytes, body).ok_or(SectionError::Truncated)?;
            let contents = text
                .checked_add(len as usize)
                .filter(|start| *start <= end)
                .ok_or(SectionError::Truncated)?;
            if &bytes[text..contents] == name.as_bytes() {
                return Ok(Some(&bytes[contents..end]));
            }
        }
        at = end;
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn module(sections: &[(u8, &[u8])]) -> Vec<u8> {
        let mut out = b"\0asm\x01\0\0\0".to_vec();
        for (id, body) in sections {
            out.push(*id);
            out.push(body.len() as u8);
            out.extend_from_slice(body);
        }
        out
    }

    fn custom_body(name: &str, contents: &[u8]) -> Vec<u8> {
        let mut body = vec![name.len() as u8];
        body.extend_from_slice(name.as_bytes());
        body.extend_from_slice(contents);
        body
    }

    #[test]
    fn a_custom_section_is_found_among_the_others() {
        let version = custom_body("guatiao.version", b"1.2.0");
        let other = custom_body("name", b"\0\0");
        let bytes = module(&[(1, &[0]), (0, &other), (0, &version)]);
        assert_eq!(custom(&bytes, "guatiao.version"), Ok(Some(&b"1.2.0"[..])));
        assert_eq!(custom(&bytes, "missing"), Ok(None));
    }

    #[test]
    fn what_is_not_a_module_is_refused_without_panicking() {
        assert_eq!(
            custom(b"\0asm\x0d\0\x01\0", "x"),
            Err(SectionError::NotAModule)
        );
        assert_eq!(custom(b"hello", "x"), Err(SectionError::NotAModule));
        // A section longer than the file, a name longer than its section,
        // a size that never ends.
        let mut long = module(&[]);
        long.extend_from_slice(&[0, 9, 1]);
        assert_eq!(custom(&long, "x"), Err(SectionError::Truncated));
        let mut name = module(&[]);
        name.extend_from_slice(&[0, 2, 5, b'a']);
        assert_eq!(custom(&name, "x"), Err(SectionError::Truncated));
        let mut endless = module(&[]);
        endless.extend_from_slice(&[0, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff]);
        assert_eq!(custom(&endless, "x"), Err(SectionError::Truncated));
    }
}
