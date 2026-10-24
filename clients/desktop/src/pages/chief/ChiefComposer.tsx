import { forwardRef } from "react";
import { AttachButton, DraftChips, DropOverlay } from "../../ui/attachments";
import { type DraftFiles, useFileDrop } from "../../ui/draftFiles";
import { cx } from "../../ui/cx";
import { AutoTextarea, Button } from "../../ui/kit";

/**
 * Where you talk to the chief, as in Selene: the text above, a hint and
 * Send along the foot. Enter sends; Shift-Enter adds a line. Files come in
 * through the paperclip, a paste or a drop, and show as chips on top.
 */
export const ChiefComposer = forwardRef<
  HTMLTextAreaElement,
  {
    value: string;
    onChange: (value: string) => void;
    onSend: () => void;
    sending: boolean;
    disabled?: boolean;
    note?: string | null;
    /** Files on the message; without it, nothing can be attached. */
    files?: DraftFiles;
  }
>(function ChiefComposer({ value, onChange, onSend, sending, disabled, note, files }, ref) {
  const uploading = files?.uploading ?? false;
  const hasFiles = (files?.ready.length ?? 0) > 0;
  const canSend = !disabled && !sending && !uploading && (value.trim().length > 0 || hasFiles);
  const drop = useFileDrop((picked) => files?.add(picked), Boolean(files) && !disabled);
  const send = () => {
    if (canSend) onSend();
  };
  return (
    <form
      onSubmit={(event) => {
        event.preventDefault();
        send();
      }}
      {...drop.handlers}
      className={cx(
        "relative rounded-[18px] border border-line-strong bg-raised shadow-composer transition-[border-color,box-shadow]",
        "focus-within:border-ink-3/50",
        disabled && "opacity-60",
      )}
    >
      <DropOverlay show={drop.dragging} />
      {files ? <DraftChips draft={files} /> : null}
      <AutoTextarea
        ref={ref}
        aria-label="Message the Chief of Staff"
        value={value}
        disabled={disabled}
        minRows={2}
        maxRows={12}
        placeholder="What should we work on?"
        onChange={(event) => onChange(event.target.value)}
        onKeyDown={(event) => {
          if (event.key === "Enter" && !event.shiftKey && !event.nativeEvent.isComposing) {
            event.preventDefault();
            send();
          }
        }}
        className="block w-full bg-transparent px-4 pt-3.5 pb-1 text-[15px] leading-[1.55] text-ink outline-none placeholder:text-ink-3"
      />
      <div className={cx("flex items-center gap-3 px-3 pb-2.5", files ? "pl-2" : "pl-4")}>
        {files ? (
          <AttachButton onFiles={(picked) => files.add(picked)} disabled={disabled} />
        ) : null}
        <span className="min-w-0 truncate text-[11.5px] text-ink-3">
          {uploading ? "Uploading files…" : (note ?? "Enter to send · Shift+Enter for a new line")}
        </span>
        <Button
          type="submit"
          variant="primary"
          size="sm"
          busy={sending}
          disabled={!canSend}
          className="ml-auto px-3.5"
        >
          {sending ? "Sending…" : "Send"}
        </Button>
      </div>
    </form>
  );
});
