use chrono::{Duration, Local};

/// 本地时间字符串 `YYYY-MM-DD HH:MM:SS`
pub fn now_stamp() -> String {
    Local::now().format("%Y-%m-%d %H:%M:%S").to_string()
}

/// 本地日期 `YYYY-MM-DD`
pub fn today() -> String {
    Local::now().format("%Y-%m-%d").to_string()
}

/// IMAP SEARCH 需要的日期格式，例如 `01-Mar-2024`
pub fn imap_since(days: u32) -> String {
    let days = days.clamp(1, 3650) as i64;
    let d = Local::now() - Duration::days(days);
    d.format("%d-%b-%Y").to_string()
}

/// 按字符数截断，保留可读前缀
pub fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max).collect();
    out.push('…');
    out
}

/// 文件名安全化：去掉非法字符、折叠分隔符、限制长度
pub fn sanitize_file_name(raw: &str, max_len: usize) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut last_sep = false;
    for ch in raw.chars() {
        let c = match ch {
            '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '-',
            c if (c as u32) < 0x20 => '-',
            c => c,
        };
        if c == '\n' || c == '\r' || c == '\t' {
            continue;
        }
        let is_sep = c == '-' || c == '_' || c == ' ';
        if is_sep && last_sep {
            continue;
        }
        last_sep = is_sep;
        out.push(c);
    }
    let trimmed = out.trim().trim_matches(|c| c == '_' || c == '-' || c == '.' || c == ' ');
    let trimmed = trimmed.replace(' ', "_");
    let mut final_name: String = trimmed.chars().take(max_len).collect();
    final_name = final_name.trim_end_matches(|c| c == '_' || c == '-' || c == '.').to_string();
    final_name
}

/// 压缩空白为单空格，去掉多余换行
pub fn normalize_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// 去掉所有空白字符，用于中文发票的紧凑文本匹配
pub fn squish(s: &str) -> String {
    s.chars().filter(|c| !c.is_whitespace() && *c != '\u{3000}').collect()
}

/// 小写包含判断
pub fn contains_ci(haystack: &str, needle: &str) -> bool {
    haystack.to_lowercase().contains(&needle.to_lowercase())
}

/// 16 进制 sha256
pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(64);
    for b in digest.iter() {
        out.push_str(&format!("{:02x}", b));
    }
    out
}

/// 从字符串中提取第一段连续数字（长度 >= min_len）
pub fn first_digits(s: &str, min_len: usize, max_len: usize) -> Option<String> {
    let mut run = String::new();
    for ch in s.chars() {
        if ch.is_ascii_digit() {
            run.push(ch);
        } else {
            if run.len() >= min_len && run.len() <= max_len {
                return Some(run);
            }
            run.clear();
        }
    }
    if run.len() >= min_len && run.len() <= max_len {
        Some(run)
    } else {
        None
    }
}
