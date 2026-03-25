mod state;
mod commands;

use std::sync::Mutex;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(state::AppState {
            volume: Mutex::new(None),
        })
        .invoke_handler(tauri::generate_handler![
            commands::detect_volume,
            commands::create_volume,
            commands::unlock_volume,
            commands::lock_volume,
            commands::get_volume_info,
            commands::get_status,
            commands::list_dir,
            commands::stat_entry,
            commands::read_file_preview,
            commands::list_raw_partitions,
            commands::check_winfsp,
            commands::mount_drive,
            commands::unmount_drive,
            commands::get_default_volumes_dir,
            commands::list_volume_files,
            commands::list_free_drive_letters,
            commands::get_disk_free_space,
            commands::benchmark_kdf,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
