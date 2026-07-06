mod state;
mod commands;

use std::sync::Mutex;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(state::AppState {
            volume: Mutex::new(None),
            bench_cancel: Arc::new(AtomicBool::new(false)),
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
            commands::get_cpu_count,
            commands::benchmark_kdf,
            commands::benchmark_format_io,
            commands::cancel_benchmark,
            commands::verify_volume,
            commands::verify_mounted_volume,
            commands::wipe_volume,
            commands::wipe_mounted_volume,
            commands::check_aes_ni,
            // Phase B2 / C / D — v3 security features
            commands::benchmark_crypto_speed,
            commands::add_key_slot,
            commands::remove_key_slot,
            commands::list_key_slots,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
