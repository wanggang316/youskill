#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tauri::webview::PageLoadEvent;
#[cfg(target_os = "macos")]
use tauri::window::{Effect, EffectState, EffectsBuilder};
use tauri::{App, AppHandle, Manager, WebviewWindow, WebviewWindowBuilder};

#[cfg(target_os = "macos")]
mod app_menu;
mod commands;
mod tray;

use youskill_core::{config, models, services, utils};

use commands::{
  agent_apps::{
    add_user_agent_app, list_local_agent_apps, refresh_agent_apps, remove_user_agent_app,
    update_user_agent_app,
  },
  cli::{cli_status, install_cli, uninstall_cli},
  favorites::{list_favorites, set_favorite},
  github::{get_github_auth_status, get_github_avatar, set_github_account},
  hub::{
    check_source_updates, diff_skill, get_hub_skill, import_scanned, import_skills, install_skill,
    list_hub_skills, migrate_legacy, migration_status, plan_import, remove_hub_skill, scan_folder,
    sync_skill, uninstall_skill,
  },
  instructions::{
    create_instruction, detect_instruction_files, detect_instruction_github, diff_instruction,
    get_instruction, import_instructions, install_instruction, list_instruction_files,
    list_instructions, read_instruction, read_instruction_file, remove_instruction,
    rename_instruction, sync_instruction, uninstall_instruction, write_instruction,
    write_instruction_file,
  },
  library_sync::{
    library_sync_apply_pending, library_sync_apply_restore, library_sync_backup,
    library_sync_connect, library_sync_disconnect, library_sync_history, library_sync_migrate,
    library_sync_pending, library_sync_plan_restore, library_sync_status,
    library_sync_undo_restore,
  },
  remote::{
    fetch_hot_repositories, fetch_market_categories, fetch_remote_skill, fetch_remote_skills,
    fetch_skills_by_names, record_skill_install,
  },
  scan_roots::{add_scan_root, list_scan_roots, remove_scan_root, suggest_scan_roots},
  search::global_search,
  settings::{get_settings, list_openrouter_models, update_settings},
  skill::{
    detect_folder, detect_github_auto, detect_github_manual, detect_zip,
    get_cached_skill_translation, list_skill_directory, open_in_file_manager, read_skill_file,
    read_skill_relative_file, read_skill_relative_file_bytes, translate_skill_markdown,
  },
  user_projects::{
    add_workspace, list_memory_files, list_user_projects, list_workspaces, register_projects,
    remove_user_project, remove_workspace, scan_workspace,
  },
};
use tray::setup_tray;

/// Longest wait for the first page load before the main window is shown anyway.
const MAIN_WINDOW_SHOW_TIMEOUT: Duration = Duration::from_secs(3);

static MAIN_WINDOW_SHOWN: AtomicBool = AtomicBool::new(false);

/// The main window starts hidden so the native blank webview is never on screen; show it
/// once the page (with its boot skeleton) has loaded. Later reloads must not reopen a
/// window the user hid to the tray.
fn show_main_window_once(app: &AppHandle) {
  if MAIN_WINDOW_SHOWN.swap(true, Ordering::SeqCst) {
    return;
  }
  if let Some(window) = app.get_webview_window("main") {
    let _ = window.show();
    let _ = window.set_focus();
  }
}

/// The main window is declared in `tauri.conf.json` with `create: false` and built here, so
/// macOS alone gets a transparent window behind the native sidebar material. Other platforms
/// keep an opaque window: a transparent one without a supported effect would show the desktop.
fn build_main_window(app: &App) -> tauri::Result<WebviewWindow> {
  let config = app
    .config()
    .app
    .windows
    .iter()
    .find(|window| window.label == "main")
    .cloned()
    .expect("main window must be declared in tauri.conf.json");
  let builder = WebviewWindowBuilder::from_config(app.handle(), &config)?;
  #[cfg(target_os = "macos")]
  let builder = builder.transparent(true).effects(
    EffectsBuilder::new()
      .effect(Effect::Sidebar)
      .state(EffectState::FollowsWindowActiveState)
      .build(),
  );
  builder.build()
}

#[tauri::command]
fn ping() -> String {
  "pong".to_string()
}

fn main() {
  // Set default log level: info for dev, warn for production
  #[cfg(debug_assertions)]
  let default_level = "info";
  #[cfg(not(debug_assertions))]
  let default_level = "warn";

  // Initialize tracing subscriber for logging
  tracing_subscriber::fmt()
    .with_env_filter(
      tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(default_level)),
    )
    .init();

  tauri::Builder::default()
    .plugin(tauri_plugin_dialog::init())
    .plugin(tauri_plugin_shell::init())
    .plugin(tauri_plugin_fs::init())
    .plugin(tauri_plugin_updater::Builder::new().build())
    .plugin(tauri_plugin_process::init())
    .on_page_load(|webview, payload| {
      if webview.label() == "main" && payload.event() == PageLoadEvent::Finished {
        show_main_window_once(webview.app_handle());
      }
    })
    .setup(|app| {
      let main_window = build_main_window(app)?;

      // Fallback for a page load that never finishes, so the window cannot stay hidden.
      let handle = app.handle().clone();
      std::thread::spawn(move || {
        std::thread::sleep(MAIN_WINDOW_SHOW_TIMEOUT);
        let main_thread_handle = handle.clone();
        let _ = handle.run_on_main_thread(move || show_main_window_once(&main_thread_handle));
      });

      // Setup tray icon and menu
      if let Err(e) = setup_tray(app.handle()) {
        eprintln!("Failed to setup tray: {}", e);
      }

      #[cfg(target_os = "macos")]
      if let Err(e) = app_menu::setup_app_menu(app.handle()) {
        eprintln!("Failed to setup app menu: {}", e);
      }

      // Handle window close event - hide to tray instead of quitting
      let window_clone = main_window.clone();

      main_window.on_window_event(move |event| {
        if let tauri::WindowEvent::CloseRequested { api, .. } = event {
          // Prevent window from closing
          api.prevent_close();
          // Just hide window instead
          let _ = window_clone.hide();
        }
      });

      Ok(())
    })
    .invoke_handler(tauri::generate_handler![
      ping,
      get_settings,
      update_settings,
      cli_status,
      install_cli,
      uninstall_cli,
      get_github_auth_status,
      set_github_account,
      get_github_avatar,
      fetch_remote_skills,
      fetch_remote_skill,
      fetch_market_categories,
      fetch_hot_repositories,
      fetch_skills_by_names,
      record_skill_install,
      list_favorites,
      set_favorite,
      list_local_agent_apps,
      refresh_agent_apps,
      add_user_agent_app,
      remove_user_agent_app,
      update_user_agent_app,
      detect_zip,
      detect_folder,
      detect_github_manual,
      detect_github_auto,
      list_hub_skills,
      get_hub_skill,
      plan_import,
      import_skills,
      install_skill,
      uninstall_skill,
      remove_hub_skill,
      sync_skill,
      diff_skill,
      check_source_updates,
      scan_folder,
      import_scanned,
      list_scan_roots,
      add_scan_root,
      remove_scan_root,
      suggest_scan_roots,
      migrate_legacy,
      migration_status,
      library_sync_status,
      library_sync_connect,
      library_sync_disconnect,
      library_sync_backup,
      library_sync_migrate,
      library_sync_plan_restore,
      library_sync_apply_restore,
      library_sync_undo_restore,
      library_sync_history,
      library_sync_pending,
      library_sync_apply_pending,
      list_openrouter_models,
      read_skill_file,
      list_skill_directory,
      read_skill_relative_file,
      read_skill_relative_file_bytes,
      get_cached_skill_translation,
      translate_skill_markdown,
      open_in_file_manager,
      list_user_projects,
      remove_user_project,
      list_workspaces,
      add_workspace,
      remove_workspace,
      scan_workspace,
      register_projects,
      list_memory_files,
      list_instructions,
      get_instruction,
      read_instruction,
      import_instructions,
      create_instruction,
      install_instruction,
      uninstall_instruction,
      remove_instruction,
      sync_instruction,
      diff_instruction,
      write_instruction,
      rename_instruction,
      list_instruction_files,
      read_instruction_file,
      write_instruction_file,
      detect_instruction_files,
      detect_instruction_github,
      global_search
    ])
    .build(tauri::generate_context!())
    .expect("error while building tauri application")
    .run(|_app_handle, _event| {
      // RunEvent::Reopen only exists on macOS, where clicking the Dock icon
      // fires it - show the main window again.
      #[cfg(target_os = "macos")]
      if let tauri::RunEvent::Reopen { .. } = _event {
        if let Some(window) = _app_handle.get_webview_window("main") {
          let _ = window.show();
          let _ = window.set_focus();
        }
      }
    });
}
