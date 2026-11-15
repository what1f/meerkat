export type DetectedTarget =
  | 'macos-arm64'
  | 'macos-x86_64'
  | 'linux-x86_64'
  | 'linux-arm64'
  | 'windows'
  | 'mobile'
  | 'unknown';

export type SelectableTarget = 'macos-arm64' | 'macos-x86_64' | 'linux-x86_64' | 'linux-arm64';

export interface PlatformDetection {
  target: DetectedTarget;
  selected: SelectableTarget;
  ambiguousMacArch: boolean;
}

interface NavigatorUAData {
  platform?: string;
  mobile?: boolean;
  getHighEntropyValues?: (hints: string[]) => Promise<{
    architecture?: string;
    bitness?: string;
    platform?: string;
  }>;
}

const VALID_TARGETS: DetectedTarget[] = [
  'macos-arm64',
  'macos-x86_64',
  'linux-x86_64',
  'linux-arm64',
  'windows',
  'mobile',
];

function detectMacGpuArch(): 'arm64' | 'x86_64' | null {
  try {
    const canvas = document.createElement('canvas');
    const gl = (canvas.getContext('webgl2') || canvas.getContext('webgl')) as WebGLRenderingContext | null;
    if (!gl) return null;
    const ext = gl.getExtension('WEBGL_debug_renderer_info');
    const raw = ext ? gl.getParameter(ext.UNMASKED_RENDERER_WEBGL) : gl.getParameter(gl.RENDERER);
    if (typeof raw !== 'string') return null;
    if (/Apple\s*(M\d|GPU)/i.test(raw)) return 'arm64';
    if (/\b(Intel|AMD|Radeon|NVIDIA)\b/i.test(raw)) return 'x86_64';
  } catch {
    return null;
  }
  return null;
}

function isSelectable(target: DetectedTarget): target is SelectableTarget {
  return (
    target === 'macos-arm64' ||
    target === 'macos-x86_64' ||
    target === 'linux-x86_64' ||
    target === 'linux-arm64'
  );
}

export async function detectPlatform(): Promise<PlatformDetection> {
  const override = new URLSearchParams(location.search).get('platform') as DetectedTarget | null;
  if (override && VALID_TARGETS.includes(override)) {
    return {
      target: override,
      selected: isSelectable(override) ? override : 'macos-arm64',
      ambiguousMacArch: false,
    };
  }

  const ua = navigator.userAgent || '';
  const platformStr = navigator.platform || '';
  const nav = navigator as Navigator & { userAgentData?: NavigatorUAData };
  let hintPlatform = nav.userAgentData?.platform || '';
  let hintArch = '';

  if (nav.userAgentData?.getHighEntropyValues) {
    try {
      const high = await nav.userAgentData.getHighEntropyValues(['architecture', 'platform', 'bitness']);
      if (high.platform) hintPlatform = high.platform;
      if (high.architecture) hintArch = high.architecture.toLowerCase();
    } catch {
      // Fall back to standard UA/platform signals.
    }
  }

  const isIPadOS = /Macintosh|MacIntel/i.test(platformStr) && (navigator.maxTouchPoints || 0) > 1;
  const isMobile =
    Boolean(nav.userAgentData?.mobile) ||
    isIPadOS ||
    /Android|iPhone|iPad|iPod|HarmonyOS|Mobile/i.test(ua) ||
    /^(Android|iOS)$/i.test(hintPlatform);

  if (isMobile) {
    return { target: 'mobile', selected: 'macos-arm64', ambiguousMacArch: false };
  }

  if (/^Windows/i.test(hintPlatform) || /Windows|Win32|Win64/i.test(`${ua} ${platformStr}`)) {
    return { target: 'windows', selected: 'macos-arm64', ambiguousMacArch: false };
  }

  const isMac = /^macOS$/i.test(hintPlatform) || /Mac/i.test(`${ua} ${platformStr}`);
  if (isMac) {
    if (hintArch.includes('arm') || hintArch.includes('aarch64') || /\b(arm64|aarch64|apple silicon)\b/i.test(ua)) {
      return { target: 'macos-arm64', selected: 'macos-arm64', ambiguousMacArch: false };
    }
    if (hintArch.includes('x86')) {
      return { target: 'macos-x86_64', selected: 'macos-x86_64', ambiguousMacArch: false };
    }
    const gpuArch = detectMacGpuArch();
    if (gpuArch === 'arm64') {
      return { target: 'macos-arm64', selected: 'macos-arm64', ambiguousMacArch: false };
    }
    if (gpuArch === 'x86_64') {
      return { target: 'macos-x86_64', selected: 'macos-x86_64', ambiguousMacArch: false };
    }
    // Safari/Firefox on Apple Silicon freeze navigator.platform to "MacIntel".
    return { target: 'macos-arm64', selected: 'macos-arm64', ambiguousMacArch: true };
  }

  const isLinux = /^Linux/i.test(hintPlatform) || /Linux|X11|CrOS/i.test(`${ua} ${platformStr}`);
  if (isLinux) {
    if (hintArch.includes('arm') || hintArch.includes('aarch64') || /\b(arm64|aarch64|armv8)\b/i.test(`${ua} ${platformStr}`)) {
      return { target: 'linux-arm64', selected: 'linux-arm64', ambiguousMacArch: false };
    }
    return { target: 'linux-x86_64', selected: 'linux-x86_64', ambiguousMacArch: false };
  }

  return { target: 'unknown', selected: 'macos-arm64', ambiguousMacArch: false };
}
