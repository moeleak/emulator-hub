use crate::preferences::Language;

pub fn tr(language: Language, zh: &'static str, en: &'static str) -> &'static str {
    match language {
        Language::Chinese => zh,
        Language::English => en,
    }
}
