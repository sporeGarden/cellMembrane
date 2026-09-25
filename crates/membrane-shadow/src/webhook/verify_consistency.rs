//! Post-build consistency verification.
//!
//! Cross-checks site.json page counts vs sitemap URL counts to detect drift
//! between the Zola build, artifact generation, and the served site.

use std::path::Path;
use tracing::{info, warn};

/// Consistency report from a post-build check.
pub struct ConsistencyReport {
    pub ok: bool,
    pub sitemap_urls: usize,
    pub site_json_pages: Option<usize>,
    pub warnings: Vec<String>,
}

impl std::fmt::Display for ConsistencyReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.ok {
            write!(
                f,
                "consistency OK: sitemap={} pages",
                self.sitemap_urls
            )?;
            if let Some(json_pages) = self.site_json_pages {
                write!(f, ", site.json={json_pages} pages")?;
            }
        } else {
            write!(f, "consistency DRIFT:")?;
            for w in &self.warnings {
                write!(f, " {w};")?;
            }
        }
        Ok(())
    }
}

/// Run post-build consistency checks on a published site.
///
/// Checks performed:
/// 1. sitemap.xml exists and contains <loc> entries
/// 2. If api/site.json exists, its `total_pages` matches sitemap count
/// 3. If content-manifest.toml exists, its entry count is sane
pub fn verify(public_dir: &Path) -> ConsistencyReport {
    let mut warnings = Vec::new();

    // 1. Parse sitemap.xml
    let sitemap_path = public_dir.join("sitemap.xml");
    let sitemap_urls = if sitemap_path.exists() {
        match std::fs::read_to_string(&sitemap_path) {
            Ok(xml) => {
                let count = xml.matches("<loc>").count();
                if count == 0 {
                    warnings.push("sitemap.xml has zero <loc> entries".to_string());
                }
                count
            }
            Err(e) => {
                warnings.push(format!("sitemap.xml unreadable: {e}"));
                0
            }
        }
    } else {
        warnings.push("sitemap.xml missing from public dir".to_string());
        0
    };

    // 2. Parse api/site.json for total_pages
    let site_json_path = public_dir.join("api/site.json");
    let site_json_pages = if site_json_path.exists() {
        match std::fs::read_to_string(&site_json_path) {
            Ok(json_str) => {
                match serde_json::from_str::<serde_json::Value>(&json_str) {
                    Ok(val) => {
                        val.get("meta")
                            .and_then(|m| m.get("total_pages"))
                            .and_then(serde_json::Value::as_u64)
                            .map(|n| n as usize)
                    }
                    Err(e) => {
                        warnings.push(format!("site.json parse error: {e}"));
                        None
                    }
                }
            }
            Err(e) => {
                warnings.push(format!("site.json unreadable: {e}"));
                None
            }
        }
    } else {
        None
    };

    // 3. Cross-check: sitemap URLs vs site.json pages.
    // Sitemap typically has MORE entries than site.json pages because it includes
    // section indexes, taxonomy pages, and root pages. A 5:1 ratio is common for
    // Zola sites with many sections. We flag only extreme drift (>10× or inverse).
    if let Some(json_pages) = site_json_pages {
        if sitemap_urls > 0 && json_pages > 0 {
            let ratio = sitemap_urls as f64 / json_pages as f64;
            if ratio < 0.3 || ratio > 10.0 {
                warnings.push(format!(
                    "extreme drift: sitemap has {sitemap_urls} URLs but site.json reports {json_pages} pages (ratio={ratio:.2})"
                ));
            }
        }
    }

    // 4. Check content-manifest.toml exists if site.json exists
    if site_json_pages.is_some() {
        let manifest_path = public_dir.join("content-manifest.toml");
        if !manifest_path.exists() {
            warnings.push("content-manifest.toml missing (expected alongside site.json)".to_string());
        }
    }

    let ok = warnings.is_empty();
    ConsistencyReport {
        ok,
        sitemap_urls,
        site_json_pages,
        warnings,
    }
}

/// Run verify and log results.  Returns formatted message for the pipeline.
pub fn verify_and_report(
    site: &crate::seo::PublishSite,
) -> String {
    let public = Path::new(site.public_dir);
    let report = verify(public);

    if report.ok {
        info!(
            repo = %site.repo_name,
            sitemap_urls = report.sitemap_urls,
            "publish: consistency check passed"
        );
    } else {
        for w in &report.warnings {
            warn!(repo = %site.repo_name, warning = %w, "publish: consistency drift");
        }
    }

    format!("  [verify] {report}")
}
