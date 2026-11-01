use std::net::TcpStream;
use std::time::Duration;

use mail_parser::{MessageParser, MimeHeaders};

use crate::config::Config;
use crate::util;

/// 候选 PDF（来自附件或邮件正文里的下载链接）
pub struct Candidate {
    pub label: String,
    pub detail: String,
    pub kind: &'static str,
    pub bytes: Vec<u8>,
}

/// 一封解析后的邮件
pub struct MailItem {
    pub uid: u32,
    pub subject: String,
    pub from_name: String,
    pub from_addr: String,
    pub date: String,
    pub candidates: Vec<Candidate>,
}

type Logger<'a> = &'a dyn Fn(&str);

fn connect_session(
    cfg: &Config,
) -> Result<imap::Session<native_tls::TlsStream<TcpStream>>, String> {
    if cfg.imap_host.trim().is_empty() {
        return Err("请先在设置里填写 IMAP 服务器".into());
    }
    let tls = native_tls::TlsConnector::builder()
        .build()
        .map_err(|e| format!("TLS 初始化失败: {e}"))?;
    let host = cfg.imap_host.trim();
    let client = if cfg.imap_tls.eq_ignore_ascii_case("starttls") {
        imap::connect_starttls((host, cfg.imap_port), host, &tls)
    } else {
        imap::connect((host, cfg.imap_port), host, &tls)
    }
    .map_err(|e| format!("连接 {host}:{} 失败: {e}", cfg.imap_port))?;
    client
        .login(&cfg.username, &cfg.password)
        .map_err(|(e, _)| format!("登录失败（QQ/163 请使用授权码）: {e}"))
}

/// 仅测试登录与文件夹可见性
pub fn test_connection(cfg: &Config) -> Result<String, String> {
    cfg.validate()?;
    let mut session = connect_session(cfg)?;
    let mailbox = session
        .select(&cfg.folder)
        .map_err(|e| format!("打开文件夹 {} 失败: {e}", cfg.folder))?;
    let mut lines = vec![format!("登录成功：{}", cfg.username)];
    lines.push(format!(
        "文件夹 {} 共 {} 封邮件",
        cfg.folder,
        mailbox.exists
    ));
    let since = util::imap_since(cfg.since_days);
    let hits = session
        .uid_search(format!("SINCE {since}"))
        .map_err(|e| format!("搜索失败: {e}"))?;
    lines.push(format!("最近 {} 天命中 {} 封", cfg.since_days, hits.len()));
    let _ = session.logout();
    Ok(lines.join("\n"))
}

fn parse_keywords(raw: &str) -> Vec<String> {
    raw.split([',', '，', ';', '；'])
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty())
        .collect()
}

fn collect_links(html: &str, text: &str, max: usize) -> Vec<String> {
    let hints = regex::Regex::new(
        r"(?i)(pdf|invoice|fapiao|receipt|bill|download|发票|账单|收据|附件)",
    )
    .expect("内置正则必须合法");
    let mut out: Vec<String> = Vec::new();
    for source in [html, text] {
        let bytes = source.as_bytes();
        let mut i = 0usize;
        while i + 4 < bytes.len() && out.len() < max * 4 {
            let rest = &source[i..];
            let start = match rest.find("http") {
                Some(p) => i + p,
                None => break,
            };
            let slice = &source[start..];
            let end = slice
                .find(['"', '\'', '<', '>', ' ', '\n', '\r', '\t', ')', ']', '}'])
                .unwrap_or(slice.len());
            let mut url = slice[..end].to_string();
            i = start + end.max(1);
            if !(url.starts_with("http://") || url.starts_with("https://")) {
                continue;
            }
            url = url.replace("&amp;", "&").replace("&#38;", "&");
            url = url.trim_end_matches(['.', ',', ';', ':']).to_string();
            if url.len() < 12 || out.iter().any(|u| u == &url) {
                continue;
            }
            if hints.is_match(&url) {
                out.insert(0, url);
            } else {
                out.push(url);
            }
        }
    }
    out.retain(|u| hints.is_match(u));
    out.truncate(max);
    out
}

fn fetch_pdf(url: &str) -> Result<Option<Vec<u8>>, String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(30))
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) InvoiceBox/0.1")
        .redirect(reqwest::redirect::Policy::limited(6))
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client.get(url).send().map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Ok(None);
    }
    let content_type = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_lowercase();
    let bytes = resp.bytes().map_err(|e| e.to_string())?;
    if bytes.len() > 48 * 1024 * 1024 || bytes.len() < 64 {
        return Ok(None);
    }
    if content_type.contains("pdf") || bytes.starts_with(b"%PDF") {
        return Ok(Some(bytes.to_vec()));
    }
    Ok(None)
}

fn file_label_from_url(url: &str) -> String {
    let path = url.split(['?', '#']).next().unwrap_or(url);
    let last = path.rsplit('/').next().unwrap_or("");
    if last.is_empty() || last.len() > 80 {
        "link.pdf".to_string()
    } else if last.to_lowercase().ends_with(".pdf") {
        last.to_string()
    } else {
        format!("{last}.pdf")
    }
}

fn parse_mail(raw: &[u8], cfg: &Config, log: Logger) -> Result<Option<MailItem>, String> {
    let message = MessageParser::default()
        .parse(raw)
        .ok_or_else(|| "无法解析邮件 MIME 内容".to_string())?;

    let subject = message.subject().unwrap_or("").trim().to_string();
    let (from_name, from_addr) = match message.from() {
        Some(address) => match address.first() {
            Some(addr) => (
                addr.name().unwrap_or("").trim().to_string(),
                addr.address().unwrap_or("").trim().to_string(),
            ),
            None => (String::new(), String::new()),
        },
        None => (String::new(), String::new()),
    };
    let date = message
        .date()
        .map(|d| d.to_rfc3339())
        .map(|s| s.chars().take(10).collect::<String>())
        .filter(|s| s.len() == 10)
        .unwrap_or_else(util::today);

    let mut candidates: Vec<Candidate> = Vec::new();
    for part in message.attachments() {
        let name = part
            .attachment_name()
            .map(|s| s.to_string())
            .unwrap_or_else(|| "attachment.pdf".to_string());
        let bytes = part.contents();
        let looks_pdf = name.to_lowercase().ends_with(".pdf")
            || part.is_content_type("application", "pdf")
            || bytes.starts_with(b"%PDF");
        if !looks_pdf || bytes.len() < 64 {
            continue;
        }
        candidates.push(Candidate {
            label: name.clone(),
            detail: format!("附件 {name}"),
            kind: "attachment",
            bytes: bytes.to_vec(),
        });
    }

    let keywords = parse_keywords(&cfg.keyword_filter);
    let keyword_hit = keywords.is_empty()
        || keywords.iter().any(|k| {
            util::contains_ci(&subject, k)
                || util::contains_ci(&from_name, k)
                || util::contains_ci(&from_addr, k)
        });
    if !keyword_hit && candidates.is_empty() {
        return Ok(None);
    }

    if cfg.max_links_per_mail > 0 {
        let html = message
            .body_html(0)
            .map(|c| c.to_string())
            .unwrap_or_default();
        let text = message
            .body_text(0)
            .map(|c| c.to_string())
            .unwrap_or_default();
        let links = collect_links(&html, &text, cfg.max_links_per_mail as usize);
        for url in links {
            log(&format!("  尝试下载：{}", util::truncate(&url, 110)));
            match fetch_pdf(&url) {
                Ok(Some(bytes)) => candidates.push(Candidate {
                    label: file_label_from_url(&url),
                    detail: url.clone(),
                    kind: "link",
                    bytes,
                }),
                Ok(None) => {}
                Err(e) => log(&format!("  链接下载失败：{e}")),
            }
        }
    }

    if candidates.is_empty() {
        return Ok(None);
    }
    Ok(Some(MailItem {
        uid: 0,
        subject,
        from_name,
        from_addr,
        date,
        candidates,
    }))
}

/// 连接邮箱并收集所有带 PDF 的邮件
pub fn fetch_all(cfg: &Config, log: Logger) -> Result<Vec<MailItem>, String> {
    cfg.validate()?;
    log(&format!(
        "连接 {}:{}（{}）…",
        cfg.imap_host,
        cfg.imap_port,
        if cfg.imap_tls.eq_ignore_ascii_case("starttls") {
            "STARTTLS"
        } else {
            "SSL/TLS"
        }
    ));
    let mut session = connect_session(cfg)?;
    log("登录成功");

    let mailbox = session
        .select(&cfg.folder)
        .map_err(|e| format!("打开文件夹 {} 失败: {e}", cfg.folder))?;
    log(&format!("文件夹 {} 共 {} 封邮件", cfg.folder, mailbox.exists));

    let since = util::imap_since(cfg.since_days);
    let mut uids: Vec<u32> = session
        .uid_search(format!("SINCE {since}"))
        .map_err(|e| format!("搜索邮件失败: {e}"))?
        .into_iter()
        .collect();
    uids.sort_unstable();
    let max = cfg.max_mails.max(1) as usize;
    if uids.len() > max {
        uids = uids.split_off(uids.len() - max);
    }
    log(&format!(
        "SINCE {since} 命中 {} 封，开始逐封解析…",
        uids.len()
    ));

    let mut items: Vec<MailItem> = Vec::new();
    let total = uids.len();
    for (index, chunk) in uids.chunks(15).enumerate() {
        let set = chunk
            .iter()
            .map(|u| u.to_string())
            .collect::<Vec<_>>()
            .join(",");
        let fetched = session
            .uid_fetch(set, "(UID RFC822)")
            .map_err(|e| format!("拉取邮件失败: {e}"))?;
        for message in fetched.iter() {
            let Some(body) = message.body() else {
                continue;
            };
            let uid = message.uid.unwrap_or(0);
            match parse_mail(body, cfg, log) {
                Ok(Some(mut item)) => {
                    item.uid = uid;
                    log(&format!(
                        "  ✓ {} · {} · {} 个 PDF",
                        util::truncate(&item.subject, 40),
                        if item.from_name.is_empty() {
                            &item.from_addr
                        } else {
                            &item.from_name
                        },
                        item.candidates.len()
                    ));
                    items.push(item);
                }
                Ok(None) => {}
                Err(e) => log(&format!("  UID {uid} 解析失败：{e}")),
            }
        }
        log(&format!(
            "扫描进度 {}/{}",
            ((index + 1) * 15).min(total),
            total
        ));
    }

    let _ = session.logout();
    Ok(items)
}
