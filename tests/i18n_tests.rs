use obsid_ambilight::config::Language;
use obsid_ambilight::i18n::{self, Lang};

#[test]
fn auto_follows_windows_ui_language() {
    assert_eq!(i18n::resolve(Language::Auto, 0x0419), Lang::Ru, "ru-RU");
    assert_eq!(i18n::resolve(Language::Auto, 0x0819), Lang::Ru, "ru-MD shares the primary id");
    assert_eq!(i18n::resolve(Language::Auto, 0x0409), Lang::En, "en-US");
    assert_eq!(i18n::resolve(Language::Auto, 0x0422), Lang::En, "uk-UA is not Russian");
    assert_eq!(i18n::resolve(Language::Auto, 0), Lang::En);
}

#[test]
fn explicit_choice_wins() {
    assert_eq!(i18n::resolve(Language::Ru, 0x0409), Lang::Ru);
    assert_eq!(i18n::resolve(Language::En, 0x0419), Lang::En);
}

#[test]
fn fill_replaces_placeholders_in_order() {
    assert_eq!(i18n::fill("{} of {}", &["1", "2"]), "1 of 2");
    assert_eq!(i18n::fill("no args", &["x"]), "no args");
    assert_eq!(i18n::fill("{} and {}", &["only"]), "only and {}");
}

#[test]
fn tables_differ_and_are_filled() {
    let ru = i18n::strings(Lang::Ru);
    let en = i18n::strings(Lang::En);
    assert_ne!(ru.tab_device, en.tab_device);
    assert!(ru.status_connected.contains("{}"));
    assert!(en.status_connected.contains("{}"));
    assert!(!ru.exit_item.is_empty() && !en.exit_item.is_empty());
}
