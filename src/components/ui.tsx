import { ChevronRight } from "lucide-react";
import {
  type ButtonHTMLAttributes,
  type CSSProperties,
  type KeyboardEvent,
  type ReactNode,
  type RefObject,
  useCallback,
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useState,
} from "react";
import { createPortal } from "react-dom";

/** Keeps an element mounted through its exit transition. Exit is faster than enter. */
export function usePresence(open: boolean, exitMs = 130) {
  const [mounted, setMounted] = useState(open);
  const [state, setState] = useState<"open" | "closed">(open ? "open" : "closed");
  useEffect(() => {
    if (open) {
      setMounted(true);
      let r2 = 0;
      const r1 = requestAnimationFrame(() => (r2 = requestAnimationFrame(() => setState("open"))));
      return () => {
        cancelAnimationFrame(r1);
        cancelAnimationFrame(r2);
      };
    }
    setState("closed");
    const timer = setTimeout(() => setMounted(false), exitMs);
    return () => clearTimeout(timer);
  }, [open, exitMs]);
  return { mounted, state };
}

/** Floating layers keep at least this far from the window edges. */
const EDGE = 8;
const clamp = (v: number, lo: number, hi: number) => Math.max(lo, Math.min(v, hi));

type Side = "top" | "bottom";

/**
 * Positions a floating layer (menu, tooltip) that is rendered into <body>, so scroll
 * containers such as the inspector or the queue can never clip it. It prefers `side`,
 * flips when the other side has more room, slides along the window edge to stay fully
 * visible, and grows from the point nearest its trigger. It follows the trigger while
 * a panel scrolls, and calls `onDetach` once the trigger has scrolled out of view.
 */
function useFloating(
  active: boolean,
  anchor: RefObject<HTMLElement | null>,
  floating: RefObject<HTMLElement | null>,
  { side, align, gap }: { side: Side; align: "start" | "center" | "end"; gap: number },
  onDetach: () => void,
) {
  const [pos, setPos] = useState<{ style: CSSProperties; side: Side }>({ style: { visibility: "hidden" }, side });
  const detach = useRef(onDetach);
  useLayoutEffect(() => {
    detach.current = onDetach;
  });
  useLayoutEffect(() => {
    if (!active) return;
    // Panels around the trigger that clip their content.
    const clips: HTMLElement[] = [];
    for (let p = anchor.current?.parentElement; p && p !== document.body; p = p.parentElement) {
      const cs = getComputedStyle(p);
      if (cs.overflowX !== "visible" || cs.overflowY !== "visible") clips.push(p);
    }
    const place = () => {
      const a = anchor.current?.getBoundingClientRect();
      const f = floating.current;
      if (!a || !f) return;
      const vw = document.documentElement.clientWidth;
      const vh = document.documentElement.clientHeight;
      let [vt, vb, vl, vr] = [Math.max(a.top, 0), Math.min(a.bottom, vh), Math.max(a.left, 0), Math.min(a.right, vw)];
      for (const c of clips) {
        const r = c.getBoundingClientRect();
        [vt, vb, vl, vr] = [Math.max(vt, r.top), Math.min(vb, r.bottom), Math.max(vl, r.left), Math.min(vr, r.right)];
      }
      if (vb <= vt || vr <= vl) {
        detach.current();
        return;
      }
      // Layout size, unaffected by the enter animation's scale.
      const w = f.offsetWidth;
      const h = f.offsetHeight;
      const other: Side = side === "top" ? "bottom" : "top";
      const room = (s: Side) => (s === "top" ? a.top : vh - a.bottom) - gap - EDGE;
      const s = room(side) >= h || room(side) >= room(other) ? side : other;
      const ax = align === "start" ? a.left : align === "end" ? a.right : a.left + a.width / 2;
      const left = clamp(align === "start" ? ax : align === "end" ? ax - w : ax - w / 2, EDGE, vw - w - EDGE);
      const top = clamp(s === "bottom" ? a.bottom + gap : a.top - gap - h, EDGE, vh - h - EDGE);
      setPos({ side: s, style: { top, left, transformOrigin: `${clamp(ax - left, 0, w)}px ${s === "bottom" ? "0" : "100%"}` } });
    };
    place();
    window.addEventListener("resize", place);
    window.addEventListener("scroll", place, true);
    return () => {
      window.removeEventListener("resize", place);
      window.removeEventListener("scroll", place, true);
    };
  }, [active, anchor, floating, side, align, gap]);
  return pos;
}

type BtnProps = ButtonHTMLAttributes<HTMLButtonElement> & {
  variant?: "primary" | "ghost" | "danger" | "default";
  size?: "sm" | "lg";
};

export function Button({ variant = "default", size, className = "", ...rest }: BtnProps) {
  const cls = ["btn", variant !== "default" && `btn--${variant}`, size && `btn--${size}`, className].filter(Boolean).join(" ");
  return <button type="button" className={cls} {...rest} />;
}

export function IconButton({ label, children, ...rest }: ButtonHTMLAttributes<HTMLButtonElement> & { label: string }) {
  return (
    <Tooltip text={label}>
      <button type="button" className="icon-btn" aria-label={label} {...rest}>
        {children}
      </button>
    </Tooltip>
  );
}

// Tooltips: the first one waits, neighbours open instantly while one was just shown.
let warmUntil = 0;

export function Tooltip({ text, children, disabled }: { text?: ReactNode; children: ReactNode; disabled?: boolean }) {
  const [open, setOpen] = useState(false);
  const [instant, setInstant] = useState(false);
  const timer = useRef<ReturnType<typeof setTimeout>>(undefined);
  const id = useId();
  const anchor = useRef<HTMLSpanElement>(null);
  const bubble = useRef<HTMLSpanElement>(null);
  const { mounted, state } = usePresence(open, instant ? 0 : 125);
  const pos = useFloating(mounted, anchor, bubble, { side: "top", align: "center", gap: 6 }, () => {
    clearTimeout(timer.current);
    setOpen(false);
  });
  if (!text || disabled) return <>{children}</>;
  const show = () => {
    const warm = Date.now() < warmUntil;
    setInstant(warm);
    clearTimeout(timer.current);
    timer.current = setTimeout(() => setOpen(true), warm ? 0 : 450);
  };
  const hide = () => {
    clearTimeout(timer.current);
    if (open) warmUntil = Date.now() + 300;
    setOpen(false);
  };
  return (
    <span ref={anchor} className="tip" onPointerEnter={show} onPointerLeave={hide} onFocus={show} onBlur={hide} aria-describedby={open ? id : undefined}>
      {children}
      {mounted &&
        createPortal(
          <span ref={bubble} className="tip__bubble" role="tooltip" id={id} data-state={state} data-side={pos.side} data-instant={instant} style={pos.style}>
            {text}
          </span>,
          document.body,
        )}
    </span>
  );
}

export interface SegOption<T extends string> {
  value: T;
  label: ReactNode;
  disabled?: boolean;
  hint?: string;
}

/** Radio group with a sliding thumb. Keyboard changes skip the animation. */
export function Segmented<T extends string>({
  value,
  options,
  onChange,
  label,
}: {
  value: T;
  options: SegOption<T>[];
  onChange: (v: T) => void;
  label: string;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const [thumb, setThumb] = useState<CSSProperties>({ width: 0 });
  const instant = useRef(false);

  // Measured against the group itself: options may sit inside positioned tooltip wrappers,
  // so offsetLeft would be relative to the wrapper, not the group.
  const place = (el: HTMLElement | null | undefined, transition?: string) => {
    const seg = ref.current;
    if (!el || !seg) return;
    const s = seg.getBoundingClientRect();
    const r = el.getBoundingClientRect();
    setThumb({ width: r.width, transform: `translateX(${r.left - s.left - seg.clientLeft}px)`, transition });
  };

  useLayoutEffect(() => {
    place(ref.current?.querySelector<HTMLButtonElement>(`[data-value="${CSS.escape(value)}"]`), instant.current ? "none" : undefined);
    instant.current = false;
  }, [value, options.length]);

  useEffect(() => {
    const ro = new ResizeObserver(() => place(ref.current?.querySelector<HTMLButtonElement>(`[aria-checked="true"]`), "none"));
    if (ref.current) ro.observe(ref.current);
    return () => ro.disconnect();
  }, []);

  const onKey = (e: KeyboardEvent) => {
    if (!["ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown"].includes(e.key)) return;
    e.preventDefault();
    const enabled = options.filter((o) => !o.disabled);
    const i = enabled.findIndex((o) => o.value === value);
    const d = e.key === "ArrowLeft" || e.key === "ArrowUp" ? -1 : 1;
    const next = enabled[(i + d + enabled.length) % enabled.length];
    if (next) {
      instant.current = true;
      onChange(next.value);
      ref.current?.querySelector<HTMLButtonElement>(`[data-value="${CSS.escape(next.value)}"]`)?.focus();
    }
  };

  return (
    <div className="seg" role="radiogroup" aria-label={label} ref={ref} onKeyDown={onKey}>
      <span className="seg__thumb" style={thumb} aria-hidden />
      {options.map((o) => (
        <Tooltip key={o.value} text={o.hint} disabled={!o.hint}>
          <button
            type="button"
            role="radio"
            className="seg__opt"
            data-value={o.value}
            aria-checked={o.value === value}
            tabIndex={o.value === value ? 0 : -1}
            disabled={o.disabled}
            onClick={() => onChange(o.value)}
            style={{ width: "100%" }}
          >
            {o.label}
          </button>
        </Tooltip>
      ))}
    </div>
  );
}

export function Field({ label, hint, children, aside }: { label: ReactNode; hint?: ReactNode; children: ReactNode; aside?: ReactNode }) {
  return (
    <div className="field">
      <div className="field__label">
        <span className="grow">{label}</span>
        {aside}
      </div>
      {children}
      {hint && <div className="field__hint">{hint}</div>}
    </div>
  );
}

export function Check({ checked, onChange, children, disabled }: { checked: boolean; onChange: (v: boolean) => void; children: ReactNode; disabled?: boolean }) {
  return (
    <label className="check">
      <input type="checkbox" checked={checked} disabled={disabled} onChange={(e) => onChange(e.target.checked)} />
      <span>{children}</span>
    </label>
  );
}

export function Slider({ value, min, max, step = 1, onChange, label }: { value: number; min: number; max: number; step?: number; onChange: (v: number) => void; label: string }) {
  const fill = ((value - min) / (max - min)) * 100;
  return (
    <input
      className="slider"
      type="range"
      aria-label={label}
      min={min}
      max={max}
      step={step}
      value={value}
      style={{ "--fill": `${fill}%` } as CSSProperties}
      onChange={(e) => onChange(Number(e.target.value))}
    />
  );
}

export function Disclosure({ title, children, defaultOpen = false }: { title: ReactNode; children: ReactNode; defaultOpen?: boolean }) {
  const [open, setOpen] = useState(defaultOpen);
  const id = useId();
  return (
    <div className="disclosure" data-open={open}>
      <button type="button" className="disclosure__btn" aria-expanded={open} aria-controls={id} onClick={() => setOpen((o) => !o)}>
        <ChevronRight size={14} className="disclosure__chev" />
        {title}
      </button>
      {open && (
        <div className="disclosure__panel" id={id}>
          {children}
        </div>
      )}
    </div>
  );
}

export interface MenuItem {
  label: ReactNode;
  onSelect?: () => void;
  checked?: boolean;
  disabled?: boolean;
  separator?: boolean;
  heading?: boolean;
}

/** Dropdown that grows from its trigger; Esc / Tab / outside click closes; arrow keys move. */
export function Menu({ trigger, items, align = "left" }: { trigger: (p: { open: boolean; toggle: () => void }) => ReactNode; items: MenuItem[]; align?: "left" | "right" }) {
  const [open, setOpen] = useState(false);
  const { mounted, state } = usePresence(open, 120);
  const anchor = useRef<HTMLDivElement>(null);
  const menu = useRef<HTMLDivElement>(null);
  const pos = useFloating(mounted, anchor, menu, { side: "bottom", align: align === "right" ? "end" : "start", gap: 4 }, () => setOpen(false));
  const toggle = useCallback(() => setOpen((o) => !o), []);

  useEffect(() => {
    if (!open) return;
    const onDown = (e: PointerEvent) => {
      const target = e.target as Node;
      if (!anchor.current?.contains(target) && !menu.current?.contains(target)) setOpen(false);
    };
    window.addEventListener("pointerdown", onDown);
    return () => window.removeEventListener("pointerdown", onDown);
  }, [open]);

  useEffect(() => {
    if (state === "open") menu.current?.querySelector<HTMLButtonElement>(".menu__item:not(:disabled)")?.focus();
  }, [state]);

  // The menu sits in a portal; React still bubbles its key events up to this anchor.
  const onKey = (e: KeyboardEvent) => {
    if (!open) return;
    if (e.key === "Escape" || e.key === "Tab") {
      // Back to the trigger; on Tab the browser then moves on from there.
      setOpen(false);
      anchor.current?.querySelector<HTMLElement>("button")?.focus();
      return;
    }
    if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
    e.preventDefault();
    const btns = Array.from(menu.current?.querySelectorAll<HTMLButtonElement>(".menu__item:not(:disabled)") ?? []);
    const i = btns.indexOf(document.activeElement as HTMLButtonElement);
    btns[(i + (e.key === "ArrowDown" ? 1 : -1) + btns.length) % btns.length]?.focus();
  };

  return (
    <div className="pop-anchor" ref={anchor} onKeyDown={onKey}>
      {trigger({ open, toggle })}
      {mounted &&
        createPortal(
          <div ref={menu} className="menu" role="menu" data-state={state} data-side={pos.side} style={pos.style}>
            {items.map((it, i) =>
              it.separator ? (
                <div key={i} className="menu__sep" />
              ) : it.heading ? (
                <div key={i} className="menu__label">
                  {it.label}
                </div>
              ) : (
                <button
                  key={i}
                  type="button"
                  role={it.checked !== undefined ? "menuitemradio" : "menuitem"}
                  aria-checked={it.checked}
                  className="menu__item"
                  disabled={it.disabled}
                  onClick={() => {
                    setOpen(false);
                    it.onSelect?.();
                  }}
                >
                  {it.label}
                </button>
              ),
            )}
          </div>,
          document.body,
        )}
    </div>
  );
}

/** Bulk/destructive actions: press and hold instead of a confirmation dialog. */
export function HoldButton({ children, onConfirm, ms = 1200, disabled }: { children: ReactNode; onConfirm: () => void; ms?: number; disabled?: boolean }) {
  const [holding, setHolding] = useState(false);
  const timer = useRef<ReturnType<typeof setTimeout>>(undefined);
  const start = () => {
    if (disabled) return;
    setHolding(true);
    timer.current = setTimeout(() => {
      setHolding(false);
      onConfirm();
    }, ms);
  };
  const stop = () => {
    clearTimeout(timer.current);
    setHolding(false);
  };
  return (
    <button
      type="button"
      className="btn btn--sm btn--ghost hold"
      data-holding={holding}
      disabled={disabled}
      onPointerDown={start}
      onPointerUp={stop}
      onPointerLeave={stop}
      onKeyDown={(e) => {
        if ((e.key === "Enter" || e.key === " ") && !e.repeat) {
          e.preventDefault();
          start();
        }
      }}
      onKeyUp={(e) => {
        if (e.key === "Enter" || e.key === " ") stop();
      }}
    >
      <span className="hold__fill" />
      <span className="hold__label">{children}</span>
    </button>
  );
}

export function Dialog({
  open,
  onClose,
  title,
  children,
  wide,
  persistent,
}: {
  open: boolean;
  onClose: () => void;
  title: ReactNode;
  children: ReactNode;
  wide?: boolean;
  persistent?: boolean;
}) {
  const { mounted, state } = usePresence(open, 160);
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (state !== "open") return;
    const first = ref.current?.querySelector<HTMLElement>("[data-autofocus], button, input, select");
    first?.focus();
  }, [state]);
  if (!mounted) return null;
  return (
    <div
      className="backdrop"
      data-state={state}
      onPointerDown={(e) => {
        if (e.target === e.currentTarget && !persistent) onClose();
      }}
      onKeyDown={(e) => {
        if (e.key === "Escape" && !persistent) onClose();
      }}
    >
      <div className={`dialog ${wide ? "dialog--wide" : ""}`} role="dialog" aria-modal="true" aria-label={typeof title === "string" ? title : undefined} ref={ref}>
        <h2 className="dialog__title">{title}</h2>
        {children}
      </div>
    </div>
  );
}

export function Chip({ tone, children, title }: { tone?: "accent" | "ok" | "bad" | "ask" | "run"; children: ReactNode; title?: string }) {
  return (
    <span className={`chip ${tone ? `chip--${tone}` : ""}`} title={title}>
      {children}
    </span>
  );
}

export function Spinner() {
  return <div className="spinner" role="status" aria-label="loading" />;
}
