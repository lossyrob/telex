const NEW_MESSAGE_WAV: &[u8] = include_bytes!("../sounds/telex-new-msg-1.wav");

#[cfg(target_os = "windows")]
pub fn play_new_message() -> Result<(), String> {
    use windows::core::PCWSTR;
    use windows::Win32::Media::Audio::{
        PlaySoundW, SND_ASYNC, SND_MEMORY, SND_NODEFAULT, SND_NOSTOP,
    };

    let played = unsafe {
        PlaySoundW(
            PCWSTR::from_raw(NEW_MESSAGE_WAV.as_ptr().cast()),
            None,
            SND_ASYNC | SND_MEMORY | SND_NODEFAULT | SND_NOSTOP,
        )
    };
    if played.as_bool() {
        Ok(())
    } else {
        Err("Windows PlaySoundW rejected the embedded new-message WAV".into())
    }
}

#[cfg(not(target_os = "windows"))]
pub fn play_new_message() -> Result<(), String> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_new_message_sound_is_a_wave_file() {
        assert!(NEW_MESSAGE_WAV.len() > 12);
        assert_eq!(&NEW_MESSAGE_WAV[..4], b"RIFF");
        assert_eq!(&NEW_MESSAGE_WAV[8..12], b"WAVE");
    }
}
