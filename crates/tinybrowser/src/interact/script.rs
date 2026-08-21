//! The JavaScript this crate runs inside the page.
//!
//! # Why any JavaScript at all
//!
//! CDP can find a node and dispatch an input event at a coordinate, but it
//! cannot answer "is this covered", "what does this element read as", or "which
//! of these forty buttons says *Submit*" — those are questions about layout and
//! computed text, and the only place both are already known is the page itself.
//!
//! Each function here is written to be called with the target element as
//! `this`, via `Runtime.callFunctionOn`. They are collected in one file so the
//! JavaScript in this crate is a short list somebody can read, rather than a
//! string literal buried in every function that needs one.

/// Scrolls the element into view and reports where it can be clicked.
///
/// Returns `{ ok, x, y, reason }`. The occlusion test is the reason this exists:
/// dispatching a click at a point covered by a consent banner delivers the click
/// to the banner, and the caller is told the click succeeded. Naming the
/// covering element turns that into something an agent can act on — dismiss the
/// banner, then retry.
pub(crate) const CLICK_POINT: &str = r#"
function () {
  const describe = (node) => {
    if (!node || !node.tagName) return 'another element';
    const tag = node.tagName.toLowerCase();
    const id = node.id ? '#' + node.id : '';
    const cls = typeof node.className === 'string' && node.className.trim()
      ? '.' + node.className.trim().split(/\s+/).slice(0, 2).join('.')
      : '';
    const text = (node.innerText || '').trim().slice(0, 40);
    return tag + id + cls + (text ? ' \"' + text + '\"' : '');
  };

  this.scrollIntoView({ block: 'center', inline: 'center' });
  const rect = this.getBoundingClientRect();
  if (rect.width === 0 || rect.height === 0) {
    return { ok: false, reason: 'element has zero size, so there is nothing to click' };
  }

  const x = rect.left + rect.width / 2;
  const y = rect.top + rect.height / 2;
  if (x < 0 || y < 0 || x > window.innerWidth || y > window.innerHeight) {
    return { ok: false, reason: 'element is outside the viewport even after scrolling to it' };
  }

  const top = document.elementFromPoint(x, y);
  if (top && top !== this && !this.contains(top) && !top.contains(this)) {
    return { ok: false, x, y, reason: 'covered by ' + describe(top) };
  }

  return { ok: true, x, y };
}
"#;

/// Clears a field and selects whatever is left, so an insertion replaces it.
///
/// `select()` covers `<input>` and `<textarea>`; the range selection covers a
/// `contenteditable`, which has no `select` and is otherwise filled by appending
/// to what is already there.
pub(crate) const SELECT_ALL: &str = r"
function () {
  this.focus();
  if (typeof this.select === 'function') {
    this.select();
    return true;
  }
  const range = document.createRange();
  range.selectNodeContents(this);
  const selection = window.getSelection();
  selection.removeAllRanges();
  selection.addRange(range);
  return true;
}
";

/// Reads an element's text the way it reads on screen.
///
/// `innerText` rather than `textContent`: the second returns the text of hidden
/// nodes and ignores line breaks introduced by layout, so a caller comparing it
/// with what a person sees finds neither the same content nor the same shape.
pub(crate) const TEXT: &str = r"
function () {
  if (this.value !== undefined && this.tagName && /^(INPUT|TEXTAREA|SELECT)$/.test(this.tagName)) {
    return String(this.value);
  }
  return (this.innerText || this.textContent || '').trim();
}
";

/// Reads one attribute, or null when the element does not carry it.
pub(crate) const ATTRIBUTE: &str = r"
function (name) {
  const value = this.getAttribute(name);
  return value === null ? null : String(value);
}
";

/// Whether the element is rendered: in the document, with a box, and not hidden.
pub(crate) const IS_VISIBLE: &str = r"
function () {
  if (!this.isConnected) return false;
  const style = window.getComputedStyle(this);
  if (style.visibility === 'hidden' || style.display === 'none' || style.opacity === '0') {
    return false;
  }
  const rect = this.getBoundingClientRect();
  return rect.width > 0 && rect.height > 0;
}
";

/// Chooses options in a `<select>` and fires the events a form listens for.
///
/// Setting `selected` without dispatching leaves the page's own state untouched:
/// a framework-controlled select re-renders back to its previous value the
/// moment anything else changes.
pub(crate) const SELECT_OPTIONS: &str = r"
function (values) {
  if (!this.options) throw new Error('element is not a select');
  const wanted = new Set(values);
  let matched = 0;
  for (const option of this.options) {
    const chosen = wanted.has(option.value) || wanted.has(option.label);
    option.selected = chosen;
    if (chosen) matched += 1;
  }
  if (matched === 0) throw new Error('no option matched ' + JSON.stringify(values));
  this.dispatchEvent(new Event('input', { bubbles: true }));
  this.dispatchEvent(new Event('change', { bubbles: true }));
  return matched;
}
";

/// Whether a checkbox or radio is already in the wanted state.
pub(crate) const CHECKED: &str = r"
function () {
  return Boolean(this.checked);
}
";

/// Scrolls an element's own scroll box.
pub(crate) const SCROLL_BY: &str = r"
function (x, y) {
  this.scrollBy(x, y);
  return { left: this.scrollLeft, top: this.scrollTop };
}
";

/// Finds an element semantically, and returns it for the caller to describe.
///
/// One function rather than one per [`tinybrowser_bus::LocateBy`] because the
/// hard part is shared: gather candidates, normalise their text the same way the
/// accessibility name is normalised, and pick the nth match. Splitting it would
/// mean seven copies of that normalisation, drifting apart one bug at a time.
pub(crate) const LOCATE: &str = r"
function (by, value, name, exact, index) {
  // The same normalisation the accessibility name gets in `snapshot::render`:
  // pages build their labels out of non-breaking spaces and zero-width joiners,
  // and an agent matching on 'Add to cart' finds nothing otherwise. Written as
  // escapes rather than literals so the characters are visible in this source.
  const norm = (text) => (text || '')
    .replace(/\u00A0/g, ' ')
    .replace(/[\u200B\u200C\u200D\u2060\uFEFF]/g, '')
    .trim()
    .replace(/\s+/g, ' ')
    .toLowerCase();

  const wanted = norm(value);
  const matches = (candidate) => {
    const text = norm(candidate);
    return exact ? text === wanted : text.includes(wanted);
  };

  const visible = (node) => {
    const rect = node.getBoundingClientRect();
    return rect.width > 0 && rect.height > 0;
  };

  const ROLE_SELECTORS = {
    button: 'button, [role=button], input[type=button], input[type=submit], input[type=reset]',
    link: 'a[href], [role=link]',
    textbox: 'input:not([type=button]):not([type=submit]):not([type=reset]):not([type=checkbox]):not([type=radio]), textarea, [role=textbox], [contenteditable=true]',
    checkbox: 'input[type=checkbox], [role=checkbox]',
    radio: 'input[type=radio], [role=radio]',
    combobox: 'select, [role=combobox]',
    heading: 'h1, h2, h3, h4, h5, h6, [role=heading]',
    img: 'img, [role=img]',
  };

  const accessibleName = (node) => node.getAttribute('aria-label')
    || (node.labels && node.labels[0] ? node.labels[0].innerText : '')
    || node.getAttribute('title')
    || node.getAttribute('alt')
    || node.value
    || node.innerText
    || '';

  let candidates = [];
  if (by === 'role') {
    const selector = ROLE_SELECTORS[wanted] || '[role=' + CSS.escape(wanted) + ']';
    candidates = Array.from(document.querySelectorAll(selector));
    if (name) {
      const wantedName = norm(name);
      candidates = candidates.filter((node) => {
        const text = norm(accessibleName(node));
        return exact ? text === wantedName : text.includes(wantedName);
      });
    }
  } else if (by === 'text') {
    candidates = Array.from(document.querySelectorAll('body *'))
      .filter((node) => node.children.length === 0 || node.tagName === 'BUTTON' || node.tagName === 'A')
      .filter((node) => matches(node.innerText));
  } else if (by === 'label') {
    candidates = Array.from(document.querySelectorAll('input, textarea, select, [contenteditable=true]'))
      .filter((node) => matches(node.getAttribute('aria-label')
        || (node.labels && node.labels[0] ? node.labels[0].innerText : '')));
  } else if (by === 'placeholder') {
    candidates = Array.from(document.querySelectorAll('[placeholder]'))
      .filter((node) => matches(node.getAttribute('placeholder')));
  } else if (by === 'test_id') {
    candidates = Array.from(document.querySelectorAll('[data-testid], [data-test-id], [data-test]'))
      .filter((node) => matches(node.getAttribute('data-testid')
        || node.getAttribute('data-test-id')
        || node.getAttribute('data-test')));
  } else if (by === 'alt_text') {
    candidates = Array.from(document.querySelectorAll('[alt]'))
      .filter((node) => matches(node.getAttribute('alt')));
  } else if (by === 'title') {
    candidates = Array.from(document.querySelectorAll('[title]'))
      .filter((node) => matches(node.getAttribute('title')));
  } else {
    throw new Error('unknown locator dimension ' + by);
  }

  // A hidden match is almost never the one meant — a template, a closed menu, an
  // off-screen duplicate — but it is better than nothing when it is all there is.
  const rendered = candidates.filter(visible);
  const pool = rendered.length > 0 ? rendered : candidates;
  return pool[index] || null;
}
";
