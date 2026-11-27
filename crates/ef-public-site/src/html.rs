//! Semantic, indexable HTML rendering helpers with strict escaping and zero canvas markup.

/// Escapes HTML special characters in text or attribute values.
#[must_use]
pub fn escape_html(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for ch in raw.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
    out
}

/// Robots indexing policy for a rendered semantic HTML document.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RobotsPolicy {
    IndexFollow,
    NoIndexNoFollow,
}

impl RobotsPolicy {
    #[must_use]
    pub const fn meta_content(self) -> &'static str {
        match self {
            Self::IndexFollow => "index, follow",
            Self::NoIndexNoFollow => "noindex, nofollow",
        }
    }
}

/// Renders a complete semantic HTML5 page with landmarks, navigation, and zero canvas elements.
#[must_use]
pub fn render_semantic_page(
    title: &str,
    description: &str,
    canonical_path: &str,
    robots: RobotsPolicy,
    main_html: &str,
) -> String {
    let title_esc = escape_html(title);
    let desc_esc = escape_html(description);
    let path_esc = escape_html(canonical_path);
    let robots_meta = robots.meta_content();

    format!(
        "<!DOCTYPE html>\n\
<html lang=\"en\">\n\
<head>\n\
  <meta charset=\"utf-8\">\n\
  <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
  <title>{title_esc}</title>\n\
  <meta name=\"description\" content=\"{desc_esc}\">\n\
  <meta name=\"robots\" content=\"{robots_meta}\">\n\
  <link rel=\"canonical\" href=\"{path_esc}\">\n\
</head>\n\
<body>\n\
  <header>\n\
    <nav aria-label=\"Primary\">\n\
      <a href=\"/\">Elysian Fields</a>\n\
      <a href=\"/mobile\">Curie Mobile</a>\n\
      <a href=\"/recovery\">Dei Recovery Finder</a>\n\
      <a href=\"/demo\">Interactive Demo</a>\n\
      <a href=\"/privacy\">Privacy</a>\n\
      <a href=\"/terms\">Terms</a>\n\
      <a href=\"/clinical-transparency\">Clinical Transparency</a>\n\
    </nav>\n\
  </header>\n\
  <main id=\"main-content\">\n\
{main_html}\n\
  </main>\n\
  <footer>\n\
    <p>Elysian Fields Public Information &amp; Legal Archive.</p>\n\
  </footer>\n\
</body>\n\
</html>\n"
    )
}
