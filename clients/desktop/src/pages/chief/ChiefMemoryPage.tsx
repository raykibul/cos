// What a project's Chief of Staff carries from one session to the next
// (docs/chief-of-staff.md, "Memory notes"): its identity, which only you
// write; its brief, which says what it's working on now; and its memory
// notes, which keep what lasts. The chief writes the brief and notes itself;
// here you read them, set its identity and brief, and keep notes for it. Changes reach it on its next start or new session. Opened from the
// chief's header and ⋯ menu, and the project's menu in the sidebar.

import {
  ChevronRight,
  FolderGit2,
  Lock,
  MessageSquarePlus,
  Pencil,
  Plus,
  Undo2,
  X,
} from "lucide-react";
import { type ReactNode, useEffect, useId, useState } from "react";
import { messageOf } from "../../api/errors";
import {
  IDENTITY_WORDS,
  MAX_BRIEF_BYTES,
  MAX_IDENTITY_BYTES,
  MAX_NOTE_BYTES,
  MAX_NOTE_DESCRIPTION,
  MAX_NOTES,
  type StaffNote,
  noteNameProblem,
  utf8Bytes,
} from "../../api/staff";
import { useApp, useCanChange, useCos } from "../../app/context";
import { project as projectOf } from "../../store/selectors";
import { cx } from "../../ui/cx";
import { Dialog, ErrorText } from "../../ui/Dialog";
import { relativeTime } from "../../ui/format";
import {
  AutoTextarea,
  Button,
  Card,
  EmptyState,
  Field,
  IconButton,
  SectionTitle,
  Spinner,
  inputClass,
} from "../../ui/kit";
import { Crumb, CrumbSep, PageBody, PageHeader, PageTitle } from "../../ui/Page";

/** How long Undo stays after deleting a note. */
const UNDO_MS = 10_000;

export function ChiefMemoryPage({ projectId }: { projectId: number }) {
  const { cos, ui } = useApp();
  const canChange = useCanChange();
  const project = useCos((s) => projectOf(s, projectId));
  const loaded = useCos((s) => s.projectsLoaded);
  const supported = useCos((s) => s.staffSupported);
  const hasAgent = useCos((s) => (s.staffs[projectId]?.agentId ?? null) !== null);

  if (!project || !supported) {
    return (
      <>
        <PageHeader />
        <PageBody>
          <EmptyState
            icon={FolderGit2}
            title={
              !supported
                ? "This daemon has no Chief of Staff"
                : loaded
                  ? "This project is gone"
                  : "Loading…"
            }
          >
            {!supported
              ? "Update cos to give each project a Chief of Staff."
              : loaded
                ? "It was removed. Pick another one in the sidebar."
                : null}
          </EmptyState>
        </PageBody>
      </>
    );
  }

  return (
    <>
      <PageHeader>
        <Crumb onClick={() => ui.navigate({ kind: "chief", id: project.id })}>{project.name}</Crumb>
        <CrumbSep />
        <Crumb onClick={() => ui.navigate({ kind: "chief", id: project.id })}>Chief of Staff</Crumb>
        <CrumbSep />
        <span className="truncate font-medium text-ink">Identity & memory</span>
      </PageHeader>
      <PageBody>
        <PageTitle
          subtitle={
            <span className="block max-w-[620px] leading-relaxed">
              Who {project.name}'s Chief of Staff is, and what it carries from one session to the
              next. It reads changes on its next start, or now if you start a new session.
            </span>
          }
          actions={
            hasAgent ? (
              <Button
                icon={MessageSquarePlus}
                disabled={!canChange}
                title="Starts a fresh conversation with the identity, brief and notes as they are now"
                onClick={() =>
                  void ui.perform("Couldn't start a new session", async () => {
                    await cos.newStaffSession(project.id);
                    ui.navigate({ kind: "chief", id: project.id });
                  })
                }
              >
                New session
              </Button>
            ) : null
          }
        >
          Identity & memory
        </PageTitle>
        <IdentityEditor projectId={project.id} />
        <BriefEditor projectId={project.id} />
        <MemoryNotes projectId={project.id} />
      </PageBody>
    </>
  );
}

/**
 * Its identity: who it is and how it works, written by you. Only you change
 * it (the chief has no tool for it); it heads the chief's prompt.
 */
function IdentityEditor({ projectId }: { projectId: number }) {
  const { cos } = useApp();
  const staff = useCos((s) => s.staffs[projectId]);
  return (
    <ProseEditor
      title="Identity"
      badge="Only you can change this"
      saved={staff?.identity ?? ""}
      ready={staff !== undefined}
      maxBytes={MAX_IDENTITY_BYTES}
      words={IDENTITY_WORDS}
      minRows={8}
      placeholder={
        "Who is your Chief of Staff? For example: You are Ada, the Chief of Staff for opal-app. " +
        "You're calm, direct and careful. Talk to me like a senior engineer: short answers, " +
        "no fluff. Before starting agents, say in one line what each will do. Prefer small PRs. " +
        "Never push to main. I work in Dhaka time and review PRs the same day."
      }
      note={(justSaved) =>
        justSaved
          ? "Saved. It takes this identity from its next start or new session."
          : "Who it is and how it works. It's at the top of the chief's prompt, it can't change it, and it lasts across sessions."
      }
      saveLabel="Save identity"
      save={(text) => cos.setStaffIdentity(projectId, text)}
    />
  );
}

/** The brief: what the chief is working on now, for whom, and what's open. */
function BriefEditor({ projectId }: { projectId: number }) {
  const { cos } = useApp();
  const staff = useCos((s) => s.staffs[projectId]);
  return (
    <ProseEditor
      title="Brief"
      saved={staff?.brief ?? ""}
      ready={staff !== undefined}
      maxBytes={MAX_BRIEF_BYTES}
      minRows={4}
      placeholder="What should it be working on, for whom, and what's open? For example: Ship 0.2: the login fix, left-pad and the release notes. I review PRs the same day."
      note={(justSaved) =>
        justSaved
          ? "Saved. It reads the new brief on its next start or new session."
          : "It keeps this up to date itself with brief_update, so it may rewrite what you put here."
      }
      saveLabel="Save brief"
      save={(text) => cos.setStaffBrief(projectId, text)}
    />
  );
}

/** A text the chief reads from its prompt, with a size limit and Save. */
function ProseEditor({
  title,
  badge,
  saved,
  ready,
  maxBytes,
  words,
  minRows,
  placeholder,
  note,
  saveLabel,
  save: write,
}: {
  title: string;
  badge?: string;
  saved: string;
  ready: boolean;
  maxBytes: number;
  /** Shows a word count against this many words, a guide rather than a limit. */
  words?: number;
  minRows: number;
  placeholder: string;
  note: (justSaved: boolean) => string;
  saveLabel: string;
  save: (text: string) => Promise<unknown>;
}) {
  const canChange = useCanChange();
  // Untouched, the editor follows the saved text, even as the chief rewrites it.
  const [draft, setDraft] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [savedAt, setSavedAt] = useState<number | null>(null);
  const id = useId();

  const text = draft ?? saved;
  const bytes = utf8Bytes(text.trim());
  const tooLong = bytes > maxBytes;
  const dirty = draft !== null && draft.trim() !== saved;
  const wordCount = wordsIn(text);

  const save = () => {
    if (!dirty || tooLong || busy) return;
    setBusy(true);
    setError(null);
    write(text).then(
      () => {
        setBusy(false);
        setDraft(null);
        setSavedAt(Date.now());
      },
      (failure: unknown) => {
        setBusy(false);
        setError(messageOf(failure));
      },
    );
  };

  return (
    <section aria-labelledby={id} className="mb-9">
      <SectionTitle>
        <span id={id}>{title}</span>
        {badge ? (
          <span className="ml-2 inline-flex items-center gap-1 rounded-full bg-subtle px-2 py-0.5 text-[10.5px] font-medium tracking-normal text-ink-2 normal-case">
            <Lock className="size-3" strokeWidth={2} aria-hidden />
            {badge}
          </span>
        ) : null}
      </SectionTitle>
      <Card className="overflow-hidden">
        <AutoTextarea
          aria-labelledby={id}
          value={text}
          minRows={minRows}
          maxRows={24}
          disabled={!canChange || !ready}
          placeholder={placeholder}
          onChange={(event) => {
            setDraft(event.target.value);
            setSavedAt(null);
          }}
          onKeyDown={(event) => {
            if (event.key === "Enter" && (event.metaKey || event.ctrlKey)) {
              event.preventDefault();
              save();
            }
          }}
          className="block w-full bg-transparent px-4 py-3 text-[13.5px] leading-relaxed text-ink outline-none placeholder:text-ink-3 disabled:opacity-60"
        />
        <div className="flex min-h-11 items-center gap-2 border-t border-line px-4 py-2">
          <p className="min-w-0 flex-1 text-[12px] leading-snug text-ink-3">
            {note(savedAt !== null && !dirty)}
          </p>
          <span
            className={cx(
              "shrink-0 font-mono text-[11px] tabular-nums",
              tooLong ? "text-tone-red" : "text-ink-3",
            )}
            title={`It goes in the chief's prompt, so it's kept under ${maxBytes.toLocaleString()} bytes`}
          >
            {words !== undefined
              ? tooLong
                ? `${bytes.toLocaleString()} / ${maxBytes.toLocaleString()} bytes`
                : `${wordCount.toLocaleString()} / ~${words} words`
              : `${bytes.toLocaleString()} / ${maxBytes.toLocaleString()} bytes`}
          </span>
          {dirty ? (
            <Button
              size="sm"
              variant="ghost"
              onClick={() => {
                setDraft(null);
                setError(null);
              }}
            >
              Revert
            </Button>
          ) : null}
          <Button
            size="sm"
            variant="primary"
            busy={busy}
            disabled={!canChange || !dirty || tooLong}
            onClick={save}
            title="Save (⌘↩)"
          >
            {saveLabel}
          </Button>
        </div>
      </Card>
      {error ? (
        <div className="mt-2">
          <ErrorText>{error}</ErrorText>
        </div>
      ) : null}
    </section>
  );
}

/** How many words: runs of anything but white space. */
function wordsIn(text: string): number {
  return text.split(/\s+/).filter(Boolean).length;
}

type Editing = { mode: "add" } | { mode: "edit"; note: StaffNote };

/** The memory notes: what lasts, one subject per note. */
function MemoryNotes({ projectId }: { projectId: number }) {
  const { cos } = useApp();
  const canChange = useCanChange();
  const running = useCos((s) => s.connection === "running");
  const memory = useCos((s) => s.memories[projectId]);
  const [open, setOpen] = useState<Record<string, boolean>>({});
  const [editing, setEditing] = useState<Editing | null>(null);
  const [removed, setRemoved] = useState<StaffNote | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => cos.watchMemory(projectId), [cos, projectId]);

  useEffect(() => {
    if (!removed) return;
    const timer = setTimeout(() => setRemoved(null), UNDO_MS);
    return () => clearTimeout(timer);
  }, [removed]);

  const notes = memory?.notes ?? null;
  const supported = memory?.supported ?? true;
  const full = (notes?.length ?? 0) >= MAX_NOTES;

  const remove = (note: StaffNote) => {
    setBusy(note.name);
    setError(null);
    cos.deleteStaffNote(projectId, note.name).then(
      () => {
        setBusy(null);
        setRemoved(note);
      },
      (failure: unknown) => {
        setBusy(null);
        setError(messageOf(failure));
      },
    );
  };

  const undo = () => {
    if (!removed) return;
    const { name, description, body } = removed;
    setRemoved(null);
    setError(null);
    cos
      .writeStaffNote(projectId, name, { description, body })
      .catch((failure: unknown) => setError(messageOf(failure)));
  };

  let body: ReactNode;
  if (!supported) {
    body = (
      <Card className="px-5 py-4 text-[13px] text-ink-2">
        This cos daemon is older than memory notes. Update cos and restart the daemon.
      </Card>
    );
  } else if (notes === null) {
    body = (
      <Card className="flex items-center gap-2 px-5 py-4 text-[13px] text-ink-3">
        {memory?.error ? (
          <span className="text-tone-red">{memory.error}</span>
        ) : running ? (
          <Spinner />
        ) : (
          "Memory notes are kept by the cos daemon, which isn't answering right now."
        )}
      </Card>
    );
  } else if (notes.length === 0) {
    body = (
      <Card className="flex flex-col items-start gap-3 px-5 py-4">
        <p className="max-w-[560px] text-[13px] leading-relaxed text-ink-2">
          No notes yet. The chief writes them as it learns how you and the project work. Add your
          own for what it should always know: your preferences, decisions and why, where things are.
        </p>
        <Button
          size="sm"
          icon={Plus}
          disabled={!canChange}
          onClick={() => setEditing({ mode: "add" })}
        >
          Add the first note
        </Button>
      </Card>
    );
  } else {
    body = (
      <Card>
        <ul className="divide-y divide-line" aria-label="Memory notes">
          {notes.map((note) => (
            <NoteRow
              key={note.name}
              note={note}
              open={!!open[note.name]}
              onToggle={() => setOpen((s) => ({ ...s, [note.name]: !s[note.name] }))}
            >
              <IconButton
                icon={Pencil}
                label={`Edit ${note.name}`}
                size="sm"
                disabled={!canChange || busy === note.name}
                onClick={() => setEditing({ mode: "edit", note })}
              />
              <IconButton
                icon={X}
                label={`Delete ${note.name}`}
                size="sm"
                disabled={!canChange || busy === note.name}
                onClick={() => remove(note)}
                className="hover:bg-tone-red/10 hover:text-tone-red"
              />
            </NoteRow>
          ))}
        </ul>
      </Card>
    );
  }

  return (
    <section aria-label="Memory notes">
      <SectionTitle
        actions={
          <>
            {notes !== null && notes.length > 0 ? (
              <span className="text-[12px] text-ink-3 tabular-nums">
                {notes.length} of {MAX_NOTES}
              </span>
            ) : null}
            <Button
              size="sm"
              icon={Plus}
              disabled={!canChange || !supported || notes === null || full}
              title={full ? `The chief keeps at most ${MAX_NOTES} notes` : undefined}
              onClick={() => setEditing({ mode: "add" })}
            >
              Add note
            </Button>
          </>
        }
      >
        Memory notes
      </SectionTitle>
      <p className="mb-3 max-w-[620px] text-[12.5px] leading-relaxed text-ink-3">
        Only each note's name and description go in its prompt; it reads a note when it needs it.
      </p>
      {body}
      {removed ? (
        <div
          role="status"
          className="mt-2 flex items-center gap-2 rounded-lg bg-subtle py-1 pr-1 pl-3 text-[12.5px] text-ink-2 animate-fade-in"
        >
          <span className="min-w-0 flex-1 truncate">
            Deleted <code className="font-mono text-[12px] text-ink">{removed.name}</code>.
          </span>
          <Button size="sm" variant="ghost" icon={Undo2} disabled={!canChange} onClick={undo}>
            Undo
          </Button>
        </div>
      ) : null}
      {error ? (
        <div className="mt-2">
          <ErrorText>{error}</ErrorText>
        </div>
      ) : null}
      {editing ? (
        <NoteSheet
          projectId={projectId}
          editing={editing}
          taken={notes?.map((n) => n.name) ?? []}
          onClose={() => setEditing(null)}
        />
      ) : null}
    </section>
  );
}

/** A note: its name and description, and its body when unfolded. */
function NoteRow({
  note,
  open,
  onToggle,
  children,
}: {
  note: StaffNote;
  open: boolean;
  onToggle: () => void;
  children: ReactNode;
}) {
  return (
    <li aria-label={note.name} className="group">
      <div className="flex min-h-12 items-start gap-2 px-3 py-2.5">
        <button
          type="button"
          aria-expanded={open}
          aria-label={`${open ? "Hide" : "Show"} ${note.name}`}
          onClick={onToggle}
          className="flex min-w-0 flex-1 items-start gap-2 rounded-md text-left"
        >
          <ChevronRight
            className={cx(
              "mt-[3px] size-3.5 shrink-0 text-ink-3 transition-transform",
              open && "rotate-90",
            )}
            strokeWidth={2}
            aria-hidden
          />
          <span className="flex min-w-0 flex-1 flex-col gap-0.5">
            <span className="flex min-w-0 items-baseline gap-2">
              <code className="truncate font-mono text-[12.5px] font-medium text-ink">
                {note.name}
              </code>
              {note.updatedMs ? (
                <span className="shrink-0 text-[11.5px] text-ink-3">
                  {relativeTime(note.updatedMs)}
                </span>
              ) : null}
            </span>
            <span className="text-[13px] leading-snug text-ink-2">{note.description}</span>
          </span>
        </button>
        <div className="flex shrink-0 items-center gap-0.5">{children}</div>
      </div>
      {open ? (
        <pre className="mx-3 mb-3 ml-[34px] rounded-lg bg-subtle px-3 py-2.5 font-mono text-[12px] leading-relaxed whitespace-pre-wrap text-ink-2 select-text">
          {note.body}
        </pre>
      ) : null}
    </li>
  );
}

/** Adds a note, or edits one; the name is fixed once it exists. */
function NoteSheet({
  projectId,
  editing,
  taken,
  onClose,
}: {
  projectId: number;
  editing: Editing;
  taken: string[];
  onClose: () => void;
}) {
  const { cos } = useApp();
  const canChange = useCanChange();
  const existing = editing.mode === "edit" ? editing.note : null;
  const [name, setName] = useState(existing?.name ?? "");
  const [description, setDescription] = useState(existing?.description ?? "");
  const [body, setBody] = useState(existing?.body ?? "");
  const [tried, setTried] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const nameId = useId();
  const descriptionId = useId();
  const bodyId = useId();

  const trimmedName = name.trim();
  const nameError = existing ? null : noteNameProblem(trimmedName);
  const replaces = !existing && taken.includes(trimmedName);
  const oneLine = description.replace(/\s*\n\s*/g, " ").trim();
  const descriptionError = !oneLine
    ? "Say in a line what the note holds, so the chief knows when to read it."
    : oneLine.length > MAX_NOTE_DESCRIPTION
      ? `Keep it to ${MAX_NOTE_DESCRIPTION} characters.`
      : null;
  const bytes = utf8Bytes(body.trim());
  const bodyError = !body.trim()
    ? "Write what the chief should know."
    : bytes > MAX_NOTE_BYTES
      ? `Keep it under ${MAX_NOTE_BYTES.toLocaleString()} bytes; split it into more notes.`
      : null;

  const submit = () => {
    setTried(true);
    if (nameError || descriptionError || bodyError || busy) return;
    setBusy(true);
    setError(null);
    cos.writeStaffNote(projectId, trimmedName, { description: oneLine, body: body.trim() }).then(
      () => {
        setBusy(false);
        onClose();
      },
      (failure: unknown) => {
        setBusy(false);
        setError(messageOf(failure));
      },
    );
  };

  const problem = (message: string | null) =>
    tried && message ? (
      <span className="text-tone-red" role="alert">
        {message}
      </span>
    ) : null;

  return (
    <Dialog
      width="lg"
      title={
        existing ? (
          <>
            Edit <span className="font-mono text-[17px]">{existing.name}</span>
          </>
        ) : (
          "Add a memory note"
        )
      }
      description="One subject per note. The chief reads it from its next start or new session."
      onClose={onClose}
      onSubmit={submit}
      footer={
        <>
          <Button variant="ghost" onClick={onClose}>
            Cancel
          </Button>
          <Button
            type="submit"
            variant="primary"
            busy={busy}
            disabled={!canChange || (tried && (!!nameError || !!descriptionError || !!bodyError))}
          >
            {existing ? "Save" : replaces ? "Replace" : "Add note"}
          </Button>
        </>
      }
    >
      <Field
        label="Name"
        htmlFor={nameId}
        hint={
          problem(nameError) ??
          (replaces
            ? `There's already a note called ${trimmedName}; this replaces it.`
            : existing
              ? undefined
              : "Lowercase letters, digits and dashes, such as release-process.")
        }
      >
        <input
          id={nameId}
          data-autofocus={existing ? undefined : true}
          readOnly={!!existing}
          value={name}
          onChange={(event) => setName(event.target.value.toLowerCase())}
          spellCheck={false}
          autoComplete="off"
          placeholder="user-preferences"
          className={cx(inputClass, "font-mono text-[13px]", existing && "bg-subtle text-ink-2")}
        />
      </Field>
      <Field
        label="Description"
        htmlFor={descriptionId}
        hint={
          problem(descriptionError) ?? "One line: what the note holds. This goes in its prompt."
        }
      >
        <input
          id={descriptionId}
          data-autofocus={existing ? true : undefined}
          value={description}
          onChange={(event) => setDescription(event.target.value)}
          placeholder="How I like PRs and replies"
          className={inputClass}
        />
      </Field>
      <Field
        label="Note"
        htmlFor={bodyId}
        hint={
          problem(bodyError) ?? (
            <span className="tabular-nums">
              {bytes.toLocaleString()} / {MAX_NOTE_BYTES.toLocaleString()} bytes
            </span>
          )
        }
      >
        <AutoTextarea
          id={bodyId}
          value={body}
          minRows={6}
          maxRows={18}
          onChange={(event) => setBody(event.target.value)}
          placeholder={
            "- Keep replies short and lead with the result.\n- Ask before pushing a tag."
          }
          className={cx(inputClass, "font-mono text-[12.5px] leading-relaxed")}
        />
      </Field>
      <ErrorText>{error}</ErrorText>
    </Dialog>
  );
}
