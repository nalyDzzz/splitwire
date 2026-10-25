import clsx from "clsx";
import { Folder, Search } from "lucide-react";
import { useEffect } from "react";

import { useStore } from "@/store";

/** An exe's icon, or a quiet monogram tile when it has none. */
export function AppIcon({
  path,
  name,
  size = 28,
  kind,
  className,
}: {
  path: string | null | undefined;
  name: string;
  size?: number;
  kind?: "folder" | "keyword";
  className?: string;
}) {
  const icon = useStore((s) => (path ? s.icons[path] : undefined));
  const requestIcons = useStore((s) => s.requestIcons);

  useEffect(() => {
    if (path) requestIcons([path]);
  }, [path, requestIcons]);

  const box = { width: size, height: size };

  if (icon) {
    return <img src={icon} alt="" style={box} className={clsx("shrink-0 object-contain", className)} draggable={false} />;
  }

  if (kind) {
    const Glyph = kind === "folder" ? Folder : Search;
    return (
      <span
        style={box}
        className={clsx("inline-flex shrink-0 items-center justify-center rounded-[28%] border border-rule-2 bg-plate-3 text-ink-2", className)}
      >
        <Glyph size={Math.round(size * 0.5)} strokeWidth={1.75} />
      </span>
    );
  }

  const letters = monogram(name);
  return (
    <span
      style={{ ...box, fontSize: Math.round(size * (letters.length > 1 ? 0.36 : 0.44)) }}
      className={clsx(
        "inline-flex shrink-0 items-center justify-center rounded-[28%] border border-rule-2 bg-plate-3 font-condensed font-bold tracking-tight text-ink-2 uppercase",
        className,
      )}
    >
      {letters}
    </span>
  );
}

function monogram(name: string): string {
  const stem = name.replace(/\.exe$/i, "").replace(/[^a-z0-9 ._-]/gi, "");
  const words = stem.split(/[\s._-]+/).filter(Boolean);
  if (words.length >= 2) return (words[0]![0]! + words[1]![0]!).toUpperCase();
  return (stem.slice(0, 2) || "?").replace(/^./, (c) => c.toUpperCase());
}
