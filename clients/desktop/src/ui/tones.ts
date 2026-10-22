import type { Tone } from "../domain/approvals";

/** Text, soft background and dot colour for each tone. */
export const toneClasses: Record<Tone, { text: string; soft: string; dot: string; edge: string }> =
  {
    neutral: { text: "text-ink-2", soft: "bg-subtle", dot: "bg-ink-3", edge: "bg-line-strong" },
    accent: { text: "text-accent-ink", soft: "bg-accent/12", dot: "bg-accent", edge: "bg-accent" },
    blue: {
      text: "text-tone-blue",
      soft: "bg-tone-blue/12",
      dot: "bg-tone-blue",
      edge: "bg-tone-blue",
    },
    green: {
      text: "text-tone-green",
      soft: "bg-tone-green/12",
      dot: "bg-tone-green",
      edge: "bg-tone-green",
    },
    red: { text: "text-tone-red", soft: "bg-tone-red/12", dot: "bg-tone-red", edge: "bg-tone-red" },
    purple: {
      text: "text-tone-purple",
      soft: "bg-tone-purple/12",
      dot: "bg-tone-purple",
      edge: "bg-tone-purple",
    },
    teal: {
      text: "text-tone-teal",
      soft: "bg-tone-teal/12",
      dot: "bg-tone-teal",
      edge: "bg-tone-teal",
    },
    amber: {
      text: "text-tone-amber",
      soft: "bg-tone-amber/14",
      dot: "bg-tone-amber",
      edge: "bg-tone-amber",
    },
  };
