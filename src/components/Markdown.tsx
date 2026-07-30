/**
 * Rendering the AI's markdown.
 *
 * `react-markdown` does not use `dangerouslySetInnerHTML` and does not enable
 * raw HTML unless a plugin asks it to — which nothing here does. That matters
 * more than it looks: a summary is generated from a transcript of whatever
 * anyone said on a call, so it is not text this app authored.
 *
 * `muted` is the "enhanced notes" contrast. AI-written prose renders a shade
 * softer than the user's own, which is what makes a merged note readable at a
 * glance — your words, then the model's, without a label on either.
 */

import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { cn } from "../lib/cn";

export function Markdown({
  children,
  muted,
  className,
  /** Checkbox lines are rendered by ActionItems, which can tick them. */
  hideTaskItems,
}: {
  children: string;
  muted?: boolean;
  className?: string;
  hideTaskItems?: boolean;
}) {
  return (
    <div
      className={cn(
        "text-[15px] leading-[1.7]",
        muted ? "text-fg-ai" : "text-fg",
        // Typography, kept here rather than in a plugin so every heading level
        // is deliberate. The summary's `##` headings are its structure, so they
        // are the visual anchor and everything else stays quiet.
        "[&_h1]:mb-2 [&_h1]:mt-6 [&_h1]:text-[19px] [&_h1]:font-semibold [&_h1]:text-fg",
        "[&_h2]:mb-2 [&_h2]:mt-6 [&_h2]:text-[13px] [&_h2]:font-semibold [&_h2]:uppercase [&_h2]:tracking-wide [&_h2]:text-fg-muted",
        "[&_h3]:mb-1 [&_h3]:mt-4 [&_h3]:text-[15px] [&_h3]:font-semibold [&_h3]:text-fg",
        "[&_p]:my-2",
        "[&_ul]:my-2 [&_ul]:list-disc [&_ul]:pl-5",
        "[&_ol]:my-2 [&_ol]:list-decimal [&_ol]:pl-5",
        "[&_li]:my-1 [&_li]:pl-0.5",
        "[&_strong]:font-semibold [&_strong]:text-fg",
        "[&_em]:italic",
        "[&_a]:text-accent [&_a]:underline [&_a]:underline-offset-2",
        "[&_blockquote]:my-3 [&_blockquote]:border-l-2 [&_blockquote]:border-border-strong [&_blockquote]:pl-3 [&_blockquote]:text-fg-muted",
        "[&_code]:rounded [&_code]:bg-sunken [&_code]:px-1 [&_code]:py-0.5 [&_code]:font-mono [&_code]:text-[13px]",
        "[&_pre]:my-3 [&_pre]:overflow-x-auto [&_pre]:rounded-[var(--radius-control)] [&_pre]:bg-sunken [&_pre]:p-3",
        "[&_pre_code]:bg-transparent [&_pre_code]:p-0",
        "[&_hr]:my-5 [&_hr]:border-border",
        // Tables come out of GFM and can be wide; they scroll inside
        // themselves rather than pushing the page sideways.
        "[&_table]:my-3 [&_table]:block [&_table]:w-full [&_table]:overflow-x-auto [&_table]:text-[14px]",
        "[&_th]:border-b [&_th]:border-border [&_th]:px-2 [&_th]:py-1 [&_th]:text-left [&_th]:font-semibold [&_th]:text-fg",
        "[&_td]:border-b [&_td]:border-border [&_td]:px-2 [&_td]:py-1",
        hideTaskItems && "[&_li:has(>input[type=checkbox])]:hidden",
        className,
      )}
    >
      <ReactMarkdown remarkPlugins={[remarkGfm]}>{children}</ReactMarkdown>
    </div>
  );
}
