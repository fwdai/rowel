use crate::app::{APP_NAME, WEBSITE};
use crate::{autolock, locale, settings, window};
use tauri::image::Image;
use tauri::menu::{MenuBuilder, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Manager, Wry};
use tauri_plugin_opener::OpenerExt;

// Monochrome asterisk glyph, generated from icons/tray.svg by `bun run icons`.
// Rendered as a template image on macOS so the menu bar tints it for
// light/dark appearance; other platforms show the PNG as-is.
const TRAY_ICON: &[u8] = include_bytes!("../icons/tray/72x72.png");

/// The four labels, in one language.
///
/// `show` carries a `{}` where the app's name goes, because word order differs:
/// "Open Rowel", but "Rowel öffnen".
struct Labels {
    show: &'static str,
    lock: &'static str,
    about: &'static str,
    quit: &'static str,
}

/// The tray in every language the app ships a catalogue for.
///
/// A copy of the "Open App", "Lock vault", "About" and "Quit" entries of
/// `src/i18n/locales/*.json`, kept here because the menu is built before any
/// webview exists and lives on after the window is closed — i18next is in the
/// other process and cannot be asked. Reword one and reword the other; the
/// tags are `locale::SUPPORTED`, en-US first so it can double as the fallback.
const LABELS: [(&str, Labels); 10] = [
    (
        "en-US",
        Labels {
            show: "Open {}",
            lock: "Lock vault",
            about: "About",
            quit: "Quit",
        },
    ),
    (
        "de-DE",
        Labels {
            show: "{} öffnen",
            lock: "Tresor sperren",
            about: "Über",
            quit: "Beenden",
        },
    ),
    (
        "fr-FR",
        Labels {
            show: "Ouvrir {}",
            lock: "Verrouiller le coffre",
            about: "À propos",
            quit: "Quitter",
        },
    ),
    (
        "pl-PL",
        Labels {
            show: "Otwórz {}",
            lock: "Zablokuj sejf",
            about: "O aplikacji",
            quit: "Zakończ",
        },
    ),
    (
        "pt-BR",
        Labels {
            show: "Abrir {}",
            lock: "Bloquear cofre",
            about: "Sobre",
            quit: "Sair",
        },
    ),
    (
        "ru-RU",
        Labels {
            show: "Открыть {}",
            lock: "Заблокировать хранилище",
            about: "О программе",
            quit: "Выход",
        },
    ),
    (
        "sv-SE",
        Labels {
            show: "Öppna {}",
            lock: "Lås valvet",
            about: "Om",
            quit: "Avsluta",
        },
    ),
    (
        "tr-TR",
        Labels {
            show: "{} uygulamasını aç",
            lock: "Kasayı kilitle",
            about: "Hakkında",
            quit: "Çıkış",
        },
    ),
    (
        "uk-UA",
        Labels {
            show: "Відкрити {}",
            lock: "Замкнути сховище",
            about: "Про програму",
            quit: "Вийти",
        },
    ),
    (
        "zh-CN",
        Labels {
            show: "打开 {}",
            lock: "锁定保险库",
            about: "关于",
            quit: "退出",
        },
    ),
];

/// The labels for the language the UI is in — the explicit choice when there is
/// one, the OS otherwise, exactly as the webview resolves it.
fn labels(preferred: Option<&str>) -> &'static Labels {
    let tag = locale::resolve_preferred(preferred);
    LABELS
        .iter()
        .find(|(shipped, _)| *shipped == tag)
        .map_or(&LABELS[0].1, |(_, labels)| labels)
}

/// The items, kept so a language change can relabel them rather than rebuild
/// the tray — replacing the icon would make it jump to the end of the menu bar.
struct TrayItems {
    show: MenuItem<Wry>,
    lock: MenuItem<Wry>,
    about: MenuItem<Wry>,
    quit: MenuItem<Wry>,
}

// Tray menu mirrors legacy tray/index.js.
pub fn create(app: &AppHandle) -> tauri::Result<()> {
    let labels = labels(settings::current(app).locale.as_deref());
    let show = labels.show.replace("{}", APP_NAME);
    let item = |id: &str, text: &str| MenuItem::with_id(app, id, text, true, None::<&str>);
    let items = TrayItems {
        show: item("show", &show)?,
        lock: item("lock", labels.lock)?,
        about: item("about", labels.about)?,
        quit: item("quit", labels.quit)?,
    };

    let menu = MenuBuilder::new(app)
        .item(&items.show)
        .item(&items.lock)
        .separator()
        .item(&items.about)
        .separator()
        .item(&items.quit)
        .build()?;

    TrayIconBuilder::new()
        .icon(Image::from_bytes(TRAY_ICON)?)
        .icon_as_template(true)
        .tooltip(APP_NAME)
        .menu(&menu)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "show" => window::show(app),
            "lock" => autolock::lock(app),
            "about" => {
                let _ = app.opener().open_url(WEBSITE, None::<&str>);
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .build(app)?;

    app.manage(items);
    Ok(())
}

/// Put the menu back in the user's language after Settings › Language changes
/// it. A no-op before the tray exists, so an early `settings::set` is harmless.
///
/// A label the platform refuses to change is logged, not propagated: the
/// preference is the user's and has to be saved either way, and the tray is
/// the only thing that could be left behind — the next language change, or the
/// next launch (`create` reads the same table), relabels it again.
pub fn relabel(app: &AppHandle, preferred: Option<&str>) {
    let Some(items) = app.try_state::<TrayItems>() else {
        return;
    };
    let labels = labels(preferred);
    let updates = [
        (
            "show",
            items.show.set_text(labels.show.replace("{}", APP_NAME)),
        ),
        ("lock", items.lock.set_text(labels.lock)),
        ("about", items.about.set_text(labels.about)),
        ("quit", items.quit.set_text(labels.quit)),
    ];
    for (item, result) in updates {
        if let Err(error) = result {
            log::warn!(
                "tray item '{item}' kept its previous label; could not relabel for {}: {error}",
                preferred.unwrap_or("the OS locale")
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The tray is the one place the app says something without i18next, so the
    // table has to cover every language the catalogues do — a locale missing
    // here would silently fall back to English in the menu bar alone.
    #[test]
    fn every_shipped_locale_has_a_tray_menu() {
        // The same set, exactly: a duplicate here could otherwise stand in for
        // a locale the table forgot, and the count alone would not notice.
        let mut tags: Vec<&str> = LABELS.iter().map(|(tag, _)| *tag).collect();
        tags.sort_unstable();
        let mut shipped = locale::SUPPORTED.to_vec();
        shipped.sort_unstable();
        assert_eq!(tags, shipped);
        assert_eq!(LABELS[0].0, "en-US", "en-US is the fallback");
    }

    // `{}` is what `create` substitutes the app's name into; a label that lost
    // it would read "Open" with nothing after it.
    #[test]
    fn every_open_label_names_the_app() {
        for (tag, labels) in LABELS {
            assert!(labels.show.contains("{}"), "{tag}");
            assert!(!labels.show.replace("{}", APP_NAME).contains("{}"), "{tag}");
        }
    }
}
