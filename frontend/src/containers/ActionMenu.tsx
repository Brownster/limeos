/**
 * A menu button (WAI-ARIA menu pattern). The trigger opens it with a click,
 * Enter, Space or the arrow keys; inside, the arrows, Home, End and a typed
 * first letter move between items; Escape or Tab closes it and returns focus
 * to the trigger; a click elsewhere closes it. Unavailable items stay
 * focusable and say why (aria-disabled plus a visible reason) instead of
 * vanishing. The menu is positioned against the viewport so a scrolling
 * table cannot clip it, and it closes when the page scrolls.
 */
import { useEffect, useId, useLayoutEffect, useRef, useState, type KeyboardEvent } from "react";
import { CIcon, type ContainerIconName } from "./icons";

export interface MenuItem {
  id: string;
  label: string;
  icon?: ContainerIconName;
  /** Null or undefined when available; otherwise the reason it is not. */
  disabled?: string | null;
  separatorBefore?: boolean;
  onSelect: (trigger: HTMLButtonElement) => void;
}

const MENU_WIDTH = 248;
const EDGE = 8;

export function ActionMenu({ label, items, className }: { label: string; items: MenuItem[]; className?: string }) {
  const [open, setOpen] = useState(false);
  const [position, setPosition] = useState<{ top: number; left: number } | null>(null);
  const trigger = useRef<HTMLButtonElement>(null);
  const menu = useRef<HTMLDivElement>(null);
  const focusOnOpen = useRef<"first" | "last">("first");
  const id = useId();

  const itemElements = () => Array.from(menu.current?.querySelectorAll<HTMLButtonElement>('[role="menuitem"]') ?? []);
  const close = (refocus: boolean) => {
    setOpen(false);
    setPosition(null);
    if (refocus) trigger.current?.focus();
  };
  const openMenu = (focus: "first" | "last") => {
    focusOnOpen.current = focus;
    setOpen(true);
  };

  // Place the menu below the trigger (above when there is no room), right-aligned.
  useLayoutEffect(() => {
    if (!open || !trigger.current || !menu.current) return;
    const box = trigger.current.getBoundingClientRect();
    const height = menu.current.offsetHeight;
    const width = Math.min(MENU_WIDTH, window.innerWidth - 2 * EDGE);
    const left = Math.max(EDGE, Math.min(box.right - width, window.innerWidth - width - EDGE));
    const below = box.bottom + 4;
    const top = below + height > window.innerHeight - EDGE && box.top - height - 4 > EDGE ? box.top - height - 4 : below;
    setPosition({ top, left });
  }, [open]);

  useEffect(() => {
    if (!open || !position) return;
    const elements = itemElements();
    (focusOnOpen.current === "first" ? elements[0] : elements.at(-1))?.focus();
  }, [open, position]);

  useEffect(() => {
    if (!open) return;
    const outside = (event: PointerEvent) => {
      const target = event.target as Node;
      if (!menu.current?.contains(target) && !trigger.current?.contains(target)) close(false);
    };
    const away = () => close(false);
    document.addEventListener("pointerdown", outside, true);
    window.addEventListener("scroll", away, true);
    window.addEventListener("resize", away);
    return () => {
      document.removeEventListener("pointerdown", outside, true);
      window.removeEventListener("scroll", away, true);
      window.removeEventListener("resize", away);
    };
  }, [open]);

  function onTriggerKey(event: KeyboardEvent<HTMLButtonElement>) {
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      openMenu(event.key === "ArrowDown" ? "first" : "last");
    }
  }

  function onMenuKey(event: KeyboardEvent<HTMLDivElement>) {
    const elements = itemElements();
    const index = elements.indexOf(document.activeElement as HTMLButtonElement);
    const move = (next: number) => {
      event.preventDefault();
      elements[(next + elements.length) % elements.length]?.focus();
    };
    if (event.key === "ArrowDown") move(index + 1);
    else if (event.key === "ArrowUp") move(index - 1);
    else if (event.key === "Home") move(0);
    else if (event.key === "End") move(elements.length - 1);
    else if (event.key === "Escape" || event.key === "Tab") {
      event.preventDefault();
      event.stopPropagation();
      close(true);
    } else if (event.key.length === 1 && /\S/.test(event.key)) {
      const letter = event.key.toLowerCase();
      const order = [...elements.slice(index + 1), ...elements.slice(0, index + 1)];
      const match = order.find((element) => (element.getAttribute("aria-label") ?? "").toLowerCase().startsWith(letter));
      if (match) {
        event.preventDefault();
        match.focus();
      }
    }
  }

  function select(item: MenuItem) {
    if (item.disabled || !trigger.current) return;
    const opener = trigger.current;
    close(true);
    item.onSelect(opener);
  }

  return (
    <div className={className ? `action-menu ${className}` : "action-menu"}>
      <button
        ref={trigger}
        type="button"
        className="quiet icon-button menu-trigger"
        aria-label={label}
        title={label}
        aria-haspopup="menu"
        aria-expanded={open}
        aria-controls={open ? `${id}-menu` : undefined}
        onClick={() => (open ? close(false) : openMenu("first"))}
        onKeyDown={onTriggerKey}
      >
        <CIcon name="more" size={18} />
      </button>
      {open && (
        <div
          ref={menu}
          id={`${id}-menu`}
          role="menu"
          aria-label={label}
          className="menu"
          style={position ? { top: position.top, left: position.left, width: Math.min(MENU_WIDTH, window.innerWidth - 2 * EDGE) } : { visibility: "hidden", top: 0, left: 0 }}
          onKeyDown={onMenuKey}
        >
          {items.map((item) => (
            <div key={item.id} role="none" className={item.separatorBefore ? "menu-group" : undefined}>
              <button
                type="button"
                role="menuitem"
                tabIndex={-1}
                className="menu-item"
                aria-label={item.label}
                aria-disabled={item.disabled ? true : undefined}
                aria-describedby={item.disabled ? `${id}-${item.id}-why` : undefined}
                onClick={() => select(item)}
              >
                {item.icon && <CIcon name={item.icon} />}
                <span className="menu-text">
                  <span>{item.label}</span>
                  {item.disabled && <span className="menu-reason" id={`${id}-${item.id}-why`}>{item.disabled}</span>}
                </span>
              </button>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
