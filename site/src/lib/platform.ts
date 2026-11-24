export type DetectedPlatform =
  | 'macos-arm64'
  | 'macos-x86_64'
  | 'windows-x86_64'
  | 'linux-x86_64'
  | 'linux-arm64'
  | 'other';

interface NavigatorUAData {
  platform?: string;
  mobile?: boolean;
  getHighEntropyValues?: (hints: string[]) => Promise<{ architecture?: string; platform?: string }>;
}

const VALID: DetectedPlatform[] = [
  'macos-arm64',
  'macos-x86_64',
  'windows-x86_64',
  'linux-x86_64',
  'linux-arm64',
  'other',
];

function macGpuArch(): 'arm64' | 'x86_64' | null {
  try {
    const canvas = document.createElement('canvas');
    const gl = (canvas.getContext('webgl2') || canvas.getContext('webgl')) as WebGLRenderingContext | null;
    if (!gl) return null;
    const ext = gl.getExtension('WEBGL_debug_renderer_info');
    const renderer = ext ? gl.getParameter(ext.UNMASKED_RENDERER_WEBGL) : gl.getParameter(gl.RENDERER);
    if (typeof renderer !== 'string') return null;
    if (/Apple\s*(M\d|GPU)/i.test(renderer)) return 'arm64';
    if (/\b(Intel|AMD|Radeon|NVIDIA)\b/i.test(renderer)) return 'x86_64';
  } catch {
    return null;
  }
  return null;
}

export async function detectPlatform(): Promise<DetectedPlatform> {
  const override = new URLSearchParams(location.search).get('platform') as DetectedPlatform | null;
  if (override && VALID.includes(override)) return override;

  const ua = navigator.userAgent || '';
  const platformStr = navigator.platform || '';
  const nav = navigator as Navigator & { userAgentData?: NavigatorUAData };
  let hintPlatform = nav.userAgentData?.platform || '';
  let hintArch = '';

  if (nav.userAgentData?.getHighEntropyValues) {
    try {
      const high = await nav.userAgentData.getHighEntropyValues(['architecture', 'platform']);
      if (high.platform) hintPlatform = high.platform;
      if (high.architecture) hintArch = high.architecture.toLowerCase();
    } catch {
      // Ignore browser errors.
    }
  }

  const isIPadOS = /Macintosh|MacIntel/i.test(platformStr) && (navigator.maxTouchPoints || 0) > 1;
  if (nav.userAgentData?.mobile || isIPadOS || /Android|iPhone|iPad|iPod|Mobile/i.test(ua)) {
    return 'other';
  }

  if (/^Windows/i.test(hintPlatform) || /Windows|Win32|Win64/i.test(`${ua} ${platformStr}`)) {
    return 'windows-x86_64';
  }

  if (/^macOS$/i.test(hintPlatform) || /Mac/i.test(`${ua} ${platformStr}`)) {
    if (hintArch.includes('arm') || hintArch.includes('aarch64') || /\b(arm64|aarch64)\b/i.test(ua)) {
      return 'macos-arm64';
    }
    if (hintArch.includes('x86')) return 'macos-x86_64';
    return macGpuArch() === 'x86_64' ? 'macos-x86_64' : 'macos-arm64';
  }

  if (/^Linux/i.test(hintPlatform) || /Linux|X11/i.test(`${ua} ${platformStr}`)) {
    if (hintArch.includes('arm') || hintArch.includes('aarch64') || /\b(arm64|aarch64)\b/i.test(`${ua} ${platformStr}`)) {
      return 'linux-arm64';
    }
    return 'linux-x86_64';
  }

  return 'other';
}
