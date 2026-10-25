/** The Splitwire mark: one wire, switched onto two tracks. */
export function Mark({ size = 20, lit = true }: { size?: number; lit?: boolean }) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="none" aria-hidden>
      <path d="M2.5 15.5H10" stroke="var(--color-ink)" strokeWidth="2.4" strokeLinecap="round" />
      <path d="M10 15.5H21.5" stroke="var(--color-ink)" strokeOpacity="0.32" strokeWidth="2.4" strokeLinecap="round" />
      <path
        d="M10 15.5C13.2 15.5 13 7.5 16.4 7.5H21.5"
        stroke={lit ? "var(--color-lamp)" : "var(--color-ink-3)"}
        strokeWidth="2.4"
        strokeLinecap="round"
        strokeLinejoin="round"
        style={{ transition: "stroke 300ms" }}
      />
      <circle cx="10" cy="15.5" r="2.6" fill="var(--color-sunk)" stroke="var(--color-ink)" strokeWidth="1.6" />
    </svg>
  );
}
