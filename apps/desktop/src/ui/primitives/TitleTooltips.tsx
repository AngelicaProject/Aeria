import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { placeTooltip } from "../tooltipPlacement";

/** Hover time before a tooltip appears. */
const SHOW_DELAY_MS = 500;
/** While a tooltip was just visible, the next one appears at once. */
const SKIP_DELAY_MS = 300;

type Shown = { anchor: HTMLElement; text: string };

/**
 * Shows every `title` in the application as a themed tooltip instead of the
 * native one. On hover the attribute moves to `data-tooltip`, so the browser
 * never draws its own; an element that relied on its title for an accessible
 * name gets it as `aria-label`. Keyboard focus shows the tooltip too.
 */
export function TitleTooltips() {
  const [shown, setShown] = useState<Shown | null>(null);
  const [position, setPosition] = useState<{ left: number; top: number } | null>(null);
  const tooltipRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    let timer: number | null = null;
    let current: HTMLElement | null = null;
    let lastHidden = 0;

    const adopt = (element: Element | null): HTMLElement | null => {
      const anchor = element?.closest<HTMLElement>("[title], [data-tooltip]") ?? null;
      if (!anchor) return null;
      const title = anchor.getAttribute("title");
      if (title !== null) {
        anchor.removeAttribute("title");
        anchor.dataset.tooltip = title;
        if (!anchor.hasAttribute("aria-label") && !anchor.hasAttribute("aria-labelledby") && !anchor.textContent?.trim()) {
          anchor.setAttribute("aria-label", title);
        }
      }
      return anchor.dataset.tooltip?.trim() ? anchor : null;
    };
    const clear = () => {
      if (timer !== null) window.clearTimeout(timer);
      timer = null;
    };
    const hide = () => {
      clear();
      if (current) lastHidden = Date.now();
      current = null;
      setShown(null);
    };
    const show = (anchor: HTMLElement, immediate: boolean) => {
      clear();
      current = anchor;
      const reveal = () => {
        if (current === anchor && anchor.isConnected) setShown({ anchor, text: anchor.dataset.tooltip ?? "" });
      };
      if (immediate || Date.now() - lastHidden < SKIP_DELAY_MS) reveal();
      else timer = window.setTimeout(reveal, SHOW_DELAY_MS);
    };

    const onOver = (event: PointerEvent) => {
      const anchor = adopt(event.target as Element);
      if (anchor === current) return;
      if (!anchor) { hide(); return; }
      show(anchor, false);
    };
    const onOut = (event: PointerEvent) => {
      if (current && !current.contains(event.relatedTarget as Node | null)) hide();
    };
    const onFocus = (event: FocusEvent) => {
      const target = event.target as HTMLElement;
      if (!target.matches?.(":focus-visible")) return;
      const anchor = adopt(target);
      if (anchor === target) show(anchor, false);
    };

    document.addEventListener("pointerover", onOver, true);
    document.addEventListener("pointerout", onOut, true);
    document.addEventListener("focusin", onFocus, true);
    document.addEventListener("focusout", hide, true);
    document.addEventListener("pointerdown", hide, true);
    document.addEventListener("keydown", hide, true);
    window.addEventListener("scroll", hide, true);
    window.addEventListener("blur", hide);
    return () => {
      clear();
      document.removeEventListener("pointerover", onOver, true);
      document.removeEventListener("pointerout", onOut, true);
      document.removeEventListener("focusin", onFocus, true);
      document.removeEventListener("focusout", hide, true);
      document.removeEventListener("pointerdown", hide, true);
      document.removeEventListener("keydown", hide, true);
      window.removeEventListener("scroll", hide, true);
      window.removeEventListener("blur", hide);
    };
  }, []);

  useLayoutEffect(() => {
    const tooltip = tooltipRef.current;
    if (!shown || !tooltip) { setPosition(null); return; }
    const anchor = shown.anchor.getBoundingClientRect();
    setPosition(placeTooltip(
      { left: anchor.left, top: anchor.top, width: anchor.width, height: anchor.height },
      { width: tooltip.offsetWidth, height: tooltip.offsetHeight },
      { width: window.innerWidth, height: window.innerHeight },
    ));
  }, [shown]);

  if (!shown) return null;
  return createPortal(
    <div
      ref={tooltipRef}
      className="tooltip title-tooltip"
      role="tooltip"
      style={position ? { left: position.left, top: position.top } : { left: 0, top: 0, visibility: "hidden" }}
    >
      {shown.text}
    </div>,
    document.body,
  );
}
