//! The extraction script.
//!
//! # Why this runs in the page rather than over the DOM protocol
//!
//! Extraction needs the rendered result: what is visible, in what order, with
//! which text. CDP could walk the DOM node by node, but every question worth
//! asking — is this displayed, is this the article or the cookie banner, does
//! this heading precede that paragraph — is a computed-style or layout question,
//! and each one would be a protocol round trip. In the page it is one call and
//! one traversal.

/// Extracts the page, or a subtree of it, as text or Markdown.
///
/// Returns a string. The Markdown it produces is deliberately plain — headings,
/// links, list items, code, and paragraphs — because it is read by a model
/// rather than rendered. Anything more elaborate spends tokens on syntax.
pub(crate) const EXTRACT: &str = r#"
function (format, selector) {
  const root = selector ? document.querySelector(selector) : document.body;
  if (!root) return null;

  const SKIP = new Set(['SCRIPT', 'STYLE', 'NOSCRIPT', 'TEMPLATE', 'SVG', 'CANVAS', 'IFRAME']);
  const BLOCK = new Set([
    'P', 'DIV', 'SECTION', 'ARTICLE', 'HEADER', 'FOOTER', 'MAIN', 'ASIDE', 'NAV',
    'UL', 'OL', 'LI', 'TABLE', 'TR', 'BLOCKQUOTE', 'PRE', 'FIGURE', 'FORM', 'BR', 'HR',
  ]);

  const hidden = (node) => {
    const style = window.getComputedStyle(node);
    return style.display === 'none' || style.visibility === 'hidden' || node.hidden;
  };

  const out = [];
  const push = (text) => {
    const trimmed = text.replace(/[ \t]+/g, ' ').trim();
    if (trimmed) out.push(trimmed);
  };

  const walk = (node, depth) => {
    if (node.nodeType === Node.TEXT_NODE) {
      push(node.nodeValue);
      return;
    }
    if (node.nodeType !== Node.ELEMENT_NODE) return;
    if (SKIP.has(node.tagName)) return;
    if (hidden(node)) return;

    const tag = node.tagName;

    if (format === 'markdown') {
      if (/^H[1-6]$/.test(tag)) {
        push('\n' + '#'.repeat(Number(tag[1])) + ' ' + node.innerText.trim() + '\n');
        return;
      }
      if (tag === 'A') {
        const href = node.getAttribute('href');
        const text = node.innerText.trim();
        if (text) push(href ? '[' + text + '](' + node.href + ')' : text);
        return;
      }
      if (tag === 'LI') {
        push('- ' + node.innerText.trim().replace(/\n+/g, ' '));
        return;
      }
      if (tag === 'PRE') {
        push('\n```\n' + node.innerText.replace(/\n+$/, '') + '\n```\n');
        return;
      }
      if (tag === 'CODE' && node.parentElement && node.parentElement.tagName !== 'PRE') {
        push('`' + node.innerText.trim() + '`');
        return;
      }
      if (tag === 'IMG') {
        const alt = node.getAttribute('alt');
        if (alt) push('![' + alt + '](' + node.src + ')');
        return;
      }
      if (tag === 'BLOCKQUOTE') {
        push('> ' + node.innerText.trim().replace(/\n+/g, '\n> '));
        return;
      }
    }

    // A depth bound rather than a visited set: this walks a tree, so it cannot
    // cycle, but a pathological page can nest thousands of divs and blow the
    // stack on the way down.
    if (depth > 200) return;

    for (const child of node.childNodes) walk(child, depth + 1);
    if (BLOCK.has(tag)) out.push('');
  };

  walk(root, 0);

  return out
    .join('\n')
    .replace(/\n{3,}/g, '\n\n')
    .trim();
}
"#;
