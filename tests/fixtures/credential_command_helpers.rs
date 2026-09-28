#[cfg(windows)]
pub fn encoded_powershell(expression: &str) -> String {
    let bytes: Vec<u8> = expression
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect();
    let alphabet = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut encoded = String::new();
    for part in bytes.chunks(3) {
        let bits = (u32::from(part[0]) << 16)
            | (u32::from(*part.get(1).unwrap_or(&0)) << 8)
            | u32::from(*part.get(2).unwrap_or(&0));
        for offset in (0..4).rev() {
            encoded.push(if 3 - offset > part.len() {
                '='
            } else {
                alphabet[((bits >> (offset * 6)) & 63) as usize] as char
            });
        }
    }
    format!("powershell.exe -NoProfile -NonInteractive -EncodedCommand {encoded}")
}
