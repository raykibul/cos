import { ArrowUp, CornerUpLeft, X } from "lucide-react";
import { type ReactNode, forwardRef } from "react";
import { AttachButton, DraftChips, DropOverlay } from "./attachments";
import { type DraftFiles, useFileDrop } from "./draftFiles";
import { cx } from "./cx";
import { AutoTextarea, Spinner } from "./kit";

/**
 * A one-line message box, as in Claude Desktop: + to attach on the left,
 * the text, whatever `trailing` holds (a model menu, say) and a round send
 * button on the right. It grows when the text runs past one line. Enter
 * sends; Shift-Enter adds a line. Files come in through +, a paste or a
 * drop, and show as chips on top. `note` shows under the box. With
 * `reply`, the message it answers shows on top too, with × (or Escape) to
 * drop it.
 */
export const ChatComposer = forwardRef<
  HTMLTextAreaElement,
  {
    value: string;
    onChange: (value: string) => void;
    onSend: () => void;
    sending: boolean;
    label: string;
    placeholder: string;
    disabled?: boolean;
    /** Send with nothing typed and no files (starting an idle agent). */
    allowEmpty?: boolean;
    note?: string | null;
    /** Files on the message; without it, nothing can be attached. */
    files?: DraftFiles;
    trailing?: ReactNode;
    sendLabel?: string;
    autoFocus?: boolean;
    /** The message this one answers: whose it is ("Replying to Claude") and the quote. */
    reply?: { label: string; text: string } | null;
    onCancelReply?: () => void;
  }
>(function ChatComposer(
  {
    value,
    onChange,
    onSend,
    sending,
    label,
    placeholder,
    disabled,
    allowEmpty = false,
    note,
    files,
    trailing,
    sendLabel = "Send",
    autoFocus,
    reply,
    onCancelReply,
  },
  ref,
) {
  const uploading = files?.uploading ?? false;
  const hasFiles = (files?.ready.length ?? 0) > 0;
  const canSend =
    !disabled && !sending && !uploading && (allowEmpty || value.trim().length > 0 || hasFiles);
  const drop = useFileDrop((picked) => files?.add(picked), Boolean(files) && !disabled);
  const send = () => {
    if (canSend) onSend();
  };
  const shownNote = uploading ? "Uploading files…" : note;
  return (
    <div className="@container">
      <form
        onSubmit={(event) => {
          event.preventDefault();
          send();
        }}
        {...drop.handlers}
        className={cx(
          "relative rounded-[24px] border border-line-strong bg-raised shadow-composer transition-[border-color,box-shadow]",
          "focus-within:border-ink-3/50",
          disabled && "opacity-60",
        )}
      >
        <DropOverlay show={drop.dragging} />
        {reply ? <ReplyStrip reply={reply} onCancel={onCancelReply} /> : null}
        {files ? <DraftChips draft={files} /> : null}
        <div className="flex items-end gap-1 p-[7px]">
          {files ? (
            <AttachButton
              onFiles={(picked) => files.add(picked)}
              disabled={disabled}
              className="size-[34px] rounded-full"
            />
          ) : (
            <span className="w-2 shrink-0" />
          )}
          <AutoTextarea
            ref={ref}
            aria-label={label}
            value={value}
            disabled={disabled}
            autoFocus={autoFocus}
            data-autofocus={autoFocus ? "" : undefined}
            minRows={1}
            maxRows={12}
            placeholder={placeholder}
            onChange={(event) => onChange(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === "Enter" && !event.shiftKey && !event.nativeEvent.isComposing) {
                event.preventDefault();
                send();
              } else if (event.key === "Escape" && reply && onCancelReply) {
                event.preventDefault();
                event.stopPropagation();
                onCancelReply();
              }
            }}
            className="block min-w-0 flex-1 bg-transparent px-1.5 py-[6px] text-[15px] leading-[22px] text-ink outline-none placeholder:text-ink-3"
          />
          {trailing ? <div className="flex shrink-0 items-center gap-0.5">{trailing}</div> : null}
          <button
            type="submit"
            aria-label={sendLabel}
            title={`${sendLabel} (Return)`}
            disabled={!canSend}
            className={cx(
              "flex size-[34px] shrink-0 items-center justify-center rounded-full transition-colors",
              "bg-accent text-white hover:bg-accent-strong disabled:bg-subtle-2 disabled:text-ink-3",
            )}
          >
            {sending ? (
              <Spinner className="text-white" />
            ) : (
              <ArrowUp className="size-4" strokeWidth={2.2} />
            )}
          </button>
        </div>
      </form>
      {shownNote ? (
        <p className="mt-1.5 truncate px-4 text-[11.5px] text-ink-3" role="status">
          {shownNote}
        </p>
      ) : null}
    </div>
  );
});

/** The message you're answering, on top of the box. */
function ReplyStrip({
  reply,
  onCancel,
}: {
  reply: { label: string; text: string };
  onCancel?: () => void;
}) {
  return (
    <div
      role="group"
      aria-label={reply.label}
      className="mx-[7px] mt-[7px] flex items-start gap-2 rounded-[17px] bg-subtle/80 py-1.5 pr-1.5 pl-3"
    >
      <CornerUpLeft className="mt-[3px] size-3.5 shrink-0 text-ink-3" strokeWidth={2} aria-hidden />
      <div className="min-w-0 flex-1 text-[12px] leading-[18px]">
        <div className="font-medium text-ink-2">{reply.label}</div>
        <div className="line-clamp-2 break-words text-ink-3">{reply.text}</div>
      </div>
      {onCancel ? (
        <button
          type="button"
          aria-label="Cancel reply"
          title="Cancel reply (Escape)"
          onClick={onCancel}
          className="flex size-6 shrink-0 items-center justify-center rounded-full text-ink-3 hover:bg-subtle-2 hover:text-ink"
        >
          <X className="size-3.5" strokeWidth={2} />
        </button>
      ) : null}
    </div>
  );
}
