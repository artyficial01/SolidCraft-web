//! The library page: every stored design and its versions, as plain HTML (no scripts) to browse
//! from any device and download from.

use std::fmt::Write as _;

use crate::store::{Store, StoreError};

/// Percent-encode for a URL path segment or an RFC 5987 file name.
pub fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            let _ = write!(out, "%{b:02X}");
        }
    }
    out
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&#39;")
}

/// `2026-10-14 19:30:05 UTC` from milliseconds since 1970 (civil-from-days, no date crate).
pub fn format_utc(ms: u64) -> String {
    let secs = ms / 1000;
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (h, m, s) = (rem / 3600, rem % 3600 / 60, rem % 60);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{d:02} {h:02}:{m:02}:{s:02} UTC")
}

fn size(bytes: u64) -> String {
    match bytes {
        b if b >= 1 << 20 => format!("{:.1} MB", b as f64 / (1u64 << 20) as f64),
        b if b >= 1 << 10 => format!("{:.1} KB", b as f64 / 1024.0),
        b => format!("{b} B"),
    }
}

pub fn render(store: &Store) -> Result<String, StoreError> {
    let current = store.list()?;
    let mut rows = String::new();
    for name in store.all_names()? {
        let info = current.iter().find(|f| f.name == name);
        let enc = url_encode(&name);
        let versions = store.versions(&name)?;
        let head = match info {
            Some(f) => format!(
                "<a href=\"api/files/{enc}\">{}</a> <span class=meta>{} &middot; {}</span>",
                escape(&name),
                size(f.size),
                format_utc(f.modified)
            ),
            None => format!("{} <span class=meta>deleted</span>", escape(&name)),
        };
        let _ = write!(rows, "<li><div class=name>{head}</div>");
        if !versions.is_empty() {
            let _ = write!(rows, "<details><summary>{} earlier version(s)</summary><ul>", versions.len());
            for v in &versions {
                let _ = write!(
                    rows,
                    "<li><a href=\"api/files/{enc}/versions/{ts}\">{when}</a> <span class=meta>{sz}</span></li>",
                    ts = v.ts,
                    when = format_utc(v.ts),
                    sz = size(v.size)
                );
            }
            rows.push_str("</ul></details>");
        }
        rows.push_str("</li>");
    }
    if rows.is_empty() {
        rows.push_str("<li class=meta>No designs stored yet. Save one in SolidCraft and it appears here.</li>");
    }
    Ok(format!(
        "<!doctype html><html lang=en><head><meta charset=utf-8><meta name=viewport content=\"width=device-width,initial-scale=1\">\
<title>SolidCraft library</title><style>\
:root{{color-scheme:light dark;--bg:#fff;--fg:#1c1e21;--dim:#656a73;--line:#d9dce1;--link:#1a56db}}\
@media (prefers-color-scheme:dark){{:root{{--bg:#16181c;--fg:#e6e8eb;--dim:#9aa0aa;--line:#2c3037;--link:#7aa7ff}}}}\
body{{margin:0;background:var(--bg);color:var(--fg);font:16px/1.5 system-ui,sans-serif}}\
main{{max-width:46rem;margin:0 auto;padding:1.5rem 16px}}h1{{font-size:1.4rem;margin:0 0 .25rem}}\
p{{color:var(--dim);margin:.25rem 0 1.25rem}}ul{{list-style:none;padding:0;margin:0}}\
main>ul>li{{padding:.75rem 0;border-top:1px solid var(--line)}}details ul{{padding:.25rem 0 0 1rem}}\
a{{color:var(--link)}}.meta{{color:var(--dim);font-size:.9rem}}summary{{cursor:pointer;color:var(--dim);font-size:.9rem}}\
</style></head><body><main><h1>SolidCraft library</h1>\
<p>Every design saved from the app, with the versions each save replaced. Download a file and open it in SolidCraft.</p>\
<ul>{rows}</ul></main></body></html>"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_names_for_urls() {
        assert_eq!(url_encode("My part (2).solidcraft"), "My%20part%20%282%29.solidcraft");
        assert_eq!(url_encode("a/b"), "a%2Fb");
    }

    #[test]
    fn escapes_markup_in_names() {
        assert_eq!(escape("<b onclick=\"x\">&'"), "&lt;b onclick=&quot;x&quot;&gt;&amp;&#39;");
    }

    #[test]
    fn formats_dates() {
        assert_eq!(format_utc(0), "1970-01-01 00:00:00 UTC");
        assert_eq!(format_utc(1_000_000_000_000), "2001-09-09 01:46:40 UTC");
        assert_eq!(format_utc(1_781_393_405_000), "2026-06-13 23:30:05 UTC");
    }
}
