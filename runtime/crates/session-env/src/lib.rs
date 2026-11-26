//! Locale policy shared by Husk session entry points.
//!
//! GNU gettext gives `LANGUAGE` precedence over `LANG`. A Chinese default must
//! therefore set both values together and must not synthesize a Chinese
//! `LANGUAGE` when the caller explicitly selected English.

pub fn choose_session_locale(
    caller_lang: Option<&str>,
    caller_language: Option<&str>,
    has_zh_locale: bool,
) -> (String, String) {
    let lang = match caller_lang {
        Some(v) if !matches!(v, "" | "POSIX" | "C" | "C.UTF-8") => v.to_string(),
        _ if has_zh_locale => "zh_CN.UTF-8".to_string(),
        _ => "C.UTF-8".to_string(),
    };
    let language = match caller_language {
        Some(v) if !v.is_empty() => v.to_string(),
        _ if lang == "zh_CN.UTF-8" => "zh_CN:zh".to_string(),
        _ => String::new(),
    };
    (lang, language)
}

#[cfg(test)]
mod tests {
    use super::choose_session_locale;

    #[test]
    fn defaults_to_chinese_when_available() {
        assert_eq!(
            choose_session_locale(None, None, true),
            ("zh_CN.UTF-8".into(), "zh_CN:zh".into())
        );
    }

    #[test]
    fn preserves_explicit_english_without_gettext_override() {
        assert_eq!(
            choose_session_locale(Some("en_US.UTF-8"), None, true),
            ("en_US.UTF-8".into(), String::new())
        );
    }

    #[test]
    fn preserves_explicit_language() {
        assert_eq!(
            choose_session_locale(Some("en_US.UTF-8"), Some("en_US:en"), true),
            ("en_US.UTF-8".into(), "en_US:en".into())
        );
    }

    #[test]
    fn falls_back_without_chinese_archive() {
        assert_eq!(
            choose_session_locale(None, None, false),
            ("C.UTF-8".into(), String::new())
        );
    }
}
