#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

mod state;
mod tasks;

use std::thread;

use log::{debug, LevelFilter};
use tasks::trackers::TrackerManager;
use tauri::{Manager, RunEvent};
use tauri_plugin_log::LoggerBuilder;
use tauri_plugin_store::StoreBuilder;
use tokio::{runtime, sync::oneshot};

use hdt_mem_reader::manager::ManagerHandle;

#[cfg(debug_assertions)]
static LOG_LEVEL: LevelFilter = log::LevelFilter::Debug;

#[cfg(not(debug_assertions))]
static LOG_LEVEL: LevelFilter = log::LevelFilter::Info;

static MAIN_CONFIG: &str = "hd-toolbox.config";

#[tauri::command]
fn launch_spelunky_hd() -> Result<String, String> {
    open::that("steam://run/239350").map_err(|_| "Failed to launch!")?;
    Ok("Launched!".into())
}

async fn run_mem_manager() -> ManagerHandle {
    debug!("Spawning thread for Memory Manager");
    let (tx, rx) = oneshot::channel::<ManagerHandle>();
    thread::spawn(move || {
        let basic_rt = runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        basic_rt.block_on(async {
            let mut manager = hdt_mem_reader::manager::Manager::new();
            let handle = manager.get_handle();
            let _ = tx.send(handle);

            manager.run_forever().await;
        })
    });
    rx.await.expect("Failed to run Memory Manger")
}

fn main() -> anyhow::Result<()> {
    let main_config = StoreBuilder::new(MAIN_CONFIG.parse()?).build();
    let log_plugin = LoggerBuilder::new()
        .level_for("attohttpc", log::LevelFilter::Warn)
        .level_for("mio::poll", log::LevelFilter::Warn)
        .level_for("tungstenite::protocol", log::LevelFilter::Warn)
        .level(LOG_LEVEL)
        .build();

    tauri::Builder::default()
        .plugin(log_plugin)
        .plugin(
            tauri_plugin_store::PluginBuilder::default()
                .stores([main_config])
                .freeze()
                .build(),
        )
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .setup(|app| {
            let handle = app.handle();
            tauri::async_runtime::spawn(async move {
                let memory_manager = run_mem_manager().await;
                let tracker_manager =
                    TrackerManager::run_in_background(memory_manager.clone()).await;
                handle.manage(state::State::new(memory_manager, tracker_manager));
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            launch_spelunky_hd,
            tasks::start_task,
            tasks::stop_task,
            tasks::fix_slowlook,
            tasks::set_character,
        ])
        .build(tauri::generate_context!())
        .expect("error while running tauri application")
        .run(|app, event| {
            if let RunEvent::Exit = event {
                let app_handle = app.clone();
                tauri::async_runtime::spawn(async move {
                    let state = app_handle.state::<state::State>();
                    state.tracker_manager.shutdown().await.ok();
                    state.mem_manager.shutdown().await.ok();
                });
            }
        });
    Ok(())
}
