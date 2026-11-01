#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod config;
mod invoice;
mod mail;
mod pdf;
mod store;
mod util;

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            commands::app_info,
            commands::load_config,
            commands::save_config,
            commands::test_connection,
            commands::start_sync,
            commands::list_invoices,
            commands::update_fields,
            commands::rename_invoices,
            commands::export_invoices,
            commands::export_csv,
            commands::delete_invoices,
            commands::open_data_dir,
            commands::open_invoice,
            commands::reveal_invoice,
        ])
        .run(tauri::generate_context!())
        .expect("InvoiceBox 启动失败");
}
