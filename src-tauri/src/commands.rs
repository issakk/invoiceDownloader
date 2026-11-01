use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

use serde::Serialize;
use tauri::{AppHandle, Emitter};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;

use crate::config::{self, Config};
use crate::invoice;
use crate::mail;
use crate::pdf::{Fields, Meta};
use crate::store::{self, Invoice};
use crate::util;

static SYNC_RUNNING: AtomicBool = AtomicBool::new(false);

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncReport {
    scanned_mails: usize,
    found_pdfs: usize,
    new_count: usize,
    duplicates: usize,
    errors: Vec<String>,
    invoices: Vec<Invoice>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    portable: bool,
    data_dir: String,
}

/// 前端用来显示「便携模式 / 标准模式」以及数据实际位置
#[tauri::command]
pub fn app_info(app: AppHandle) -> Result<AppInfo, String> {
    Ok(AppInfo {
        portable: config::portable_root().is_some(),
        data_dir: config::data_dir(&app)?.to_string_lossy().to_string(),
    })
}

#[tauri::command]
pub fn load_config(app: AppHandle) -> Result<Config, String> {
    config::load(&app)
}

#[tauri::command]
pub fn save_config(app: AppHandle, config: Config) -> Result<Config, String> {
    config::save(&app, &config)?;
    Ok(config)
}

#[tauri::command]
pub async fn test_connection(config: Config) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || mail::test_connection(&config))
        .await
        .map_err(|e| format!("测试任务异常: {e}"))?
}

#[tauri::command]
pub async fn start_sync(app: AppHandle, config: Config) -> Result<SyncReport, String> {
    if SYNC_RUNNING.swap(true, Ordering::SeqCst) {
        return Err("已有收信任务在运行，请等待完成".into());
    }
    // 记住本次实际使用的配置
    let _ = config::save(&app, &config);

    let handle = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || run_sync(handle, config)).await;
    SYNC_RUNNING.store(false, Ordering::SeqCst);
    match result {
        Ok(value) => value,
        Err(e) => Err(format!("收信任务异常: {e}")),
    }
}

fn run_sync(app: AppHandle, cfg: Config) -> Result<SyncReport, String> {
    let emitter = app.clone();
    let log = move |message: &str| {
        let _ = emitter.emit("sync-log", message.to_string());
    };
    log(&format!(
        "开始收取：{} 最近 {} 天",
        cfg.folder, cfg.since_days
    ));
    let mails = mail::fetch_all(&cfg, &log)?;
    log(&format!("解析完成，发现 {} 封含 PDF 的邮件", mails.len()));
    let outcome = invoice::ingest(&app, &cfg, mails, &log)?;
    log(&format!(
        "完成：新增 {} 个，重复跳过 {} 个",
        outcome.new_invoices.len(),
        outcome.duplicates
    ));
    Ok(SyncReport {
        scanned_mails: outcome.scanned_mails,
        found_pdfs: outcome.found_pdfs,
        new_count: outcome.new_invoices.len(),
        duplicates: outcome.duplicates,
        errors: outcome.errors,
        invoices: outcome.new_invoices,
    })
}

#[tauri::command]
pub fn list_invoices(app: AppHandle) -> Result<Vec<Invoice>, String> {
    store::list(&app)
}

#[tauri::command]
pub fn update_fields(
    app: AppHandle,
    hash: String,
    date: String,
    amount: String,
    seller: String,
    no: String,
) -> Result<(), String> {
    let mut manifest = store::load(&app)?;
    let item = manifest
        .invoices
        .iter_mut()
        .find(|i| i.hash == hash)
        .ok_or_else(|| "未找到该发票记录".to_string())?;
    item.date = date;
    item.amount = amount;
    item.seller = seller;
    item.no = no;
    store::save(&app, &manifest)
}

fn unique_among(base: &str, used: &mut HashSet<String>) -> String {
    let base = if base.is_empty() { "invoice" } else { base };
    let candidate = format!("{base}.pdf");
    if used.insert(candidate.to_lowercase()) {
        return candidate;
    }
    for index in 2..2000 {
        let candidate = format!("{base}-{index}.pdf");
        if used.insert(candidate.to_lowercase()) {
            return candidate;
        }
    }
    format!("{base}-{}.pdf", util::sha256_hex(base.as_bytes()))
}

fn meta_from(item: &Invoice) -> (Fields, String, String, String) {
    let fields = Fields {
        date: item.date.clone(),
        amount: item.amount.clone(),
        seller: item.seller.clone(),
        no: item.no.clone(),
        warnings: item.warnings.clone(),
    };
    (
        fields,
        item.subject.clone(),
        item.from.clone(),
        item.from_addr.clone(),
    )
}

/// 按模板批量重命名选中的发票（磁盘文件同步改名）
#[tauri::command]
pub fn rename_invoices(
    app: AppHandle,
    hashes: Vec<String>,
    template: String,
) -> Result<Vec<Invoice>, String> {
    if hashes.is_empty() {
        return Err("请先选择要重命名的发票".into());
    }
    let selected: HashSet<String> = hashes.into_iter().collect();
    let mut manifest = store::load(&app)?;
    let dir = config::invoices_dir(&app)?;
    let mut used: HashSet<String> = manifest
        .invoices
        .iter()
        .filter(|i| !selected.contains(&i.hash))
        .map(|i| i.file_name.to_lowercase())
        .collect();

    let mut updated: Vec<Invoice> = Vec::new();
    for item in manifest.invoices.iter_mut() {
        if !selected.contains(&item.hash) {
            continue;
        }
        let (fields, subject, from_name, from_addr) = meta_from(item);
        let old_stem = item
            .file_name
            .rsplit('/')
            .next()
            .unwrap_or(&item.file_name)
            .trim_end_matches(".pdf")
            .to_string();
        let meta = Meta {
            subject: &subject,
            from_name: &from_name,
            from_addr: &from_addr,
            file_stem: &old_stem,
            mail_date: &fields.date,
        };
        let base = invoice::render_name(&template, &fields, &meta, &item.source_kind, &item.hash);
        let file_name = unique_among(&base, &mut used);
        let target = dir.join(&file_name);

        if item.path != target.to_string_lossy() {
            if let Err(e) = fs::rename(&item.path, &target) {
                return Err(format!("重命名 {} 失败: {e}", item.file_name));
            }
        }
        item.file_name = file_name;
        item.path = target.to_string_lossy().to_string();
        updated.push(item.clone());
    }
    store::save(&app, &manifest)?;
    Ok(updated)
}

fn csv_cell(raw: &str) -> String {
    let needs_quote = raw.contains(',') || raw.contains('"') || raw.contains('\n');
    let escaped = raw.replace('"', "\"\"");
    if needs_quote {
        format!("\"{escaped}\"")
    } else {
        escaped
    }
}

fn csv_rows(items: &[Invoice]) -> String {
    let mut out = String::from("\u{feff}文件名,日期,销售方,发票号,金额,来源,类型,邮件主题\n");
    for item in items {
        out.push_str(&format!(
            "{},{},{},{},{},{},{},{}\n",
            csv_cell(&item.file_name),
            csv_cell(&item.date),
            csv_cell(&item.seller),
            csv_cell(&item.no),
            csv_cell(&item.amount),
            csv_cell(&item.from_addr),
            csv_cell(if item.source_kind.is_empty() {
                ""
            } else {
                &item.source_kind
            }),
            csv_cell(&item.subject),
        ));
    }
    out
}

#[tauri::command]
pub async fn export_invoices(app: AppHandle, hashes: Vec<String>) -> Result<String, String> {
    let manifest = store::load(&app)?;
    let selected: HashSet<String> = hashes.into_iter().collect();
    let items: Vec<Invoice> = manifest
        .invoices
        .into_iter()
        .filter(|i| selected.contains(&i.hash))
        .collect();
    if items.is_empty() {
        return Err("请先选择要导出的发票".into());
    }

    let picker = app.clone();
    let picked = tauri::async_runtime::spawn_blocking(move || {
        picker
            .dialog()
            .file()
            .set_title("选择发票导出文件夹")
            .blocking_pick_folder()
    })
    .await
    .map_err(|e| format!("打开文件夹选择器失败: {e}"))?;

    let Some(folder) = picked else {
        return Ok(String::new());
    };
    let target = PathBuf::from(folder.to_string());
    fs::create_dir_all(&target).map_err(|e| format!("创建导出目录失败: {e}"))?;

    let stamp = util::today().replace('-', "");
    let sub = target.join(format!("InvoiceBox-{stamp}"));
    fs::create_dir_all(&sub).map_err(|e| format!("创建导出子目录失败: {e}"))?;

    let mut exported = 0usize;
    let mut copied: Vec<Invoice> = Vec::new();
    for item in &items {
        let dest = sub.join(&item.file_name);
        match fs::copy(&item.path, &dest) {
            Ok(_) => {
                exported += 1;
                copied.push(item.clone());
            }
            Err(_) => continue,
        }
    }
    fs::write(sub.join("发票清单.csv"), csv_rows(&copied))
        .map_err(|e| format!("写入清单失败: {e}"))?;

    Ok(format!(
        "已导出 {exported} 个 PDF + 清单.csv\n{}",
        sub.to_string_lossy()
    ))
}

#[tauri::command]
pub fn export_csv(app: AppHandle) -> Result<String, String> {
    let items = store::list(&app)?;
    let path = config::data_dir(&app)?.join("发票清单.csv");
    fs::write(&path, csv_rows(&items)).map_err(|e| format!("写入 CSV 失败: {e}"))?;
    Ok(path.to_string_lossy().to_string())
}

#[tauri::command]
pub fn delete_invoices(app: AppHandle, hashes: Vec<String>) -> Result<(), String> {
    let selected: HashSet<String> = hashes.into_iter().collect();
    let mut manifest = store::load(&app)?;
    let mut keep: Vec<Invoice> = Vec::new();
    for item in manifest.invoices.into_iter() {
        if selected.contains(&item.hash) {
            let _ = fs::remove_file(&item.path);
        } else {
            keep.push(item);
        }
    }
    manifest.invoices = keep;
    store::save(&app, &manifest)
}

#[tauri::command]
pub fn open_data_dir(app: AppHandle) -> Result<(), String> {
    let dir = config::data_dir(&app)?;
    app.opener()
        .open_path(dir.to_string_lossy().to_string(), None::<&str>)
        .map_err(|e| format!("打开目录失败: {e}"))
}

#[tauri::command]
pub fn open_invoice(app: AppHandle, path: String) -> Result<(), String> {
    app.opener()
        .open_path(path, None::<&str>)
        .map_err(|e| format!("打开文件失败: {e}"))
}

#[tauri::command]
pub fn reveal_invoice(app: AppHandle, path: String) -> Result<(), String> {
    app.opener()
        .reveal_item_in_dir(path)
        .map_err(|e| format!("定位文件失败: {e}"))
}
