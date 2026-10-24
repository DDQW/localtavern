// Colours spoken dialogue: text between quotation marks gets a `md-quote` span, the way SillyTavern
// (and this app's earlier WinUI version) styled it. Actions written as *asterisks* are plain
// emphasis, so they come out italic without any help from here. A character's thoughts are italics
// that are a parenthesis too, `*(I hope he buys it.)*`, and get a `md-thought` class.
//
// Straight, curly and guillemet quotes are all treated as the same toggle, since models and phones
// emit curly ones far more often than a keyboard's ASCII `"`. The quote marks themselves belong to
// the coloured span. A quote left open runs to the end of its paragraph. Apostrophes and single
// quotes are deliberately ignored: they appear in every contraction.

import type { MarkdownIt, StateCore, Token } from "markdown-it";

const QUOTE_CHARS = new Set(['"', "“", "”", "„", "«", "»"]);

export interface Segment {
  text: string;
  quoted: boolean;
}

/**
 * Splits `text` at quotation marks. `inQuote` is whether a quote is already open when the text
 * starts (it carries across the pieces of one paragraph, so `"speech with *emphasis* inside"` stays
 * coloured); the returned `inQuote` is the state afterwards.
 */
export function splitDialogue(text: string, inQuote: boolean): { segments: Segment[]; inQuote: boolean } {
  const segments: Segment[] = [];
  let start = 0;

  const flush = (end: number, quoted: boolean) => {
    if (end > start) segments.push({ text: text.slice(start, end), quoted });
    start = end;
  };

  for (let i = 0; i < text.length; i++) {
    if (!QUOTE_CHARS.has(text[i])) continue;
    // An opening mark starts the coloured span; a closing mark is its last character.
    flush(inQuote ? i + 1 : i, inQuote);
    inQuote = !inQuote;
  }
  flush(text.length, inQuote);

  return { segments, inQuote };
}

/** Whether emphasised text is a thought: all of it inside one pair of parentheses. */
export function isThought(text: string): boolean {
  const t = text.trim();
  if (t.length < 3 || !t.startsWith("(") || !t.endsWith(")")) return false;
  // The first "(" has to close at the very end: "(a) and (b)" is two asides, not one thought.
  let depth = 0;
  for (let i = 0; i < t.length; i++) {
    if (t[i] === "(") depth++;
    else if (t[i] === ")" && --depth === 0 && i < t.length - 1) return false;
  }
  return depth === 0;
}

/** Gives the emphasis that holds a thought (see [`isThought`]) the class `md-thought`. */
function markThoughts(children: Token[]): void {
  for (let i = 0; i < children.length; i++) {
    if (children[i].type !== "em_open") continue;
    let depth = 0;
    let text = "";
    for (let j = i; j < children.length; j++) {
      const token = children[j];
      if (token.type === "em_open") depth++;
      else if (token.type === "em_close" && --depth === 0) break;
      else if (token.type === "text" || token.type === "code_inline") text += token.content;
    }
    if (isThought(text)) children[i].attrJoin("class", "md-thought");
  }
}

/** markdown-it plugin: wraps quoted text in `<span class="md-quote">` and marks thoughts. */
export function dialoguePlugin(md: MarkdownIt): void {
  md.core.ruler.push("dialogue_thoughts", (state: StateCore) => {
    for (const block of state.tokens) {
      if (block.type === "inline" && block.children) markThoughts(block.children);
    }
  });
  md.core.ruler.push("dialogue_quotes", (state: StateCore) => {
    for (const block of state.tokens) {
      if (block.type !== "inline" || !block.children) continue;

      let inQuote = false;
      const children: Token[] = [];
      for (const token of block.children) {
        if (token.type !== "text") {
          children.push(token);
          continue;
        }

        const split = splitDialogue(token.content, inQuote);
        inQuote = split.inQuote;
        for (const segment of split.segments) {
          if (segment.quoted) {
            // One self-contained span per text token, so it can never straddle an emphasis boundary.
            const open = new state.Token("html_inline", "", 0);
            open.content = '<span class="md-quote">';
            children.push(open);
          }
          const text = new state.Token("text", "", 0);
          text.content = segment.text;
          children.push(text);
          if (segment.quoted) {
            const close = new state.Token("html_inline", "", 0);
            close.content = "</span>";
            children.push(close);
          }
        }
      }
      block.children = children;
    }
  });
}
