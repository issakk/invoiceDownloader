use std::collections::HashSet;
use std::fs;

use tauri::AppHandle;

use crate::config;
use crate::mail::{Candidate, MailItem};
use crate::pdf::{self, Fields, Meta};
use crate::store::{self, Invoice};
use crate::util;

pub struct Outcome {
    pub scanned_mails: usize,
    pub found_pdfs: usize,
    pub new_invoices: Vec<Invoice>,
    pub duplicates: usize,
    pub errors: Vec<String>,
}

type Logger<'a> = &'a dyn Fn(&str);

fn unique_name(base: &str, used: &mut HashSet<String>) -> String {
    let base = if base.is_empty() { "invoice" } else { base };
    let candidate = format!("{base}.pdf");
    if !used.contains(&candidate.to_lowercase()) {
        used.insert(candidate.to_lowercase());
        return candidate;
    }
    for index in 2..1000 {
        let candidate = format!("{base}-{index}.pdf");
        if !used.contains(&candidate.to_lowercase()) {
            used.insert(candidate.to_lowercase());
            return candidate;
        }
    }
    let candidate = format!("{base}-{}.pdf", util::sha256_hex(base.as_bytes()));
    used.insert(candidate.to_lowercase());
    candidate
}

/// 用模板渲染文件名，缺失字段自动省略（连续分隔符会被折叠）
pub fn render_name(template: &str, fields: &Fields, meta: &Meta, kind: &str, hash: &str) -> String {
    let template = if template.trim().is_empty() {
        "{date}_{amount}_{seller}_{no}"
    } else {
        template
    };
    let from = if meta.from_name.is_empty() {
        meta.from_addr
    } else {
        meta.from_name
    };
    let mut out = template.to_string();
    for (token, value) in [
        ("{date}", fields.date.as_str()),
        ("{amount}", fields.amount.as_str()),
        ("{seller}", fields.seller.as_str()),
        ("{no}", fields.no.as_str()),
        ("{subject}", meta.subject),
        ("{from}", from),
        ("{kind}", kind),
    ] {
        out = out.replace(token, value);
    }
    let cleaned = util::sanitize_file_name(&out, 90);
    if cleaned.is_empty() {
        format!("invoice-{}", hash.chars().take(8).collect::<String>())
    } else {
        cleaned
    }
}

fn stem_of(name: &str) -> String {
    let file = name.rsplit('/').next().unwrap_or(name);
    match file.rfind('.') {
        Some(pos) => file[..pos].to_string(),
        None => file.to_string(),
    }
}

fn ingest_one(
    dir: &std::path::Path,
    cfg: &crate::config::Config,
    mail: &MailItem,
    candidate: &Candidate,
    hash: &str,
    used_names: &mut HashSet<String>,
) -> Result<Invoice, String> {
    let text = pdf::extract_text(&candidate.bytes);
    let stem = stem_of(&candidate.label);
    let meta = Meta {
        subject: &mail.subject,
        from_name: &mail.from_name,
        from_addr: &mail.from_addr,
        file_stem: &stem,
        mail_date: &mail.date,
    };
    let fields = pdf::parse_fields(&text, &meta);
    let base = render_name(&cfg.naming_template, &fields, &meta, candidate.kind, hash);
    let file_name = unique_name(&base, used_names);
    let path = dir.join(&file_name);
    fs::write(&path, &candidate.bytes).map_err(|e| format!("写入 {file_name} 失败: {e}"))?;

    Ok(Invoice {
        hash: hash.to_string(),
        file_name,
        path: path.to_string_lossy().to_string(),
        date: fields.date,
        amount: fields.amount,
        seller: fields.seller,
        no: fields.no,
        subject: mail.subject.clone(),
        from: mail.from_name.clone(),
        from_addr: mail.from_addr.clone(),
        source_kind: candidate.kind.to_string(),
        source_detail: candidate.detail.clone(),
        created_at: util::now_stamp(),
        size: candidate.bytes.len() as u64,
        text_chars: text.chars().count(),
        warnings: fields.warnings,
    })
}

pub fn ingest(
    app: &AppHandle,
    cfg: &config::Config,
    mails: Vec<MailItem>,
    log: Logger,
) -> Result<Outcome, String> {
    let mut manifest = store::load(app)?;
    let mut hashes: HashSet<String> = manifest.hashes();
    let mut used_names: HashSet<String> = manifest
        .invoices
        .iter()
        .map(|i| i.file_name.to_lowercase())
        .collect();
    let dir = config::invoices_dir(app)?;

    let mut outcome = Outcome {
        scanned_mails: mails.len(),
        found_pdfs: 0,
        new_invoices: Vec::new(),
        duplicates: 0,
        errors: Vec::new(),
    };

    for mail in &mails {
        for candidate in &mail.candidates {
            outcome.found_pdfs += 1;
            let hash = util::sha256_hex(&candidate.bytes);
            if hashes.contains(&hash) {
                outcome.duplicates += 1;
                log(&format!("  ↺ 已存在，跳过 {}", candidate.label));
                continue;
            }
            match ingest_one(&dir, cfg, mail, candidate, &hash, &mut used_names) {
                Ok(invoice) => {
                    log(&format!(
                        "  ＋ {} ← {} · {}",
                        invoice.file_name,
                        if invoice.seller.is_empty() {
                            "未知销售方"
                        } else {
                            &invoice.seller
                        },
                        if invoice.amount.is_empty() {
                            "金额未知"
                        } else {
                            &invoice.amount
                        }
                    ));
                    hashes.insert(hash);
                    manifest.invoices.push(invoice.clone());
                    outcome.new_invoices.push(invoice);
                    if manifest.invoices.len() % 5 == 0 {
                        store::save(app, &manifest)?;
                    }
                }
                Err(e) => {
                    outcome.errors.push(e.clone());
                    log(&format!("  ✕ {e}"));
                }
            }
        }
    }

    store::save(app, &manifest)?;
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fields(seller: &str) -> Fields {
        Fields {
            date: "2024-03-01".into(),
            amount: "1300.00".into(),
            seller: seller.into(),
            no: "24312000000123456789".into(),
            warnings: vec![],
        }
    }

    fn meta<'a>() -> Meta<'a> {
        Meta {
            subject: "您的电子发票",
            from_name: "腾讯云",
            from_addr: "noreply@tencent.com",
            file_stem: "invoice",
            mail_date: "2024-03-02",
        }
    }

    #[test]
    fn renders_default_template() {
        let name = render_name(
            "{date}_{amount}_{seller}_{no}",
            &fields("腾讯科技有限公司"),
            &meta(),
            "attachment",
            "abcdef1234567890",
        );
        assert_eq!(name, "2024-03-01_1300.00_腾讯科技有限公司_24312000000123456789");
    }

    #[test]
    fn collapses_separators_when_field_missing() {
        let name = render_name(
            "{date}_{amount}_{seller}_{no}",
            &fields(""),
            &meta(),
            "attachment",
            "abcdef1234567890",
        );
        assert_eq!(name, "2024-03-01_1300.00_24312000000123456789");
    }

    #[test]
    fn avoids_duplicate_file_names() {
        let mut used = std::collections::HashSet::new();
        assert_eq!(unique_name("a", &mut used), "a.pdf");
        assert_eq!(unique_name("a", &mut used), "a-2.pdf");
        assert_eq!(unique_name("a", &mut used), "a-3.pdf");
    }

    #[test]
    fn falls_back_to_hash_when_template_is_empty_after_render() {
        let empty = Fields::default();
        let name = render_name("{seller}", &empty, &meta(), "attachment", "abcdef1234567890");
        assert_eq!(name, "invoice-abcdef12");
    }
}
