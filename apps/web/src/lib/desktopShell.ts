export function isDesktopShell(): boolean {
  return /(?:^|\s)LankideDesktop\/\S+/.test(navigator.userAgent);
}
