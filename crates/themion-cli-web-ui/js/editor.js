import { closeBrackets, closeBracketsKeymap, completionKeymap } from "@codemirror/autocomplete";
import { defaultKeymap, history, historyKeymap, indentWithTab } from "@codemirror/commands";
import { css } from "@codemirror/lang-css";
import { html } from "@codemirror/lang-html";
import { javascript } from "@codemirror/lang-javascript";
import { json } from "@codemirror/lang-json";
import { markdown } from "@codemirror/lang-markdown";
import { rust } from "@codemirror/lang-rust";
import { yaml } from "@codemirror/lang-yaml";
import { defaultHighlightStyle, indentOnInput, syntaxHighlighting } from "@codemirror/language";
import { StreamLanguage } from "@codemirror/language";
import { shell } from "@codemirror/legacy-modes/mode/shell";
import { toml } from "@codemirror/legacy-modes/mode/toml";
import { EditorState } from "@codemirror/state";
import {
  drawSelection,
  dropCursor,
  EditorView,
  highlightActiveLine,
  highlightActiveLineGutter,
  highlightSpecialChars,
  keymap,
  lineNumbers,
  rectangularSelection,
} from "@codemirror/view";

const editorTheme = EditorView.theme({
  "&": {
    height: "100%",
    color: "#e5edf8",
    backgroundColor: "rgba(2, 6, 23, 0.72)",
    fontFamily: '"JetBrains Mono Nerd", "JetBrains Mono", ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace',
    fontSize: "0.86rem",
  },
  ".cm-scroller": {
    fontFamily: "inherit",
    lineHeight: "1.45",
  },
  ".cm-content": {
    caretColor: "#7dd3fc",
    padding: "0.65rem 0",
  },
  ".cm-gutters": {
    color: "#64748b",
    backgroundColor: "rgba(15, 23, 42, 0.48)",
    borderRight: "1px solid rgba(148, 163, 184, 0.14)",
  },
  ".cm-activeLine": {
    backgroundColor: "rgba(14, 165, 233, 0.08)",
  },
  ".cm-activeLineGutter": {
    color: "#bae6fd",
    backgroundColor: "rgba(14, 165, 233, 0.12)",
  },
  "&.cm-focused": {
    outline: "1px solid rgba(125, 211, 252, 0.58)",
  },
});

function languageExtension(language) {
  switch (language) {
    case "rust":
      return rust();
    case "toml":
      return StreamLanguage.define(toml);
    case "json":
      return json();
    case "yaml":
      return yaml();
    case "markdown":
      return markdown();
    case "html":
      return html();
    case "css":
      return css();
    case "javascript":
    case "typescript":
      return javascript({ typescript: language === "typescript" });
    case "shell":
      return StreamLanguage.define(shell);
    default:
      return [];
  }
}

function editorExtensions(language, onChange) {
  return [
    lineNumbers(),
    highlightActiveLineGutter(),
    highlightSpecialChars(),
    history(),
    drawSelection(),
    dropCursor(),
    rectangularSelection(),
    highlightActiveLine(),
    indentOnInput(),
    closeBrackets(),
    syntaxHighlighting(defaultHighlightStyle, { fallback: true }),
    languageExtension(language),
    keymap.of([
      indentWithTab,
      ...closeBracketsKeymap,
      ...defaultKeymap,
      ...historyKeymap,
      ...completionKeymap,
    ]),
    editorTheme,
    EditorView.lineWrapping,
    EditorView.updateListener.of((update) => {
      if (update.docChanged && typeof onChange === "function") {
        onChange(update.state.doc.toString());
      }
    }),
  ];
}

window.themionMountCodeEditor = function themionMountCodeEditor(host, content, onChange, language) {
  if (!host) return false;
  if (host.__themionCodeMirror) {
    host.__themionCodeMirror.destroy();
    host.__themionCodeMirror = undefined;
  }
  host.textContent = "";
  const state = EditorState.create({
    doc: content || "",
    extensions: editorExtensions(language || "text", onChange),
  });
  const view = new EditorView({ state, parent: host });
  host.__themionCodeMirror = view;
  return true;
};

window.themionSetCodeEditor = function themionSetCodeEditor(host, content, language) {
  const view = host && host.__themionCodeMirror;
  if (!view) return false;
  const next = content || "";
  const current = view.state.doc.toString();
  if (current === next) return true;
  view.dispatch({
    changes: { from: 0, to: current.length, insert: next },
  });
  return true;
};
