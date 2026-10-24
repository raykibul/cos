import { ChevronDown, ShieldCheck, Sparkles } from "lucide-react";
import { useRef, useState } from "react";
import { useFocusOnOpen } from "../app/focus";
import { models, permissionModes } from "../domain/labels";
import type { AttachTarget } from "../api/attachments";
import { ChatComposer } from "../ui/ChatComposer";
import { useDraftFiles } from "../ui/draftFiles";
import { Menu } from "../ui/Menu";
import type { ComposerValue } from "./composerValue";

const pickerClass =
  "flex h-[34px] items-center gap-1.5 rounded-full px-2.5 text-[12.5px] text-ink-2 hover:bg-subtle hover:text-ink aria-expanded:bg-subtle disabled:opacity-50";

/**
 * Starts an agent the way Claude Desktop starts a conversation: a one-line
 * box with + to attach, the permission mode and model beside the text, and
 * a round send button. Return sends; Shift-Return adds a line.
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

  const send = () => {
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
    <ChatComposer
      ref={input}
      label="Prompt"
      placeholder={placeholder}
      value={prompt}
      onChange={setPrompt}
      onSend={send}
      sending={sending}
      disabled={disabled}
      allowEmpty={allowEmpty}
      autoFocus={autoFocus}
      files={attachTo ? files : undefined}
      sendLabel="Start agent"
      trailing={
        <>
          <Menu
            align="end"
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
                title={`Permission mode: ${mode.title}`}
                className={pickerClass}
              >
                <ShieldCheck className="size-3.5" strokeWidth={1.8} />
                <span className="hidden @min-[38rem]:inline">{mode.title}</span>
                <ChevronDown className="size-3 text-ink-3" strokeWidth={2} />
              </button>
            )}
          />
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
                className={pickerClass}
              >
                <Sparkles className="size-3.5" strokeWidth={1.8} />
                <span className="hidden @min-[30rem]:inline">{modelTitle}</span>
                <ChevronDown className="size-3 text-ink-3" strokeWidth={2} />
              </button>
            )}
          />
        </>
      }
    />
  );
}
