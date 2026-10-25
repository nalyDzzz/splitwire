// Small, shared UI primitives.

import clsx from "clsx";
import { AnimatePresence, motion } from "motion/react";
import {
  forwardRef,
  useEffect,
  useId,
  useRef,
  useState,
  type ButtonHTMLAttributes,
  type InputHTMLAttributes,
  type ReactNode,
} from "react";
import { X } from "lucide-react";

type ButtonVariant = "primary" | "secondary" | "ghost" | "danger" | "outline";
type ButtonSize = "sm" | "md" | "lg";

interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: ButtonVariant;
  size?: ButtonSize;
  icon?: ReactNode;
}

export const Button = forwardRef<HTMLButtonElement, ButtonProps>(function Button(
  { variant = "secondary", size = "md", icon, className, children, type = "button", ...rest },
  ref,
) {
  return (
    <button
      ref={ref}
      type={type}
      className={clsx(
        "inline-flex shrink-0 items-center justify-center gap-1.5 rounded-lg font-medium whitespace-nowrap transition-[background-color,border-color,color,opacity,transform] duration-150 select-none active:translate-y-px disabled:pointer-events-none disabled:opacity-40",
        size === "sm" && "h-7 px-2.5 text-xs",
        size === "md" && "h-8 px-3 text-sm",
        size === "lg" && "h-10 px-5 text-base",
        variant === "primary" &&
          "bg-lamp font-semibold text-sunk shadow-[0_0_0_1px_rgba(255,174,52,.4),0_6px_24px_-8px_rgba(255,174,52,.55)] hover:bg-lamp-2",
        variant === "secondary" && "border border-rule-2 bg-plate-2 text-ink hover:border-ink-4 hover:bg-plate-3",
        variant === "outline" && "border border-rule-2 text-ink hover:border-ink-3 hover:bg-plate-2",
        variant === "ghost" && "text-ink-2 hover:bg-plate-2 hover:text-ink",
        variant === "danger" && "border border-signal/40 bg-signal-deep text-signal hover:border-signal/70",
        className,
      )}
      {...rest}
    >
      {icon}
      {children}
    </button>
  );
});

export function IconButton({
  label,
  className,
  children,
  ...rest
}: ButtonHTMLAttributes<HTMLButtonElement> & { label: string }) {
  return (
    <button
      type="button"
      aria-label={label}
      title={label}
      className={clsx(
        "inline-flex size-7 items-center justify-center rounded-md text-ink-3 transition-colors hover:bg-plate-3 hover:text-ink disabled:pointer-events-none disabled:opacity-40",
        className,
      )}
      {...rest}
    >
      {children}
    </button>
  );
}

export function Switch({
  checked,
  onChange,
  disabled,
  label,
}: {
  checked: boolean;
  onChange: (value: boolean) => void;
  disabled?: boolean;
  label: string;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={label}
      disabled={disabled}
      onClick={() => onChange(!checked)}
      className={clsx(
        "relative inline-flex h-[22px] w-[38px] shrink-0 items-center rounded-full border transition-colors duration-200 disabled:opacity-40",
        checked ? "border-lamp/60 bg-lamp-deep" : "border-rule-2 bg-sunk",
      )}
    >
      <span
        className={clsx(
          "absolute size-[14px] rounded-full transition-all duration-200 ease-(--ease-snap)",
          checked ? "left-[19px] bg-lamp shadow-[0_0_10px_rgba(255,174,52,.7)]" : "left-[3px] bg-ink-3",
        )}
      />
    </button>
  );
}

export function Segmented<T extends string>({
  value,
  options,
  onChange,
  size = "md",
}: {
  value: T;
  options: { value: T; label: ReactNode }[];
  onChange: (value: T) => void;
  size?: "sm" | "md";
}) {
  const id = useId();
  return (
    <div
      role="radiogroup"
      className="inline-flex shrink-0 rounded-lg border border-rule bg-sunk p-0.5"
    >
      {options.map((option) => {
        const active = option.value === value;
        return (
          <button
            key={option.value}
            type="button"
            role="radio"
            aria-checked={active}
            onClick={() => onChange(option.value)}
            className={clsx(
              "relative rounded-md font-medium whitespace-nowrap transition-colors",
              size === "md" ? "h-7 px-3 text-sm" : "h-6 px-2.5 text-xs",
              active ? "text-ink" : "text-ink-3 hover:text-ink-2",
            )}
          >
            {active && (
              <motion.span
                layoutId={`seg-${id}`}
                className="absolute inset-0 rounded-md border border-rule-2 bg-plate-3"
                transition={{ type: "spring", duration: 0.35, bounce: 0.15 }}
              />
            )}
            <span className="relative">{option.label}</span>
          </button>
        );
      })}
    </div>
  );
}

export const TextInput = forwardRef<
  HTMLInputElement,
  InputHTMLAttributes<HTMLInputElement> & { invalid?: boolean; mono?: boolean }
>(function TextInput({ invalid, mono, className, ...rest }, ref) {
  return (
    <input
      ref={ref}
      spellCheck={false}
      autoComplete="off"
      className={clsx(
        "h-8 w-full rounded-lg border bg-sunk px-2.5 text-sm text-ink transition-colors outline-none",
        "focus:border-lamp/60 focus:shadow-[0_0_0_3px_rgba(255,174,52,.12)]",
        invalid ? "border-signal/60" : "border-rule hover:border-rule-2",
        mono && "font-mono text-xs",
        className,
      )}
      {...rest}
    />
  );
});

export function Field({
  label,
  hint,
  error,
  children,
  htmlFor,
  aside,
}: {
  label: string;
  hint?: ReactNode;
  error?: string | null;
  children: ReactNode;
  htmlFor?: string;
  aside?: ReactNode;
}) {
  return (
    <div className="flex flex-col gap-1.5">
      <div className="flex items-baseline justify-between gap-2">
        <label htmlFor={htmlFor} className="text-xs font-medium text-ink-2">
          {label}
        </label>
        {aside}
      </div>
      {children}
      {error ? (
        <p className="text-xs text-signal">{error}</p>
      ) : hint ? (
        <p className="text-xs text-ink-3">{hint}</p>
      ) : null}
    </div>
  );
}

export function Modal({
  open,
  onClose,
  title,
  description,
  children,
  footer,
  width = 520,
}: {
  open: boolean;
  onClose: () => void;
  title: string;
  description?: ReactNode;
  children: ReactNode;
  footer?: ReactNode;
  width?: number;
}) {
  const panel = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, onClose]);

  return (
    <AnimatePresence>
      {open && (
        <motion.div
          className="fixed inset-0 z-50 flex items-center justify-center p-6 pt-14"
          initial={{ opacity: 0 }}
          animate={{ opacity: 1 }}
          exit={{ opacity: 0 }}
          transition={{ duration: 0.15 }}
        >
          <div className="absolute inset-0 bg-sunk/75 backdrop-blur-[3px]" onClick={onClose} />
          <motion.div
            ref={panel}
            role="dialog"
            aria-modal="true"
            aria-label={title}
            className="relative flex max-h-full w-full flex-col overflow-hidden rounded-2xl border border-rule-2 bg-plate shadow-[0_30px_80px_-20px_rgba(0,0,0,.8)]"
            style={{ maxWidth: width }}
            initial={{ opacity: 0, y: 10, scale: 0.98 }}
            animate={{ opacity: 1, y: 0, scale: 1 }}
            exit={{ opacity: 0, y: 6, scale: 0.98 }}
            transition={{ type: "spring", duration: 0.32, bounce: 0.12 }}
          >
            <div className="flex items-start justify-between gap-4 px-5 pt-4 pb-3">
              <div>
                <h2 className="font-semiexpanded text-[15px] font-semibold tracking-tight">{title}</h2>
                {description && <p className="mt-1 text-sm text-ink-2">{description}</p>}
              </div>
              <IconButton label="Close" onClick={onClose} className="-mr-1.5">
                <X size={16} />
              </IconButton>
            </div>
            <div className="min-h-0 flex-1 overflow-y-auto px-5 pb-5">{children}</div>
            {footer && (
              <div className="flex items-center justify-end gap-2 border-t border-rule bg-plate-2/50 px-5 py-3">
                {footer}
              </div>
            )}
          </motion.div>
        </motion.div>
      )}
    </AnimatePresence>
  );
}

/** Status lamp: a small LED in the route colors. */
export function Lamp({
  tone,
  pulse,
  className,
}: {
  tone: "lamp" | "track" | "signal" | "off" | "go";
  pulse?: boolean;
  className?: string;
}) {
  return (
    <span
      aria-hidden
      className={clsx(
        "inline-block size-2 shrink-0 rounded-full",
        tone === "lamp" && "bg-lamp shadow-[0_0_8px_rgba(255,174,52,.85)]",
        tone === "track" && "bg-track/80",
        tone === "signal" && "bg-signal shadow-[0_0_8px_rgba(255,91,77,.8)]",
        tone === "go" && "bg-go shadow-[0_0_8px_rgba(111,211,165,.6)]",
        tone === "off" && "border border-ink-4 bg-transparent",
        pulse && "anim-lamp-pulse",
        className,
      )}
    />
  );
}

export function Spinner({ size = 14 }: { size?: number }) {
  return (
    <span
      aria-hidden
      className="anim-spin inline-block rounded-full border-2 border-current border-t-transparent"
      style={{ width: size, height: size }}
    />
  );
}

/** Copies text and briefly confirms. */
export function useCopy(): [boolean, (text: string) => void] {
  const [copied, setCopied] = useState(false);
  useEffect(() => {
    if (!copied) return;
    const t = setTimeout(() => setCopied(false), 1400);
    return () => clearTimeout(t);
  }, [copied]);
  return [
    copied,
    (text) => {
      void navigator.clipboard?.writeText(text).then(() => setCopied(true));
    },
  ];
}

/** Closes a popover when clicking outside it or pressing Escape. */
export function useDismiss(open: boolean, onClose: () => void) {
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!open) return;
    const onDown = (e: PointerEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) onClose();
    };
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("pointerdown", onDown);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("pointerdown", onDown);
      window.removeEventListener("keydown", onKey);
    };
  }, [open, onClose]);
  return ref;
}
