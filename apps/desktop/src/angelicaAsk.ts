/**
 * Messages for Angelica from outside her panel, such as the localization
 * view's "Calibrate" and "Discuss": queued here until her panel takes them,
 * so a message sent while the panel is closed is not lost.
 */

const EVENT = "aeria:angelica-ask";
const OPEN_LOCALIZATION = "aeria:open-localization";
const pending: string[] = [];

/** Queues a message for Angelica and tells the workbench to show her. */
export function askAngelica(text: string): void {
  pending.push(text);
  window.dispatchEvent(new CustomEvent(EVENT));
}

/** Takes the queued messages, oldest first. */
export function takeAngelicaAsks(): string[] {
  return pending.splice(0, pending.length);
}

/** Calls `listener` whenever a message is queued. */
export function onAngelicaAsk(listener: () => void): () => void {
  window.addEventListener(EVENT, listener);
  return () => window.removeEventListener(EVENT, listener);
}

/** Asks the workbench to open the localization view. */
export function openLocalization(): void {
  window.dispatchEvent(new CustomEvent(OPEN_LOCALIZATION));
}

/** Calls `listener` whenever the localization view is asked for. */
export function onOpenLocalization(listener: () => void): () => void {
  window.addEventListener(OPEN_LOCALIZATION, listener);
  return () => window.removeEventListener(OPEN_LOCALIZATION, listener);
}
