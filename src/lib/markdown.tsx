import { marked } from "marked";
import DOMPurify from "dompurify";
import { useMemo } from "react";

// Configure marked once: GitHub-flavored linebreaks, no mangling.
marked.setOptions({
  gfm: true,
  breaks: false,
});

interface Props {
  source: string;
}

/** Render trusted-but-untrusted markdown safely. We always sanitize with
 *  DOMPurify because Claude/Codex output can contain arbitrary HTML and
 *  there's no value in letting it execute scripts inside the renderer. */
export function MarkdownView({ source }: Props) {
  const html = useMemo(() => {
    const raw = marked.parse(source ?? "", { async: false }) as string;
    return DOMPurify.sanitize(raw, {
      USE_PROFILES: { html: true },
      // Reject any active content even if marked happened to emit it.
      FORBID_TAGS: ["script", "style", "iframe", "object", "embed"],
      FORBID_ATTR: ["onerror", "onload", "onclick", "onmouseover"],
    });
  }, [source]);

  return (
    <div
      className="md-body"
      // eslint-disable-next-line react/no-danger
      dangerouslySetInnerHTML={{ __html: html }}
    />
  );
}

/** Color a unified diff inline. We avoid CodeMirror and friends — split on
 *  lines, prefix-check, wrap each line in a span with a class. The hunk
 *  headers (@@ … @@) get their own tone too. */
export function DiffView({ source }: { source: string }) {
  const lines = source.split(/\r?\n/);
  return (
    <pre className="diff-body">
      {lines.map((line, i) => {
        let cls = "diff-ctx";
        if (line.startsWith("+++") || line.startsWith("---")) cls = "diff-meta";
        else if (line.startsWith("@@")) cls = "diff-hunk";
        else if (line.startsWith("+")) cls = "diff-add";
        else if (line.startsWith("-")) cls = "diff-del";
        else if (line.startsWith("diff ") || line.startsWith("index "))
          cls = "diff-meta";
        return (
          <span key={i} className={cls}>
            {line || " "}
            {"\n"}
          </span>
        );
      })}
    </pre>
  );
}
