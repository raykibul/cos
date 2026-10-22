import { forwardRef } from "react";
import { cx } from "../../ui/cx";
import { AutoTextarea, Button } from "../../ui/kit";

/**
 * Where you talk to the chief, as in Selene: the text above, a hint and
 * Send along the foot. Enter sends; Shift-Enter adds a line.
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
  }
>(function ChiefComposer({ value, onChange, onSend, sending, disabled, note }, ref) {
  const canSend = !disabled && !sending && value.trim().length > 0;
  const send = () => {
    if (canSend) onSend();
  };
  return (
    <form
      onSubmit={(event) => {
        event.preventDefault();
        send();
      }}
      className={cx(
        "rounded-[18px] border border-line-strong bg-raised shadow-composer transition-[border-color,box-shadow]",
        "focus-within:border-ink-3/50",
        disabled && "opacity-60",
      )}
    >
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
      <div className="flex items-center gap-3 px-3 pb-2.5 pl-4">
        <span className="min-w-0 truncate text-[11.5px] text-ink-3">
          {note ?? "Enter to send · Shift+Enter for a new line"}
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
