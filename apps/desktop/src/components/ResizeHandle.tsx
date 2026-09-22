import { useEffect, useRef } from "react";

type ResizeHandleProps = {
  axis: "x" | "y";
  label: string;
  onDelta: (delta: number) => void;
};

export function ResizeHandle({ axis, label, onDelta }: ResizeHandleProps) {
  const startRef = useRef<number | null>(null);

  useEffect(() => {
    function handlePointerMove(event: PointerEvent) {
      if (startRef.current === null) return;
      const current = axis === "x" ? event.clientX : event.clientY;
      const delta = current - startRef.current;
      startRef.current = current;
      onDelta(delta);
    }
    function handlePointerUp() {
      startRef.current = null;
      document.body.classList.remove("resizing-layout-x", "resizing-layout-y");
    }
    window.addEventListener("pointermove", handlePointerMove);
    window.addEventListener("pointerup", handlePointerUp);
    return () => {
      window.removeEventListener("pointermove", handlePointerMove);
      window.removeEventListener("pointerup", handlePointerUp);
    };
  }, [axis, onDelta]);

  return <div className={`resize-handle ${axis === "x" ? "resize-handle-x" : "resize-handle-y"}`} role="separator" aria-label={label} onPointerDown={(event) => { event.currentTarget.setPointerCapture?.(event.pointerId); startRef.current = axis === "x" ? event.clientX : event.clientY; document.body.classList.add(axis === "x" ? "resizing-layout-x" : "resizing-layout-y"); }} />;
}
