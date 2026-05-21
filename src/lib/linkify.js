// Split reference text into linkable URL spans and plain-text spans for display.
// URLs are detected loosely (any http/https run up to whitespace or a closing
// paren). Punctuation that abuts a URL in running text - most often the full
// stop after a DOI, e.g. "doi:https://doi.org/10.1/x." - is not part of the URL,
// so it is trimmed from the link and kept as following text.

const URL_RE = /(https?:\/\/[^\s)]+)/g;

// Trailing characters that commonly follow, but are not part of, a URL: sentence
// and citation punctuation, straight and curly quotes, and closing brackets.
const TRAILING = /[.,;:!?'"‘’“”)\]}>]+$/;

/**
 * @param {string} text
 * @returns {Array<{url?: string, t?: string}>} ordered spans; a span carries
 *   `url` (a link) or `t` (plain text).
 */
export function linkifyParts(text) {
  const out = [];
  let last = 0;
  for (const m of text.matchAll(URL_RE)) {
    const start = m.index;
    const matched = m[0];
    const trail = matched.match(TRAILING)?.[0] ?? "";
    const url = matched.slice(0, matched.length - trail.length);
    if (start > last) out.push({ t: text.slice(last, start) });
    if (url) out.push({ url });
    if (trail) out.push({ t: trail });
    last = start + matched.length;
  }
  if (last < text.length) out.push({ t: text.slice(last) });
  return out;
}
