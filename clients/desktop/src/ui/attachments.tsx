// Attaching files to a message ("Attachments" in docs/agents-api.md): the
// paperclip, the chips on a composer (its files are in draftFiles.ts), the
// drop overlay, and the files a sent message names, under its bubble.

import { FileText, Plus, X } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { type AttachedFile, isImage, sizeLabel } from "../api/attachments";
import { useApp } from "../app/context";
import { cx } from "./cx";
import type { DraftFiles } from "./draftFiles";
import { Spinner } from "./kit";

/** The paperclip: opens the file picker. */
export function AttachButton({
  onFiles,
  disabled,
  className,
}: {
  onFiles: (files: File[]) => void;
  disabled?: boolean;
  className?: string;
}) {
  const input = useRef<HTMLInputElement>(null);
  return (
    <>
      <button
        type="button"
        aria-label="Attach files"
        title="Attach files (or paste or drop them here)"
        disabled={disabled}
        onClick={() => input.current?.click()}
        className={cx(
          "flex size-8 shrink-0 items-center justify-center rounded-lg text-ink-2 hover:bg-subtle hover:text-ink disabled:opacity-50",
          className,
        )}
      >
        <Plus className="size-[18px]" strokeWidth={1.8} />
      </button>
      <input
        ref={input}
        type="file"
        multiple
        hidden
        data-testid="attach-input"
        onChange={(event) => {
          const files = [...(event.target.files ?? [])];
          event.target.value = "";
          if (files.length) onFiles(files);
        }}
      />
    </>
  );
}

/** "Drop files to attach", over a composer while files are dragged onto it. */
export function DropOverlay({ show }: { show: boolean }) {
  if (!show) return null;
  return (
    <div className="pointer-events-none absolute inset-0 z-10 flex items-center justify-center rounded-[inherit] border-2 border-dashed border-accent/60 bg-accent/5 text-[13px] font-medium text-accent">
      Drop files to attach
    </div>
  );
}

/** The files on a composer, as chips above its text. */
export function DraftChips({ draft }: { draft: DraftFiles }) {
  if (draft.files.length === 0) return null;
  return (
    <ul aria-label="Attached files" className="flex flex-wrap gap-2 px-3 pt-3">
      {draft.files.map((file) => (
        <li key={file.id}>
          <FileChip
            name={file.name}
            detail={
              file.state === "failed"
                ? (file.error ?? "Couldn't upload")
                : file.state === "uploading"
                  ? "Uploading…"
                  : sizeLabel(file.size)
            }
            contentType={file.contentType}
            imageUrl={file.previewUrl}
            busy={file.state === "uploading"}
            failed={file.state === "failed"}
            onRemove={() => draft.remove(file.id)}
          />
        </li>
      ))}
    </ul>
  );
}

/** One file: a thumbnail for an image, else an icon, with its name and size. */
export function FileChip({
  name,
  detail,
  contentType,
  imageUrl,
  busy,
  failed,
  onRemove,
}: {
  name: string;
  detail: string;
  contentType: string;
  imageUrl: string | null;
  busy?: boolean;
  failed?: boolean;
  onRemove?: () => void;
}) {
  return (
    <div
      title={name}
      className={cx(
        "group relative flex h-12 max-w-[240px] items-center gap-2.5 rounded-xl border bg-surface py-1.5 pr-3 pl-1.5",
        failed ? "border-tone-red/50" : "border-line",
      )}
    >
      <div className="flex size-9 shrink-0 items-center justify-center overflow-hidden rounded-lg bg-subtle text-ink-3">
        {imageUrl ? (
          <img src={imageUrl} alt="" className="size-full object-cover" />
        ) : (
          <FileText className="size-4" strokeWidth={1.8} />
        )}
      </div>
      <div className="min-w-0">
        <div className="truncate text-[12.5px] font-medium text-ink">{name}</div>
        <div
          className={cx(
            "flex items-center gap-1 truncate text-[11px]",
            failed ? "text-tone-red" : "text-ink-3",
          )}
        >
          {busy ? <Spinner className="size-3" /> : null}
          {detail || kindLabel(contentType)}
        </div>
      </div>
      {onRemove ? (
        <button
          type="button"
          aria-label={`Remove ${name}`}
          onClick={onRemove}
          className="absolute -top-1.5 -right-1.5 flex size-5 items-center justify-center rounded-full border border-line bg-raised text-ink-2 shadow-sm hover:text-ink"
        >
          <X className="size-3" strokeWidth={2.2} />
        </button>
      ) : null}
    </div>
  );
}

function kindLabel(contentType: string): string {
  if (contentType === "application/pdf") return "PDF";
  if (contentType.startsWith("image/")) return "Image";
  if (contentType.startsWith("text/")) return "Text";
  return "File";
}

/**
 * The files a sent message names, under its bubble. Images are fetched from
 * the daemon (through `agentId`) to show; `previews` are local ones, for a
 * message the transcript hasn't caught up with.
 */
export function SentFiles({
  files,
  agentId,
  previews,
}: {
  files: AttachedFile[];
  agentId: number | null;
  previews?: Record<string, string | null>;
}) {
  if (files.length === 0) return null;
  return (
    <ul aria-label="Files sent" className="mb-1.5 flex max-w-[82%] flex-wrap justify-end gap-2">
      {files.map((file) => (
        <li key={file.path || file.name}>
          <SentFile file={file} agentId={agentId} preview={previews?.[file.name] ?? null} />
        </li>
      ))}
    </ul>
  );
}

function SentFile({
  file,
  agentId,
  preview,
}: {
  file: AttachedFile;
  agentId: number | null;
  preview: string | null;
}) {
  const { cos } = useApp();
  const [url, setUrl] = useState<string | null>(preview);
  const image = isImage(file.contentType);
  useEffect(() => {
    if (!image || preview || agentId === null) return;
    let live = true;
    cos.attachmentUrl(agentId, file.name).then(
      (data) => live && setUrl(data),
      () => undefined,
    );
    return () => {
      live = false;
    };
  }, [cos, agentId, file.name, image, preview]);
  if (image && url) {
    return (
      <img
        src={url}
        alt={file.name}
        title={`${file.name} · ${file.sizeLabel}`}
        className="max-h-44 max-w-[260px] rounded-xl border border-line object-cover"
      />
    );
  }
  return (
    <FileChip
      name={file.name}
      detail={file.sizeLabel}
      contentType={file.contentType}
      imageUrl={null}
    />
  );
}
