use regex::Regex;
use std::panic::{catch_unwind, AssertUnwindSafe};

use crate::util;

/// 从 PDF 字节中提取纯文本；解析失败或崩溃时返回空串
pub fn extract_text(bytes: &[u8]) -> String {
    let result = catch_unwind(AssertUnwindSafe(|| {
        pdf_extract::extract_text_from_mem(bytes)
    }));
    match result {
        Ok(Ok(text)) => text,
        _ => String::new(),
    }
}

#[derive(Debug, Default, Clone)]
pub struct Fields {
    pub date: String,
    pub amount: String,
    pub seller: String,
    pub no: String,
    pub warnings: Vec<String>,
}

/// 邮件与文件名提供的兜底线索
pub struct Meta<'a> {
    pub subject: &'a str,
    pub from_name: &'a str,
    pub from_addr: &'a str,
    pub file_stem: &'a str,
    pub mail_date: &'a str,
}

fn re(pattern: &str) -> Regex {
    Regex::new(pattern).expect("内置正则必须合法")
}

fn cap(re: &Regex, text: &str, group: usize) -> Option<String> {
    re.captures(text)
        .and_then(|c| c.get(group))
        .map(|m| m.as_str().trim().to_string())
        .filter(|s| !s.is_empty())
}

fn pad2(s: &str) -> String {
    let n: u32 = s.trim().parse().unwrap_or(0);
    format!("{:02}", n)
}

fn ymd(y: &str, m: &str, d: &str) -> String {
    format!("{}-{}-{}", y.trim(), pad2(m), pad2(d))
}

/// `1299.0` / `1,299.00` / `¥1,299.00` -> `1299.00`
pub fn clean_amount(raw: &str) -> String {
    let cleaned: String = raw
        .chars()
        .filter(|c| c.is_ascii_digit() || *c == '.' || *c == ',')
        .collect::<String>()
        .replace(',', "");
    if cleaned.is_empty() {
        return String::new();
    }
    match cleaned.find('.') {
        Some(dot) => {
            let int = &cleaned[..dot];
            let frac: String = cleaned[dot + 1..]
                .chars()
                .filter(|c| c.is_ascii_digit())
                .take(2)
                .collect();
            if frac.is_empty() {
                int.to_string()
            } else if frac.len() == 1 {
                format!("{int}.{frac}0")
            } else {
                format!("{int}.{frac}")
            }
        }
        None => cleaned,
    }
}

fn cut_tail(s: &str) -> String {
    const STOP: [&str; 8] = [
        "统一社会信用代码",
        "纳税人识别号",
        "纳税人识别",
        "地址",
        "电话",
        "开户行",
        "账号",
        "税号",
    ];
    let mut end = s.len();
    for stop in STOP {
        if let Some(pos) = s.find(stop) {
            end = end.min(pos);
        }
    }
    s[..end].trim().to_string()
}

fn cn_date(flat: &str, text: &str) -> Option<String> {
    let labeled = re(r"开票日期[：:]?(20\d{2})年(\d{1,2})月(\d{1,2})日");
    if let Some(c) = labeled.captures(flat) {
        return Some(ymd(&c[1], &c[2], &c[3]));
    }
    let cn = re(r"(20\d{2})年(\d{1,2})月(\d{1,2})日");
    if let Some(c) = cn.captures(flat) {
        return Some(ymd(&c[1], &c[2], &c[3]));
    }
    let dashed = re(r"(20\d{2})[-/.](\d{1,2})[-/.](\d{1,2})");
    if let Some(c) = dashed.captures(text) {
        return Some(ymd(&c[1], &c[2], &c[3]));
    }
    None
}

fn cn_invoice_no(flat: &str) -> Option<String> {
    let labeled = re(r"发票号码[：:]?([0-9]{8,24})");
    cap(&labeled, flat, 1)
}

fn cn_amount(flat: &str) -> Option<String> {
    let patterns = [
        r"价税合计[（(]小写[)）][：:]?[¥￥]?([0-9,]+\.\d{1,2})",
        r"价税合计[^0-9]{0,12}[¥￥]?([0-9,]+\.\d{1,2})",
        r"[（(]小写[)）][：:]?[¥￥]?([0-9,]+\.\d{1,2})",
        r"合计[^0-9]{0,10}[¥￥]?([0-9,]+\.\d{1,2})",
        r"金额合计[：:]?[¥￥]?([0-9,]+\.\d{1,2})",
    ];
    for p in patterns {
        if let Some(v) = cap(&re(p), flat, 1) {
            let cleaned = clean_amount(&v);
            if !cleaned.is_empty() {
                return Some(cleaned);
            }
        }
    }
    None
}

fn cn_seller(text: &str, flat: &str) -> Option<String> {
    let labeled = [
        r"销售方[^\n：:]{0,12}[：:]\s*([^\n]{2,60})",
        r"销售方(?:信息)?名称[：:]?([^\n]{2,60})",
    ];
    for p in labeled {
        if let Some(v) = cap(&re(p), text, 1) {
            let cut = cut_tail(&v);
            if cut.chars().count() >= 2 {
                return Some(util::normalize_ws(&cut));
            }
        }
    }
    // 全电发票把标签与值压在同一行，退化为在紧凑文本里找
    let flat_labeled = re(r"销售方(?:信息)?名称[：:]?([\u4e00-\u9fa5A-Za-z0-9（）()·]{4,40})");
    if let Some(v) = cap(&flat_labeled, flat, 1) {
        return Some(v);
    }
    // 兜底：取文中最后一个「公司/厂/店」形态的名称（发票版式通常销售方在右侧/靠后）
    let company = re(
        r"([\u4e00-\u9fa5（）()·]{2,24}(?:股份有限公司|有限责任公司|有限公司|公司|分公司|商行|事务所|中心|集团|厂|店))",
    );
    company
        .captures_iter(text)
        .last()
        .map(|c| c[1].to_string())
}

fn en_invoice_no(text: &str) -> Option<String> {
    let patterns = [
        r"(?i)(?:invoice|receipt|bill)\s*(?:number|num|nos?|#|号码|编号)\.?\s*[:#]?\s*([A-Za-z0-9][A-Za-z0-9_\-/]{1,22})",
        r"(?i)发票号码\s*[:：]?\s*([A-Za-z0-9][A-Za-z0-9_\-/]{1,22})",
    ];
    for p in patterns {
        if let Some(v) = cap(&re(p), text, 1) {
            let v = v.trim_end_matches(['.', ',', ')', '。']).to_string();
            if v.chars().any(|c| c.is_ascii_digit()) {
                return Some(v);
            }
        }
    }
    None
}

fn en_date(text: &str) -> Option<String> {
    let labeled = re(
        r"(?i)(?:invoice\s*date|date\s*of\s*issue|issue\s*date|billing\s*date|开票日期)\s*[:：]?\s*(\d{4})[-/.](\d{1,2})[-/.](\d{1,2})",
    );
    if let Some(c) = labeled.captures(text) {
        return Some(ymd(&c[1], &c[2], &c[3]));
    }
    let labeled_slash = re(
        r"(?i)(?:invoice\s*date|date\s*of\s*issue|issue\s*date|billing\s*date)\s*[:：]?\s*(\d{1,2})/(\d{1,2})/(\d{4})",
    );
    if let Some(c) = labeled_slash.captures(text) {
        return Some(ymd(&c[3], &c[1], &c[2]));
    }
    let any = re(r"(20\d{2})[-/.](\d{1,2})[-/.](\d{1,2})");
    if let Some(c) = any.captures(text) {
        return Some(ymd(&c[1], &c[2], &c[3]));
    }
    None
}

fn en_amount(text: &str) -> Option<String> {
    let patterns = [
        r"(?i)(?:grand\s*total|total\s*amount|amount\s*due|total\s*due|balance\s*due)\s*[:：]?\s*[A-Z]{0,3}\s?[$€£¥￥]?\s?([0-9][0-9,]*\.\d{2})",
        r"(?i)(?:total|amount|合计|总计)\s*[:：]?\s*[A-Z]{0,3}\s?[$€£¥￥]?\s?([0-9][0-9,]*\.\d{2})",
        r"[$€£¥￥]\s?([0-9][0-9,]*\.\d{2})",
    ];
    for p in patterns {
        if let Some(v) = cap(&re(p), text, 1) {
            let cleaned = clean_amount(&v);
            if !cleaned.is_empty() {
                return Some(cleaned);
            }
        }
    }
    None
}

fn en_seller(text: &str) -> Option<String> {
    let patterns = [
        r"(?i)(?:seller|vendor|supplier|merchant|sold\s*by)\s*[:：]\s*([^\n]{2,60})",
        r"(?i)seller\s+name\s*[:：]\s*([^\n]{2,60})",
    ];
    for p in patterns {
        if let Some(v) = cap(&re(p), text, 1) {
            let cut = util::normalize_ws(&cut_tail(&v));
            if cut.chars().count() >= 2 {
                return Some(cut);
            }
        }
    }
    None
}

/// 从发件人推导一个可读的销售方名称
fn seller_from_mail(meta: &Meta) -> String {
    let name = meta.from_name.trim();
    if !name.is_empty() && !name.contains('@') {
        return util::sanitize_file_name(name, 40);
    }
    let domain = meta
        .from_addr
        .rsplit('@')
        .next()
        .unwrap_or("")
        .to_lowercase();
    let labels: Vec<&str> = domain.split('.').filter(|s| !s.is_empty()).collect();
    let brand = if labels.len() >= 2 {
        labels[labels.len() - 2]
    } else {
        labels.first().copied().unwrap_or("")
    };
    let brand = brand
        .trim_start_matches("no-reply")
        .trim_start_matches("noreply")
        .trim_matches('-')
        .trim_matches('_');
    if brand.is_empty() {
        String::new()
    } else {
        let mut chars = brand.chars();
        match chars.next() {
            Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
            None => String::new(),
        }
    }
}

pub fn parse_fields(text: &str, meta: &Meta) -> Fields {
    let mut fields = Fields::default();
    let flat = util::squish(text);

    fields.date = cn_date(&flat, text)
        .or_else(|| en_date(text))
        .unwrap_or_default();
    fields.no = cn_invoice_no(&flat)
        .or_else(|| en_invoice_no(text))
        .unwrap_or_default();
    fields.amount = cn_amount(&flat)
        .or_else(|| en_amount(text))
        .unwrap_or_default();
    fields.seller = cn_seller(text, &flat)
        .or_else(|| en_seller(text))
        .unwrap_or_default();

    if fields.date.is_empty() {
        fields.date = meta.mail_date.to_string();
        fields.warnings.push("日期取自邮件时间".into());
    }
    if fields.seller.is_empty() {
        fields.seller = seller_from_mail(meta);
        if !fields.seller.is_empty() {
            fields.warnings.push("销售方取自发件人".into());
        } else {
            fields.warnings.push("未识别销售方".into());
        }
    }
    if fields.no.is_empty() {
        if let Some(digits) = util::first_digits(meta.file_stem, 6, 24) {
            fields.no = digits;
            fields.warnings.push("发票号取自文件名".into());
        } else {
            fields.warnings.push("未识别发票号".into());
        }
    }
    if fields.amount.is_empty() {
        fields.warnings.push("未识别金额".into());
    }
    if text.trim().is_empty() {
        fields.warnings.push("PDF 无可提取文本（扫描件？）".into());
    }
    fields
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta<'a>() -> Meta<'a> {
        Meta {
            subject: "您的电子发票",
            from_name: "腾讯云计算",
            from_addr: "noreply@tencent.com",
            file_stem: "invoice",
            mail_date: "2024-03-02",
        }
    }

    const CN_EW: &str = "电子发票（普通发票）
发票号码：24312000000123456789
开票日期：2024年03月01日
购买方信息 名称：北京某某科技有限公司 统一社会信用代码/纳税人识别号：91110108MA01XXXXXX
销售方信息 名称：腾讯科技（深圳）有限公司 统一社会信用代码/纳税人识别号：91440300708461136T
*信息技术服务*云服务费 1 1 1,226.42 6% 73.58
价税合计（大写）壹仟叁佰元整 （小写）¥1,300.00
备注 订单号 202403010001";

    #[test]
    fn parses_chinese_e_invoice() {
        let fields = parse_fields(CN_EW, &meta());
        assert_eq!(fields.date, "2024-03-01");
        assert_eq!(fields.no, "24312000000123456789");
        assert_eq!(fields.amount, "1300.00");
        assert_eq!(fields.seller, "腾讯科技（深圳）有限公司");
        assert!(fields.warnings.is_empty(), "warnings: {:?}", fields.warnings);
    }

    const EN_INVOICE: &str = "INVOICE
Invoice Number: INV-2024-0042
Invoice Date: 2024-02-15
Seller: Acme Cloud Inc.
Total Due: $1,250.00";

    #[test]
    fn parses_english_invoice() {
        let fields = parse_fields(EN_INVOICE, &meta());
        assert_eq!(fields.date, "2024-02-15");
        assert_eq!(fields.no, "INV-2024-0042");
        assert_eq!(fields.amount, "1250.00");
        assert_eq!(fields.seller, "Acme Cloud Inc.");
    }

    #[test]
    fn falls_back_to_mail_metadata() {
        let fields = parse_fields("", &meta());
        assert_eq!(fields.date, "2024-03-02");
        assert_eq!(fields.seller, "腾讯云计算");
        assert!(fields.warnings.iter().any(|w| w.contains("无可提取文本")));
    }

    #[test]
    fn normalizes_amounts() {
        assert_eq!(clean_amount("¥1,299.00"), "1299.00");
        assert_eq!(clean_amount("1299.0"), "1299.00");
        assert_eq!(clean_amount("1,299"), "1299");
        assert_eq!(clean_amount("abc"), "");
    }
}
