// Message rendering: markdown-it → highlight.js → DOMPurify.
//
// Raw HTML in a model's reply is never rendered (`html: false`), and the result is sanitized again as
// defence in depth. Remote images are deliberately NOT loaded: an image URL is a way for injected
// text to leak the conversation, so they become plain links instead. Local files (a `C:\...` path)
// render inline, with video and audio getting real players.

import DOMPurify from "dompurify";
import hljs from "highlight.js/lib/common";
import MarkdownIt from "markdown-it";
import { AUDIO_EXTENSIONS, VIDEO_EXTENSIONS, fileUrl, isLocalPath } from "./media";

function escapeHtml(text: string): string {
  return text.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");
}

function highlight(code: string, language: string): string {
  const lang = language.trim().split(/\s+/)[0].toLowerCase();
  let body: string;
  if (lang && hljs.getLanguage(lang)) {
    body = hljs.highlight(code, { language: lang, ignoreIllegals: true }).value;
  } else {
    body = escapeHtml(code);
  }
  const label = escapeHtml(lang || "text");
  return (
    `<pre class="code"><div class="code-head"><span class="code-lang">${label}</span>` +
    `<button type="button" class="code-copy" data-copy-code>Copy</button></div>` +
    `<code class="hljs">${body}</code></pre>`
  );
}

const md = new MarkdownIt({ html: false, linkify: true, breaks: true, highlight });

md.renderer.rules.image = (tokens, idx) => {
  const token = tokens[idx];
  const src = String(token.attrGet("src") ?? "");
  const alt = token.content || src;

  if (isLocalPath(src)) {
    const url = escapeHtml(fileUrl(src));
    if (VIDEO_EXTENSIONS.test(src)) return `<video class="md-media" controls preload="metadata" src="${url}"></video>`;
    if (AUDIO_EXTENSIONS.test(src)) return `<audio class="md-audio" controls preload="metadata" src="${url}"></audio>`;
    return `<img class="md-media" src="${url}" alt="${escapeHtml(alt)}" loading="lazy">`;
  }

  // Remote (or anything else): show a link, never fetch it automatically.
  if (/^https?:\/\//i.test(src)) {
    return `<a class="md-remote-image" href="${escapeHtml(src)}" rel="noopener noreferrer">🖼 ${escapeHtml(alt)}</a>`;
  }
  return escapeHtml(alt);
};

DOMPurify.addHook("afterSanitizeAttributes", (node) => {
  if (node.tagName === "A") {
    node.setAttribute("rel", "noopener noreferrer");
    node.setAttribute("target", "_blank");
  }
});

const SANITIZE = {
  ADD_TAGS: ["video", "audio", "source"],
  ADD_ATTR: ["controls", "preload", "loading", "data-copy-code", "target"],
};

/** Renders message text to sanitized HTML. */
export function renderMarkdown(text: string): string {
  return DOMPurify.sanitize(md.render(text), SANITIZE) as unknown as string;
}
