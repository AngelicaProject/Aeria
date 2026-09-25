/** Characters Windows forbids in a file name, including control characters. */
const forbiddenCharacters = /[<>:"/\\|?*\u0000-\u001f]/u;

/**
 * Returns the folder for a new project named `name` inside `parent`, joined
 * with the separator `parent` already uses, or `null` when `name` is not a
 * usable single folder name on Windows.
 */
export function projectFolder(parent: string, name: string): string | null {
  const folder = name.trim();
  if (!folder || folder === "." || folder === ".." || forbiddenCharacters.test(folder) || /[. ]$/u.test(folder)) return null;
  const base = parent.trim().replace(/[\\/]+$/u, "");
  const separator = base.includes("/") && !base.includes("\\") ? "/" : "\\";
  return `${base}${separator}${folder}`;
}
