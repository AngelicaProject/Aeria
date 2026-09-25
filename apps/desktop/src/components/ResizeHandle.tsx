import { useEffect, useRef, type RefObject } from "react";

type ResizeHandleProps = {
  axis: "x" | "y";
  label: string;
  /** Region size when a drag starts, and its bounds. */
  size: number;
  min: number;
  max: number;
  /** 1 when moving the pointer right or down grows the region, -1 otherwise. */
  direction: 1 | -1;
  /** Element whose CSS `variable` sizes the region. */
  target: RefObject<HTMLElement | null>;
  variable: string;
  /** Called once per drag, on release, with the total size change. */
  onResize: (delta: number) => void;
};

type Drag = { origin: number; startSize: number; size: number };

/**
 * A drag previews the new size by writing the CSS variable directly and
 * commits it to layout state only on release, so dragging re-renders nothing.
 */
export function ResizeHandle({ axis, label, size, min, max, direction, target, variable, onResize }: ResizeHandleProps) {
  const dragRef = useRef<Drag | null>(null);
  const propsRef = useRef({ axis, min, max, direction, target, variable, onResize });
  propsRef.current = { axis, min, max, direction, target, variable, onResize };

  useEffect(() => {
    function handlePointerMove(event: PointerEvent) {
      const drag = dragRef.current;
      if (!drag) return;
      const { axis, min, max, direction, target, variable } = propsRef.current;
      const current = axis === "x" ? event.clientX : event.clientY;
      const next = Math.max(min, Math.min(max, drag.startSize + direction * (current - drag.origin)));
      if (next === drag.size) return;
      drag.size = next;
      target.current?.style.setProperty(variable, `${next}px`);
    }
    function handlePointerUp() {
      const drag = dragRef.current;
      if (!drag) return;
      dragRef.current = null;
      document.body.classList.remove("resizing-layout-x", "resizing-layout-y");
      if (drag.size !== drag.startSize) propsRef.current.onResize(drag.size - drag.startSize);
    }
    window.addEventListener("pointermove", handlePointerMove);
    window.addEventListener("pointerup", handlePointerUp);
    window.addEventListener("pointercancel", handlePointerUp);
    return () => {
      window.removeEventListener("pointermove", handlePointerMove);
      window.removeEventListener("pointerup", handlePointerUp);
      window.removeEventListener("pointercancel", handlePointerUp);
    };
  }, []);

  return (
    <div
      className={`resize-handle ${axis === "x" ? "resize-handle-x" : "resize-handle-y"}`}
      role="separator"
      aria-label={label}
      onPointerDown={(event) => {
        event.currentTarget.setPointerCapture?.(event.pointerId);
        const origin = axis === "x" ? event.clientX : event.clientY;
        dragRef.current = { origin, startSize: size, size };
        document.body.classList.add(axis === "x" ? "resizing-layout-x" : "resizing-layout-y");
      }}
    />
  );
}
