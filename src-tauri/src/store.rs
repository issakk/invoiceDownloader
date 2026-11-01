use std::collections::HashSet;
use std::fs;

use serde::{Deserialize, Serialize};
use tauri::AppHandle;

use crate::config;

/// 一份已归档的 PDF 发票及其解析出的字段
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Invoice {
    pub hash: String,
    pub file_name: String,
    pub path: String,
    pub date: String,
    pub amount: String,
    pub seller: String,
    pub no: String,
    pub subject: String,
    pub from: String,
    pub from_addr: String,
    /// attachment | link
    pub source_kind: String,
    /// 附件名或下载链接
    pub source_detail: String,
    pub created_at: String,
    pub size: u64,
    pub text_chars: usize,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    #[serde(default)]
    pub invoices: Vec<Invoice>,
}

impl Manifest {
    pub fn hashes(&self) -> HashSet<String> {
        self.invoices.iter().map(|i| i.hash.clone()).collect()
    }
}

pub fn load(app: &AppHandle) -> Result<Manifest, String> {
    let path = config::manifest_file(app)?;
    if !path.exists() {
        return Ok(Manifest::default());
    }
    let raw = fs::read_to_string(&path).map_err(|e| format!("读取清单失败: {e}"))?;
    if raw.trim().is_empty() {
        return Ok(Manifest::default());
    }
    serde_json::from_str(&raw).map_err(|e| format!("清单格式错误: {e}"))
}

pub fn save(app: &AppHandle, manifest: &Manifest) -> Result<(), String> {
    let path = config::manifest_file(app)?;
    let raw = serde_json::to_string_pretty(manifest).map_err(|e| e.to_string())?;
    fs::write(&path, raw).map_err(|e| format!("写入清单失败: {e}"))
}

/// 按创建时间倒序返回
pub fn list(app: &AppHandle) -> Result<Vec<Invoice>, String> {
    let mut manifest = load(app)?;
    manifest
        .invoices
        .sort_by(|a, b| b.created_at.cmp(&a.created_at));
    Ok(manifest.invoices)
}
