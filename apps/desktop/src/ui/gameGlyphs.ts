import { gameGlyphFont, gameIcon } from "../ipc";

/**
 * Game symbols in the interface: the game font's private use glyphs as a
 * web font, and the inline icons of `<icon>` macros as images. Both come
 * from the open project's game; without a project they are absent and the
 * text falls back to the interface fonts.
 */

/**
 * The family the theme's font stacks name first. It covers only the private
 * use area, so every other character comes from the interface fonts.
 */
export const GAME_GLYPH_FAMILY = "Aeria Game Glyphs";
const PRIVATE_USE = "U+E000-F8FF";

let loadedFace: FontFace | null = null;
/** Bumps with every load, so a slower earlier load does not win. */
let generation = 0;
/** Icon images by id for the current game: a data URL, or null when the game has none. */
let icons = new Map<number, Promise<string | null>>();

/**
 * Loads the game glyph font of the game at `gamePath`, replacing the one of
 * a previous project, and forgets the previous game's icons.
 */
export async function loadGameGlyphs(gamePath: string | null): Promise<void> {
  const current = ++generation;
  icons = new Map();
  if (loadedFace) {
    document.fonts.delete(loadedFace);
    loadedFace = null;
  }
  if (gamePath === null) return;
  try {
    const bytes = await gameGlyphFont();
    if (bytes.byteLength === 0) return;
    const face = new FontFace(GAME_GLYPH_FAMILY, bytes, { unicodeRange: PRIVATE_USE });
    await face.load();
    if (current !== generation) return;
    document.fonts.add(face);
    loadedFace = face;
  } catch {
    // Without the font the symbols show as the interface fonts draw them.
  }
}

function iconUrl(bytes: ArrayBuffer): string | null {
  if (bytes.byteLength < 4) return null;
  const view = new DataView(bytes);
  const width = view.getUint16(0, true);
  const height = view.getUint16(2, true);
  if (width === 0 || height === 0 || bytes.byteLength !== 4 + width * height * 4) return null;
  const canvas = document.createElement("canvas");
  canvas.width = width;
  canvas.height = height;
  const context = canvas.getContext("2d");
  if (!context) return null;
  context.putImageData(new ImageData(new Uint8ClampedArray(bytes, 4), width, height), 0, 0);
  return canvas.toDataURL("image/png");
}

/** The image of an inline game icon as a data URL, or null when the game has none. */
export function loadIcon(id: number): Promise<string | null> {
  let icon = icons.get(id);
  if (!icon) {
    icon = gameIcon(id).then(iconUrl, () => null);
    icons.set(id, icon);
  }
  return icon;
}
