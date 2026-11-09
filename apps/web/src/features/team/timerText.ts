import type { Timer } from '../../lib/api/generated';

type Language = 'zh' | 'en';
const day = 86_400_000;
const midnight = (at: Date) => new Date(at.getFullYear(), at.getMonth(), at.getDate()).getTime();

export function clock(at: number, language: Language) {
  return new Intl.DateTimeFormat(language === 'zh' ? 'zh-CN' : 'en-US', {
    hour: '2-digit',
    minute: '2-digit',
    hourCycle: 'h23',
  }).format(at);
}

// Today, tomorrow, a weekday within the week, else the date.
export function dayOf(at: number, language: Language, now = Date.now()) {
  const days = Math.round((midnight(new Date(at)) - midnight(new Date(now))) / day);
  const locale = language === 'zh' ? 'zh-CN' : 'en-US';
  if (days === 0) return language === 'zh' ? '今天' : 'Today';
  if (days === 1) return language === 'zh' ? '明天' : 'Tomorrow';
  if (days > 1 && days < 7) return new Intl.DateTimeFormat(locale, { weekday: 'short' }).format(at);
  return new Intl.DateTimeFormat(locale, { month: 'short', day: 'numeric' }).format(at);
}

export function fullDate(at: number, language: Language) {
  return new Intl.DateTimeFormat(language === 'zh' ? 'zh-CN' : 'en-US', {
    month: 'short',
    day: 'numeric',
    weekday: 'short',
    hour: '2-digit',
    minute: '2-digit',
    hourCycle: 'h23',
  }).format(at);
}

const weekdays = {
  zh: ['周日', '周一', '周二', '周三', '周四', '周五', '周六', '周日'],
  en: [
    'Sundays',
    'Mondays',
    'Tuesdays',
    'Wednesdays',
    'Thursdays',
    'Fridays',
    'Saturdays',
    'Sundays',
  ],
};

// How often it repeats, in words; null when it fires once or the pattern has no plain wording.
export function repeatOf(timer: Timer, language: Language): string | null {
  const zh = language === 'zh';
  const repeat = timer.repeat;
  if (repeat.kind === 'once') return null;
  if (repeat.kind === 'every') {
    const m = repeat.minutes;
    if (m % 1440 === 0)
      return m === 1440
        ? zh
          ? '每天'
          : 'Daily'
        : zh
          ? `每 ${m / 1440} 天`
          : `Every ${m / 1440} d`;
    if (m % 60 === 0)
      return m === 60 ? (zh ? '每小时' : 'Hourly') : zh ? `每 ${m / 60} 小时` : `Every ${m / 60} h`;
    return zh ? `每 ${m} 分钟` : `Every ${m} min`;
  }
  const [minute, hour, date, month, weekday, ...rest] = repeat.expression.trim().split(/\s+/);
  const fixed = (value?: string) => !!value && /^\d+$/.test(value);
  if (rest.length || !fixed(minute) || !fixed(hour) || month !== '*') return null;
  if (date === '*' && weekday === '*') return zh ? '每天' : 'Daily';
  if (date === '*' && weekday === '1-5') return zh ? '工作日' : 'Weekdays';
  if (date === '*' && fixed(weekday) && Number(weekday) <= 7)
    return (zh ? weekdays.zh : weekdays.en)[Number(weekday)]!;
  if (fixed(date) && weekday === '*') return zh ? `每月 ${date} 日` : `Monthly on ${date}`;
  return null;
}
