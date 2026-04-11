pub fn filename(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        if is_unsafe_filename_char(ch) {
            out.push('_');
        } else {
            out.push(ch);
        }
    }
    let trimmed = out.trim();
    if trimmed.is_empty() {
        "Untitled".to_string()
    } else {
        trimmed.to_string()
    }
}

fn is_unsafe_filename_char(ch: char) -> bool {
    matches!(ch, '/' | '\\' | ':' | '\0')
        || ch.is_control()
        || matches!(ch, '\u{2028}' | '\u{2029}')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replaces_path_unsafe_characters() {
        assert_eq!(filename("a/b:c\\d\0e"), "a_b_c_d_e");
    }

    #[test]
    fn preserves_readable_unicode() {
        assert_eq!(filename("録音 メモ"), "録音 メモ");
    }

    #[test]
    fn falls_back_for_empty_names() {
        assert_eq!(filename("   "), "Untitled");
    }
}
