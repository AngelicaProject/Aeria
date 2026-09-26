/**
 * Images the user attaches to a message for Angelica.
 *
 * Images are scaled and encoded here so they fit what vision models accept;
 * Rust checks them again before storing (see `aeria-ai::images`).
 */

/** Most images in one message, matching `MAX_IMAGES_PER_MESSAGE` in Rust. */
export const MAX_IMAGES_PER_MESSAGE = 6;
/** Longest side Rust accepts. */
export const MAX_IMAGE_SIDE = 2048;
/** Pixels kept at most: a Full HD screenshot passes unscaled, larger ones
 * shrink before they reach the model, which would scale them anyway. */
export const MAX_IMAGE_PIXELS = 1920 * 1080;
/** Largest file Rust accepts. */
export const MAX_IMAGE_BYTES = 3_750_000;

/** An image ready to send: base64 file content and a preview URL. */
export type PreparedImage = {
  key: string;
  /** Standard base64 of a PNG or JPEG file. */
  data: string;
  /** A `data:` URL for the thumbnail. */
  preview: string;
  width: number;
  height: number;
};

/** Why an image could not be attached. */
export type ImageFailure = "unreadable" | "tooLarge";

export class ImageAttachError extends Error {
  readonly reason: ImageFailure;

  constructor(reason: ImageFailure) {
    super(reason);
    this.reason = reason;
  }
}

/** The size an image is scaled to: within both side and pixel limits, never enlarged. */
export function fitImageSize(width: number, height: number): { width: number; height: number } {
  const scale = Math.min(1, MAX_IMAGE_SIDE / Math.max(width, height), Math.sqrt(MAX_IMAGE_PIXELS / (width * height)));
  return { width: Math.max(1, Math.round(width * scale)), height: Math.max(1, Math.round(height * scale)) };
}

/** Whether a file can be sent as it is, without decoding and encoding it again. */
export function canSendAsIs(type: string, bytes: number, width: number, height: number): boolean {
  const fitted = fitImageSize(width, height);
  return (type === "image/png" || type === "image/jpeg") && bytes <= MAX_IMAGE_BYTES && fitted.width === width && fitted.height === height;
}

type ClipboardLike = {
  types: readonly string[];
  getData(format: string): string;
  files: ArrayLike<File>;
};

/**
 * Image files pasted or dropped. Content that also carries plain text, such
 * as cells copied from a spreadsheet, pastes as text, like in other editors.
 */
export function transferredImages(transfer: ClipboardLike): File[] {
  if (transfer.types.includes("text/plain") && transfer.getData("text/plain").trim()) return [];
  return Array.from(transfer.files).filter((file) => file.type.startsWith("image/"));
}

function base64(bytes: Uint8Array): string {
  let binary = "";
  for (let start = 0; start < bytes.length; start += 0x8000) {
    binary += String.fromCharCode(...bytes.subarray(start, start + 0x8000));
  }
  return btoa(binary);
}

function encode(canvas: HTMLCanvasElement, type: string, quality?: number): Promise<Blob> {
  return new Promise((resolve, reject) => canvas.toBlob((blob) => blob ? resolve(blob) : reject(new ImageAttachError("unreadable")), type, quality));
}

let nextKey = 0;

/**
 * Decodes an image of any format the webview reads, scales it to the limits,
 * and encodes it: screenshots and other PNGs stay PNG for crisp text, photos
 * become JPEG, and a PNG over the size limit falls back to JPEG.
 */
export async function prepareImage(file: Blob): Promise<PreparedImage> {
  let bitmap: ImageBitmap;
  try {
    bitmap = await createImageBitmap(file);
  } catch {
    throw new ImageAttachError("unreadable");
  }
  try {
    let blob: Blob;
    const { width, height } = fitImageSize(bitmap.width, bitmap.height);
    if (canSendAsIs(file.type, file.size, bitmap.width, bitmap.height)) {
      blob = file;
    } else {
      const canvas = document.createElement("canvas");
      canvas.width = width;
      canvas.height = height;
      const context = canvas.getContext("2d");
      if (!context) throw new ImageAttachError("unreadable");
      context.imageSmoothingQuality = "high";
      context.drawImage(bitmap, 0, 0, width, height);
      blob = file.type === "image/jpeg" ? await encode(canvas, "image/jpeg", 0.9) : await encode(canvas, "image/png");
      for (const quality of [0.9, 0.8, 0.7]) {
        if (blob.size <= MAX_IMAGE_BYTES) break;
        blob = await encode(canvas, "image/jpeg", quality);
      }
    }
    if (blob.size > MAX_IMAGE_BYTES) throw new ImageAttachError("tooLarge");
    const data = base64(new Uint8Array(await blob.arrayBuffer()));
    nextKey += 1;
    return { key: `image${nextKey}`, data, preview: `data:${blob.type};base64,${data}`, width, height };
  } finally {
    bitmap.close();
  }
}
