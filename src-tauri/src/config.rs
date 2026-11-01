use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Config {
    /// IMAP 服务器地址
    pub imap_host: String,
    /// IMAP 端口
    pub imap_port: u16,
    /// `tls` 或 `starttls`
    pub imap_tls: String,
    /// 邮箱账号
    pub username: String,
    /// 授权码 / 应用专用密码
    pub password: String,
    /// 要扫描的邮箱文件夹
    pub folder: String,
    /// 扫描最近多少天
    pub since_days: u32,
    /// 单次同步最多处理多少封邮件
    pub max_mails: u32,
    /// 每封邮件最多尝试几个下载链接，0 表示不跟随链接
    pub max_links_per_mail: u32,
    /// 主题/发件人关键字过滤，逗号分隔，空表示不过滤
    pub keyword_filter: String,
    /// 命名模板，例如 {date}_{amount}_{seller}_{no}
    pub naming_template: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            imap_host: String::new(),
            imap_port: 993,
            imap_tls: "tls".to_string(),
            username: String::new(),
            password: String::new(),
            folder: "INBOX".to_string(),
            since_days: 90,
            max_mails: 500,
            max_links_per_mail: 2,
            keyword_filter: String::new(),
            naming_template: "{date}_{amount}_{seller}_{no}".to_string(),
        }
    }
}

impl Config {
    /// 前端保存前的轻量校验
    pub fn validate(&self) -> Result<(), String> {
        if self.imap_host.trim().is_empty() {
            return Err("请填写 IMAP 服务器地址".into());
        }
        if self.username.trim().is_empty() {
            return Err("请填写邮箱账号".into());
        }
        if self.password.is_empty() {
            return Err("请填写授权码 / 密码".into());
        }
        if self.imap_port == 0 {
            return Err("端口不合法".into());
        }
        Ok(())
    }
}

fn exe_dir() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()?
        .parent()
        .map(|dir| dir.to_path_buf())
}

/// 便携模式：exe 同目录放了 `portable` 标记文件，或设置了 `INVOICEBOX_PORTABLE=1`，
/// 配置、清单和 PDF 都存到 exe 旁边的 `InvoiceBoxData`，整个程序可以放 U 盘里带走。
pub fn portable_root() -> Option<PathBuf> {
    if std::env::var("INVOICEBOX_PORTABLE").is_ok_and(|v| v == "1") {
        return Some(exe_dir()?.join("InvoiceBoxData"));
    }
    let dir = exe_dir()?;
    let marked = ["portable", "portable.txt", "InvoiceBox.portable"]
        .iter()
        .any(|marker| dir.join(marker).exists());
    marked.then(|| dir.join("InvoiceBoxData"))
}

fn ensure(dir: &PathBuf, what: &str) -> Result<(), String> {
    fs::create_dir_all(dir).map_err(|e| format!("创建{what}失败: {e}"))
}

pub fn config_dir(app: &AppHandle) -> Result<PathBuf, String> {
    if let Some(root) = portable_root() {
        ensure(&root, "配置目录")?;
        return Ok(root);
    }
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|e| format!("无法定位配置目录: {e}"))?;
    ensure(&dir, "配置目录")?;
    Ok(dir)
}

pub fn data_dir(app: &AppHandle) -> Result<PathBuf, String> {
    if let Some(root) = portable_root() {
        ensure(&root, "数据目录")?;
        return Ok(root);
    }
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("无法定位数据目录: {e}"))?;
    ensure(&dir, "数据目录")?;
    Ok(dir)
}

pub fn invoices_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = data_dir(app)?.join("invoices");
    fs::create_dir_all(&dir).map_err(|e| format!("创建发票目录失败: {e}"))?;
    Ok(dir)
}

pub fn config_file(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(config_dir(app)?.join("config.json"))
}

pub fn manifest_file(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(data_dir(app)?.join("invoices.json"))
}

pub fn load(app: &AppHandle) -> Result<Config, String> {
    let path = config_file(app)?;
    if !path.exists() {
        return Ok(Config::default());
    }
    let raw = fs::read_to_string(&path).map_err(|e| format!("读取配置失败: {e}"))?;
    if raw.trim().is_empty() {
        return Ok(Config::default());
    }
    serde_json::from_str(&raw).map_err(|e| format!("配置文件格式错误: {e}"))
}

pub fn save(app: &AppHandle, cfg: &Config) -> Result<(), String> {
    let path = config_file(app)?;
    let raw = serde_json::to_string_pretty(cfg).map_err(|e| e.to_string())?;
    fs::write(&path, raw).map_err(|e| format!("写入配置失败: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o600));
    }
    Ok(())
}
