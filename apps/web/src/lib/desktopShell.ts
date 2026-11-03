export function isDesktopShell(): boolean {
  return /(?:^|\s)MeerkatDesktop\/\S+/.test(navigator.userAgent);
}

export function syncWindowTheme(theme: 'system' | 'light' | 'dark') {
  if (isDesktopShell()) {
    void import('@tauri-apps/api/window')
      .then(({ getCurrentWindow }) =>
        getCurrentWindow().setTheme(theme === 'system' ? null : theme),
      )
      .catch(console.error);
  }
}
