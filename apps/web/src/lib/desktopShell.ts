declare global {
  interface Window {
    __MEERKAT_DESKTOP__?: {
      version: string;
      os: string;
    };
  }
}

export function isDesktopShell(): boolean {
  return (
    Boolean(window.__MEERKAT_DESKTOP__) || /(?:^|\s)MeerkatDesktop\/\S+/.test(navigator.userAgent)
  );
}

export function isMacDesktopShell(): boolean {
  if (window.__MEERKAT_DESKTOP__) {
    return window.__MEERKAT_DESKTOP__.os === 'macos';
  }
  return isDesktopShell() && /Macintosh|Mac OS X/i.test(navigator.userAgent);
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
