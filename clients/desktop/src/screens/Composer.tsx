import { ArrowUp, ChevronDown, ShieldCheck, Sparkles } from "lucide-react";
import { useRef, useState } from "react";
import { useFocusOnOpen } from "../app/focus";
import { models, permissionModes } from "../domain/labels";
import type { AttachTarget } from "../api/attachments";
import { AttachButton, DraftChips, DropOverlay } from "../ui/attachments";
import { useDraftFiles, useFileDrop } from "../ui/draftFiles";
import { cx } from "../ui/cx";
import { AutoTextarea, Spinner } from "../ui/kit";
import { Menu } from "../ui/Menu";
import type { ComposerValue } from "./composerValue";

/**
 * Starts an agent the way Claude's chat input starts a conversation: a
 * rounded box to type in, the model and permission mode in its footer, and
 * a send button. Return sends; Shift-Return adds a line.
 */
export function Composer({
  onSend,
  disabled,
  placeholder = "What should Claude work on?",
  autoFocus,
  focusOnOpen = false,
  allowEmpty = true,
  attachTo = null,
}: {
  onSend: (value: ComposerValue) => Promise<unknown>;
  disabled?: boolean;
  placeholder?: string;
  autoFocus?: boolean;
  /** Put the cursor here whenever the page it's on is opened. */
  focusOnOpen?: boolean;
  /** An empty prompt starts Claude idle, waiting in its own input. */
  allowEmpty?: boolean;
  /** Where attached files go; without it, nothing can be attached. */
  attachTo?: AttachTarget | null;
}) {
  const [prompt, setPrompt] = useState("");
  const [model, setModel] = useState("");
  const [permissionMode, setPermissionMode] = useState("default");
  const [sending, setSending] = useState(false);
  const input = useRef<HTMLTextAreaElement>(null);
  useFocusOnOpen(input, focusOnOpen);
  const files = useDraftFiles(attachTo);
  const drop = useFileDrop((picked) => files.add(picked), Boolean(attachTo) && !disabled);
  const canSend =
    !disabled &&
    !sending &&
    !files.uploading &&
    (allowEmpty || prompt.trim().length > 0 || files.ready.length > 0);

  const send = () => {
    if (!canSend) return;
    setSending(true);
    const attachments = files.ready.map((a) => a.name);
    onSend({ prompt, model, permissionMode, attachments }).then(
      () => {
        setSending(false);
        setPrompt("");
        files.clear();
      },
      () => setSending(false),
    );
  };

  const modelTitle = models.find((m) => m.value === model)?.short ?? model;
  const mode = permissionModes.find((m) => m.value === permissionMode) ?? permissionModes[0];

  return (
    <div
      {...drop.handlers}
      className={cx(
        "relative rounded-[20px] border border-line-strong bg-raised shadow-composer transition-[border-color,box-shadow]",
        "focus-within:border-ink-3/50",
        disabled && "opacity-60",
      )}
    >
      <DropOverlay show={drop.dragging} />
      <DraftChips draft={files} />
      <AutoTextarea
        ref={input}
        aria-label="Prompt"
        value={prompt}
        autoFocus={autoFocus}
        data-autofocus={autoFocus ? "" : undefined}
        disabled={disabled}
        minRows={2}
        maxRows={14}
        placeholder={placeholder}
        onChange={(event) => setPrompt(event.target.value)}
        onKeyDown={(event) => {
          if (event.key === "Enter" && !event.shiftKey && !event.nativeEvent.isComposing) {
            event.preventDefault();
            send();
          }
        }}
        className="block w-full bg-transparent px-4 pt-3.5 pb-1 text-[15px] leading-[1.55] text-ink outline-none placeholder:text-ink-3"
      />
      <div className="flex items-center gap-1 px-2.5 pb-2.5">
        {attachTo ? (
          <AttachButton onFiles={(picked) => files.add(picked)} disabled={disabled} />
        ) : null}
        <Menu
          width={250}
          items={permissionModes.map((m) => ({
            label: m.title,
            detail: m.detail,
            checked: m.value === permissionMode,
            onSelect: () => setPermissionMode(m.value),
          }))}
          trigger={(props) => (
            <button
              type="button"
              {...props}
              disabled={disabled}
              aria-label={`Permission mode: ${mode.title}`}
              className="flex h-8 items-center gap-1.5 rounded-lg px-2 text-[12.5px] text-ink-2 hover:bg-subtle hover:text-ink aria-expanded:bg-subtle"
            >
              <ShieldCheck className="size-3.5" strokeWidth={1.8} />
              {mode.title}
              <ChevronDown className="size-3 text-ink-3" strokeWidth={2} />
            </button>
          )}
        />
        <div className="ml-auto flex items-center gap-1">
          <Menu
            align="end"
            width={200}
            items={models.map((m) => ({
              label: m.title,
              checked: m.value === model,
              onSelect: () => setModel(m.value),
            }))}
            trigger={(props) => (
              <button
                type="button"
                {...props}
                disabled={disabled}
                aria-label={`Model: ${modelTitle}`}
                className="flex h-8 items-center gap-1.5 rounded-lg px-2 text-[12.5px] text-ink-2 hover:bg-subtle hover:text-ink aria-expanded:bg-subtle"
              >
                <Sparkles className="size-3.5" strokeWidth={1.8} />
                {modelTitle}
                <ChevronDown className="size-3 text-ink-3" strokeWidth={2} />
              </button>
            )}
          />
          <button
            type="button"
            aria-label="Start agent"
            title="Start agent (Return)"
            disabled={!canSend}
            onClick={send}
            className={cx(
              "ml-1 flex size-8 items-center justify-center rounded-lg transition-colors",
              "bg-accent text-white hover:bg-accent-strong disabled:bg-accent/35 disabled:text-white/90",
            )}
          >
            {sending ? (
              <Spinner className="text-white" />
            ) : (
              <ArrowUp className="size-4" strokeWidth={2.2} />
            )}
          </button>
        </div>
      </div>
    </div>
  );
}
