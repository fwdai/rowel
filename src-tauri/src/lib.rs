mod app;
mod appkey;
// The master-password domain: the failed-unlock backoff and the rekey saga.
mod auth;
mod autolock;
mod biometrics;
// The browser extension host: the KeePassXC-Browser protocol over a local
// socket, and the proxy mode the browser launches this binary in. `pub` so the
// proxy integration test can reach the socket name and framing.
#[cfg(desktop)]
pub mod browser;
mod cards;
mod commands;
pub mod crypto;
mod error;
mod events;
mod favicon;
mod grants;
mod hibp;
mod import;
mod locale;
mod models;
// Backups the OS asks the app to open (the `.rowel`/`.swftx` file associations).
mod opened;
// How a stored `otp` value is spelled — read by the generator, the importers
// and the exporters alike, so it sits below all three.
mod otp;
// Owner-only file permissions, on Unix and Windows alike.
mod owner_only;
// The WebAuthn authenticator core. Declared only: its caller is the browser
// extension host a later PR adds, so no command is registered below yet.
mod passkey;
mod save;
// Local image scanning (card / identity document). `pub` so `examples/scan.rs`
// can drive the OCR backend without the app around it.
pub mod scan;
mod secure_store;
mod session;
mod settings;
mod share;
mod state;
mod storage;
pub mod store;
mod sync;
mod timer;
// `tauri::tray` and `tauri::menu` are desktop-only.
#[cfg(desktop)]
mod tray;
mod window;
// The optional additional vaults, and which of them the app is addressing.
mod workspace;
// The macOS window zoom, re-done so the page follows it.
#[cfg(target_os = "macos")]
mod zoom;

use state::AppState;
use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // A browser launched this binary as its native messaging host: relay its
    // stdio to the running app and never build a window (see `browser::proxy`).
    #[cfg(desktop)]
    if browser::proxy::launched_by_browser(std::env::args().skip(1)) {
        std::process::exit(browser::proxy::run());
    }

    // Before anything else: on mobile Tauri builds a reqwest client while
    // launching, and with `rustls-no-provider` that aborts the process unless
    // a provider is already installed (see `sync::install_crypto_provider`).
    sync::install_crypto_provider();

    // Only the desktop-gated blocks below reassign it.
    #[cfg_attr(mobile, allow(unused_mut))]
    let mut builder = tauri::Builder::default();

    // Desktop-only plugins.
    #[cfg(desktop)]
    {
        builder = builder
            .plugin(tauri_plugin_single_instance::init(|app, argv, cwd| {
                // Focus the existing window on a second launch.
                window::show(app);
                // On Windows and Linux a file association runs the executable
                // again with the document as its argument; the second instance
                // hands its command line here and exits. `argv[0]` is itself.
                opened::accept(
                    app,
                    opened::from_args(argv.into_iter().skip(1), std::path::Path::new(&cwd)),
                );
            }))
            .plugin(tauri_plugin_updater::Builder::new().build())
            .plugin(tauri_plugin_process::init());
    }

    // Mobile-only plugins.
    #[cfg(mobile)]
    {
        // The OAuth redirect for the public mobile client arrives on the app's
        // own URL scheme (see `tauri.ios.conf.json`), not on a loopback port.
        builder = builder.plugin(tauri_plugin_deep_link::init());
    }

    // The timer behind the delayed clipboard clear. Not on iOS, where the
    // pasteboard's own expiry does the clearing and no timer is armed (see
    // `commands::clipboard`).
    #[cfg(not(target_os = "ios"))]
    {
        builder = builder.manage(commands::clipboard::ClipboardClear::default());
    }

    // In-app W3C WebDriver server (port 4445) for the E2E smoke suite. Never
    // compiled into a release binary, and desktop-only — the suite drives the
    // desktop app.
    #[cfg(all(debug_assertions, desktop))]
    {
        builder = builder.plugin(tauri_plugin_webdriver::init());
    }

    builder
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(
            // The plugin defaults to Trace, which pulls in every dependency's
            // tracing -- tao logs entry and exit of each NSWindowDelegate
            // callback, so ordinary focus changes flood stderr and the log file.
            tauri_plugin_log::Builder::new()
                .level(log::LevelFilter::Info)
                .level_for("rowel_lib", log::LevelFilter::Debug)
                .target(tauri_plugin_log::Target::new(
                    tauri_plugin_log::TargetKind::Stderr,
                ))
                .target(tauri_plugin_log::Target::new(
                    tauri_plugin_log::TargetKind::LogDir { file_name: None },
                ))
                .build(),
        )
        .manage(AppState::default())
        .manage(grants::PathGrants::default())
        .manage(autolock::AutoLock::default())
        .manage(settings::SettingsState::default())
        .setup(|app| {
            // Preferences first: the shell and the auto-lock both open on them.
            settings::boot(app.handle());
            // The plaintext favicon directory the in-vault cache replaced: the
            // vault's host list in the clear, so it goes on the first launch
            // that can see it, whether or not this one looks an icon up.
            storage::remove_legacy_icons_dir(app.handle());
            // A workspace delete a crash caught between its file moves and the
            // registry write: put back, or cleared away, before anything reads
            // either — the registry below is the first thing that does.
            let root = storage::root_dir(app.handle())?;
            workspace::recover_interrupted_delete(&root);
            // The extension host, when it was left on: the browsers' manifests
            // are put right — an update moved the executable, a browser
            // arrived since, a manifest went missing — and the listener is up
            // from launch, so a browser that starts before the vault is
            // unlocked finds the app and can ask for the unlock. Writing the
            // manifests again is idempotent, and one another host owns is
            // left alone (`manifest::install`).
            #[cfg(desktop)]
            if settings::current(app.handle()).browser.enabled {
                browser::manifest::install(&root);
                browser::server::start(app.handle());
            }
            // Which workspace was open last. Read before the window exists, so
            // the lock screen the user lands on is that workspace's.
            let registry = workspace::Registry::load(&root);
            *app.state::<AppState>().active_workspace.lock().unwrap() = registry.active;

            window::create(app.handle())?;
            #[cfg(desktop)]
            tray::create(app.handle())?;
            // Launched *for* a document (Windows/Linux file associations put
            // it on the command line; macOS sends `RunEvent::Opened` instead,
            // see `run` below). Parked until the webview asks for it.
            #[cfg(desktop)]
            if let Ok(cwd) = std::env::current_dir() {
                opened::accept(
                    app.handle(),
                    opened::from_args(std::env::args().skip(1), &cwd),
                );
            }
            // The second half of the mobile OAuth flow: iOS reopens the app
            // with Google's redirect once the user has approved.
            #[cfg(mobile)]
            {
                use tauri_plugin_deep_link::DeepLinkExt;
                let handle = app.handle().clone();
                app.deep_link().on_open_url(move |event| {
                    for url in event.urls() {
                        commands::sync::on_redirect(&handle, &url);
                    }
                });
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::auth::unlock,
            commands::auth::lock,
            commands::auth::touch_activity,
            commands::auth::unlock_biometric,
            commands::auth::enable_biometric,
            commands::auth::disable_biometric,
            commands::auth::change_master_password,
            commands::app::app_status,
            commands::app::app_ready,
            commands::app::take_opened_file,
            commands::app::set_settings,
            commands::vault::reveal_entry,
            commands::vault::save_entry,
            commands::vault::delete_entry,
            commands::vault::list_deleted,
            commands::vault::restore_entry,
            commands::vault::purge_entry,
            commands::vault::set_favorite,
            commands::vault::import_swftx,
            commands::vault::export_vault,
            commands::vault::save_env_file,
            commands::import::import_entries,
            commands::import::export_entries,
            commands::env::read_env_file,
            commands::generator::generate_password,
            commands::generator::generate_ssh_key,
            commands::generator::generate_otp,
            commands::audit::get_audit,
            commands::tools::pick_file,
            commands::tools::scan_image,
            commands::tools::fetch_favicon,
            commands::clipboard::copy_to_clipboard,
            commands::setup::setup_drive_connect,
            commands::setup::setup_drive_disconnect,
            commands::setup::setup_restore_from_drive,
            commands::setup::setup_restore_from_file,
            commands::setup::setup_create,
            commands::sync::sync_connect,
            commands::sync::sync_disconnect,
            commands::sync::sync_now,
            commands::sync::sync_adopt_pending,
            commands::share::share_create,
            commands::share::share_open,
            commands::share::share_revoke,
            commands::share::share_list,
            commands::workspace::workspace_select,
            commands::workspace::workspace_create,
            commands::workspace::workspace_drive_connect,
            commands::workspace::workspace_restore_from_drive,
            commands::workspace::workspace_restore_from_account,
            commands::workspace::workspace_rename,
            commands::workspace::workspace_set_color,
            commands::workspace::workspace_delete,
            // E2E-only vault reset. `generate_handler!` honours per-command
            // attributes, so in a release build the match arm — and with it the
            // only reference to the (also cfg'd-out) module — simply is not
            // generated. See `commands/e2e.rs` for the runtime gates.
            #[cfg(debug_assertions)]
            commands::e2e::e2e_reset,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|_app, _event| {
            // macOS hands a document over as an event rather than on the
            // command line — at launch, when Finder opened one with the app,
            // and again for every one opened while it runs.
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Opened { urls } = &_event {
                opened::accept(_app, opened::from_urls(urls));
            }
        });
}
